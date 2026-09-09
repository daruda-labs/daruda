//! Binding the socket and serving the one connection it allows.
//!
//! One connection at a time. The orchestrator is the only client, so a second
//! connection means a stale shim or something that should not be here.

use std::path::Path;

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;

use super::SocketError;
use super::auth::{TokenGate, handshake_reply, new_runtime_id};
use super::files::{Ownership, restrict_socket};
use super::frame::{read_frame, read_frame_deadlined};

/// Frames the app may fall behind by before the read loop feels it. One
/// client, so this only bounds a pathological burst.
const INBOUND_QUEUE_MAX: usize = 64;

/// How long a fresh connection has to present its token. Long enough for a
/// process that just spawned, short enough that a silent peer cannot hold the
/// one served connection.
pub(super) const HANDSHAKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// One inbound frame, with the channel its reply goes back on.
///
/// The reply is a channel rather than a return value because a `tools/call`
/// may wait on a phone tap: the read loop hands the frame off and keeps
/// reading, and a separate writer sends each answer whenever it resolves.
/// JSON-RPC matches replies by `id`, so answering out of order is legal — and
/// it is what keeps a `ping` from queueing behind a five-minute approval.
pub(crate) struct Inbound {
    pub frame: String,
    pub reply: smol::channel::Sender<String>,
    /// Which connection this arrived on. A new connection is a new session:
    /// the socket serves them one at a time, but *sequentially*, so without
    /// this a second shim would inherit the first's completed handshake.
    pub connection: u64,
}

/// A running control socket.
///
/// Dropping it releases the lock, removes the files, and cancels the accept
/// loop — so there is no separate `stop`.
pub(crate) struct Server {
    _ownership: Ownership,
    _accept: gpui::Task<()>,
    pub runtime_id: String,
    /// The session token this socket currently accepts. Rotated by whoever
    /// starts an orchestrator session — see [`TokenGate`].
    pub gate: TokenGate,
}

impl Server {
    /// Bind the socket and start accepting.
    ///
    /// The socket starts out accepting a token nobody holds: `gate.rotate()`
    /// is what mints one, and it reaches the shim through the agent session's
    /// environment, never through a file.
    ///
    /// Frames arrive on the returned receiver, which the caller drains on the
    /// foreground so `tools/call` can touch app state.
    pub(crate) fn start(
        dir: &Path,
        cx: &mut gpui::App,
    ) -> Result<(Self, smol::channel::Receiver<Inbound>), SocketError> {
        use gpui::AppContext as _;
        let ownership = Ownership::acquire(dir)?;
        let runtime_id = new_runtime_id();
        let listener =
            smol::net::unix::UnixListener::bind(ownership.socket()).map_err(SocketError::Io)?;
        // Before publishing, so no shim can connect during the window where
        // the socket is bound but still world-reachable.
        restrict_socket(ownership.socket())?;
        // Published only once the socket is bound and restricted: a shim that
        // reads the file must find something it can connect to.
        ownership.publish(&runtime_id)?;

        // Bounded: one client, and the read loop should feel backpressure from
        // an app that has stopped draining rather than buffering without end.
        let (inbound_tx, inbound_rx) = smol::channel::bounded(INBOUND_QUEUE_MAX);
        let accept_id = runtime_id.clone();
        let gate = TokenGate::new();
        let accept_gate = gate.clone();
        let accept = cx.background_spawn(async move {
            let mut connection = 0u64;
            loop {
                let stream = match listener.accept().await {
                    Ok((stream, _addr)) => stream,
                    Err(e) => {
                        LogWriter::log(
                            ErrorReport::new("Control socket accept failed")
                                .severity(ErrorSeverity::Warning)
                                .from_error(&e)
                                .at(file!(), line!())
                                .dedup("control.socket.accept")
                                .build(),
                        );
                        // A failed accept does not invalidate the listener;
                        // looping is what keeps a transient EMFILE from
                        // killing the surface for the rest of the session.
                        continue;
                    }
                };
                connection += 1;
                // Serially, not concurrently: there is one legitimate client,
                // so a second connection waits — and a stale shim cannot
                // interleave frames with the live one.
                serve(stream, &accept_gate, &accept_id, &inbound_tx, connection).await;
            }
        });

        Ok((
            Self {
                _ownership: ownership,
                _accept: accept,
                runtime_id,
                gate,
            },
            inbound_rx,
        ))
    }
}

/// Handle one connection: handshake, then relay frames until it closes.
///
/// Reading and writing are separate tasks. A `tools/call` can wait minutes on
/// an approval, and a read loop that awaited each reply would relay nothing in
/// the meantime — not a `ping`, not a cancellation, not the next call.
pub(super) async fn serve(
    stream: smol::net::unix::UnixStream,
    gate: &TokenGate,
    runtime_id: &str,
    inbound: &smol::channel::Sender<Inbound>,
    connection: u64,
) {
    use futures::AsyncWriteExt as _;

    let mut writer = stream.clone();
    let mut reader = futures::io::BufReader::new(stream.clone());

    // The handshake is the first frame, always. Reading it here rather than
    // treating it as one more relayed frame keeps the app side from having to
    // know about authentication at all. Deadlined, because a peer that
    // connects and says nothing would otherwise hold the only served
    // connection. The no-idle-timeout rule covers an established session,
    // not one that never handshakes.
    let first = match read_frame_deadlined(&mut reader, HANDSHAKE_TIMEOUT).await {
        Ok(Some(frame)) => frame,
        Ok(None) | Err(_) => return,
    };
    let (token, epoch) = gate.current();
    let (authorized, reply) = handshake_reply(&first, &token, runtime_id);
    if writer
        .write_all(format!("{reply}\n").as_bytes())
        .await
        .is_err()
        || !authorized
    {
        return;
    }

    // Replies fan in here from however many calls are outstanding, and one
    // task owns the write half so the frames cannot interleave mid-line.
    let (out_tx, out_rx) = smol::channel::unbounded::<String>();
    let writes = smol::spawn(async move {
        while let Ok(reply) = out_rx.recv().await {
            if writer
                .write_all(format!("{reply}\n").as_bytes())
                .await
                .is_err()
            {
                return;
            }
        }
    });

    loop {
        let frame = match read_frame(&mut reader).await {
            Ok(Some(frame)) => frame,
            Ok(None) => break,
            // An oversized or unreadable frame ends the connection: it can
            // carry an `id` we could never answer, so leaving the peer to wait
            // forever is worse than making it reconnect.
            Err(e) => {
                LogWriter::log(
                    ErrorReport::new("Control connection dropped a malformed frame")
                        .severity(ErrorSeverity::Warning)
                        .from_error(&e)
                        .at(file!(), line!())
                        .dedup("control.socket.frame")
                        .build(),
                );
                break;
            }
        };
        if frame.trim().is_empty() {
            continue;
        }
        // The session this connection authorized under is gone, so its token
        // is no longer the one the socket accepts. A rotation cannot close an
        // established connection, so the check lands here — on the first thing
        // it tries to do afterwards.
        if gate.epoch() != epoch {
            LogWriter::log(
                ErrorReport::new("Control connection outlived its session token")
                    .severity(ErrorSeverity::Warning)
                    .with_context("connection", connection.to_string())
                    .at(file!(), line!())
                    .dedup("control.socket.retired")
                    .build(),
            );
            break;
        }
        let (reply_tx, reply_rx) = smol::channel::bounded(1);
        if inbound
            .send(Inbound {
                frame,
                reply: reply_tx,
                connection,
            })
            .await
            .is_err()
        {
            // The app stopped draining, so it is going away.
            break;
        }
        // Whenever this one resolves. A notification produces no reply: the
        // sender is dropped without a send, which arrives as a closed channel
        // and this task simply ends.
        let out = out_tx.clone();
        smol::spawn(async move {
            if let Ok(reply) = reply_rx.recv().await {
                let _ = out.send(reply).await;
            }
        })
        .detach();
    }
    drop(out_tx);
    writes.await;
}
