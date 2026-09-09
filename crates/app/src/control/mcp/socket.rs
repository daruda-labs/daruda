//! The socket the `daruda --mcp` shim connects back through.
//!
//! `smol::net::UnixListener` rather than a blocking std listener plus a
//! thread: `smol` is already a direct dependency and re-exports `async-net`,
//! and flow execution already bridges background work into GPUI with
//! `smol::channel` + `cx.spawn` — this reuses that shape rather than inventing
//! a second one.
//!
//! One connection at a time. The orchestrator is the only client, so a second
//! connection means a stale shim or something that should not be here.
//!
//! Ownership is decided by an advisory file lock, never by probing the socket:
//! the OS releases a lock when the process dies, so a crash is handled by the
//! same code path as a clean exit.

use std::path::{Path, PathBuf};

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;

/// Bytes a single frame may carry. Generous for a tool call, far below what a
/// runaway peer could use to exhaust memory.
pub(crate) const MAX_FRAME_BYTES: usize = 1024 * 1024;

/// Frames the app may fall behind by before the read loop feels it. One
/// client, so this only bounds a pathological burst.
const INBOUND_QUEUE_MAX: usize = 64;

/// How long a fresh connection has to present its token. Long enough for a
/// process that just spawned, short enough that a silent peer cannot hold the
/// one served connection.
const HANDSHAKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Mode for the socket and its directory: owner only.
///
/// A unix socket's file mode *is* its access control — Darwin enforces it on
/// `connect` — and the default umask leaves it world-connectable, which would
/// put all nine app-driving tools behind nothing but the token.
const OWNER_ONLY_FILE: u32 = 0o600;
const OWNER_ONLY_DIR: u32 = 0o700;

/// macOS `sun_path` is 104 bytes (`sys/un.h`); Linux allows 108. Truncation
/// would bind a different path than the one written to the runtime file, so it
/// fails loudly instead.
#[cfg(target_os = "macos")]
const SUN_PATH_MAX: usize = 104;
#[cfg(not(target_os = "macos"))]
const SUN_PATH_MAX: usize = 108;

/// File names inside the profile's data directory.
const LOCK_FILE: &str = "control.lock";
const SOCKET_FILE: &str = "control.sock";
const RUNTIME_FILE: &str = "control.json";

#[derive(Debug)]
pub(crate) enum SocketError {
    /// Another process in this profile already owns the socket.
    AlreadyRunning,
    PathTooLong {
        len: usize,
        max: usize,
    },
    Io(std::io::Error),
}

impl std::fmt::Display for SocketError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyRunning => write!(f, "another daruda already owns this profile's socket"),
            Self::PathTooLong { len, max } => {
                write!(f, "socket path is {len} bytes, over the {max}-byte limit")
            }
            Self::Io(e) => write!(f, "socket io: {e}"),
        }
    }
}

/// What the shim reads to find the socket. Deliberately carries **no token**:
/// the token travels to the agent through its session environment, so a file
/// any local process can read never holds the secret.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Runtime {
    pub socket: PathBuf,
    /// Identifies this run of the app. The shim compares it against what the
    /// handshake answers, so a socket rebound by a *restarted* daruda is
    /// detected instead of silently adopted.
    pub runtime_id: String,
}

pub(crate) fn lock_path(dir: &Path) -> PathBuf {
    dir.join(LOCK_FILE)
}

pub(crate) fn socket_path(dir: &Path) -> PathBuf {
    dir.join(SOCKET_FILE)
}

pub(crate) fn runtime_path(dir: &Path) -> PathBuf {
    dir.join(RUNTIME_FILE)
}

/// Exclusive claim on this profile's control socket.
pub(crate) struct Ownership {
    /// Held for the lifetime of the claim; the OS drops the lock with it.
    _file: std::fs::File,
    socket: PathBuf,
    runtime: PathBuf,
}

impl Ownership {
    /// Take the lock and clear anything a previous run left behind.
    pub(crate) fn acquire(dir: &Path) -> Result<Self, SocketError> {
        use fs4::fs_std::FileExt;
        create_owner_only_dir(dir)?;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(lock_path(dir))
            .map_err(SocketError::Io)?;
        // `try_lock_exclusive` answers with a *bool*, not by erroring: `false`
        // means someone else holds it. Treating the `Ok(false)` as success
        // would let a second instance unlink the live socket below.
        match FileExt::try_lock_exclusive(&file) {
            Ok(true) => {}
            Ok(false) => return Err(SocketError::AlreadyRunning),
            Err(e) => return Err(SocketError::Io(e)),
        }

        let socket = socket_path(dir);
        validate_socket_path(&socket)?;
        // Holding the lock proves no live process owns this inode, so a
        // leftover file is a dead one from a crash.
        if socket.exists() {
            std::fs::remove_file(&socket).map_err(SocketError::Io)?;
        }
        Ok(Self {
            _file: file,
            socket,
            runtime: runtime_path(dir),
        })
    }

    pub(crate) fn socket(&self) -> &Path {
        &self.socket
    }

    /// Publish where the socket is, atomically.
    ///
    /// Written through a temporary file and renamed so a shim that reads while
    /// this runs sees either the old contents or the new ones, never half a
    /// file.
    pub(crate) fn publish(&self, runtime_id: &str) -> Result<(), SocketError> {
        let runtime = Runtime {
            socket: self.socket.clone(),
            runtime_id: runtime_id.to_owned(),
        };
        let dir = self.runtime.parent().unwrap_or(&self.runtime);
        let mut temp = tempfile::NamedTempFile::new_in(dir).map_err(SocketError::Io)?;
        serde_json::to_writer(&mut temp, &runtime)
            .map_err(|e| SocketError::Io(std::io::Error::other(e)))?;
        temp.as_file().sync_all().map_err(SocketError::Io)?;
        temp.persist(&self.runtime)
            .map_err(|e| SocketError::Io(e.error))?;
        Ok(())
    }
}

impl Drop for Ownership {
    fn drop(&mut self) {
        // Best effort. The lock release is what actually matters, and the OS
        // does that; a stale socket or runtime file is cleaned up by the next
        // `acquire`, which is the path a crash takes anyway.
        for path in [&self.socket, &self.runtime] {
            if let Err(e) = std::fs::remove_file(path)
                && e.kind() != std::io::ErrorKind::NotFound
            {
                LogWriter::log(
                    ErrorReport::new("Control socket file could not be removed")
                        .severity(ErrorSeverity::Info)
                        .from_error(&e)
                        .at(file!(), line!())
                        .dedup("control.socket.cleanup")
                        .build(),
                );
            }
        }
    }
}

/// Create `dir` (and its parents) so only the owner may traverse it.
///
/// The mode matters because the socket inside it is reachable by anyone who
/// can traverse the path. Existing directories are left alone: this must not
/// silently re-permission a user's data directory.
fn create_owner_only_dir(dir: &Path) -> Result<(), SocketError> {
    use std::os::unix::fs::DirBuilderExt as _;

    if dir.is_dir() {
        return Ok(());
    }
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(OWNER_ONLY_DIR)
        .create(dir)
        .map_err(SocketError::Io)
}

/// Restrict a freshly bound socket to its owner.
///
/// `bind` applies the process umask, which is typically 022 — leaving the
/// socket world-connectable. Since the mode is the only access control a unix
/// socket has, this is what stands between another local user and every tool.
fn restrict_socket(path: &Path) -> Result<(), SocketError> {
    use std::os::unix::fs::PermissionsExt as _;

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(OWNER_ONLY_FILE))
        .map_err(SocketError::Io)
}

pub(crate) fn validate_socket_path(path: &Path) -> Result<(), SocketError> {
    let len = path.as_os_str().len();
    if len >= SUN_PATH_MAX {
        return Err(SocketError::PathTooLong {
            len,
            max: SUN_PATH_MAX,
        });
    }
    Ok(())
}

/// The one non-MCP frame on the wire.
///
/// Answering without a reason on failure is deliberate: a caller guessing
/// tokens learns nothing from the reply, and there is only ever one legitimate
/// client, which already knows its own token.
pub(crate) fn handshake_reply(frame: &str, expected: &str, runtime_id: &str) -> (bool, String) {
    let offered = serde_json::from_str::<serde_json::Value>(frame)
        .ok()
        .and_then(|v| v.get("token").and_then(|t| t.as_str()).map(str::to_owned));
    // Constant-time-ish: compare full byte strings rather than returning early
    // on the first differing byte. Not a real defence — a local attacker has
    // better options — but it costs nothing.
    let ok = offered.is_some_and(|t| {
        t.len() == expected.len()
            && t.bytes()
                .zip(expected.bytes())
                .fold(0u8, |acc, (a, b)| acc | (a ^ b))
                == 0
    });
    let reply = if ok {
        serde_json::json!({ "ok": true, "runtime_id": runtime_id }).to_string()
    } else {
        serde_json::json!({ "ok": false, "error": "unauthorized" }).to_string()
    };
    // The decision travels beside the reply rather than being recovered from
    // it: an authorization outcome should not be re-derived by substring.
    (ok, reply)
}

/// A fresh per-run identity and its session token.
///
/// Both random per run: the id so a restarted daruda is distinguishable from
/// the one a shim connected to, the token so a shim from a previous run cannot
/// authenticate against this one.
pub(crate) fn new_runtime_id() -> String {
    uuid::Uuid::new_v4().as_simple().to_string()
}

pub(crate) fn new_token() -> String {
    uuid::Uuid::new_v4().as_simple().to_string()
}

/// The token the socket accepts right now, and which orchestrator session it
/// belongs to.
///
/// Shared with the accept loop rather than copied into it, because the token is
/// bound to the *session*, not the process: creating an orchestrator session
/// rotates it, so a shim from a discarded session can neither authenticate
/// again nor keep issuing calls on a connection it authorized earlier.
///
/// The epoch is what makes the second half work — a rotation cannot reach into
/// an established connection, so each one remembers the epoch it authorized
/// under and stops being served once that epoch is retired.
#[derive(Clone)]
pub(crate) struct TokenGate(std::sync::Arc<std::sync::Mutex<Gate>>);

struct Gate {
    token: String,
    epoch: u64,
}

impl TokenGate {
    /// A gate holding a token no one has been given.
    ///
    /// Random rather than empty: an empty expected token would match a peer
    /// that offers an empty one, so the pre-session state would authorize
    /// anybody. [`Self::rotate`] is the only way to obtain a usable token.
    pub(crate) fn new() -> Self {
        Self(std::sync::Arc::new(std::sync::Mutex::new(Gate {
            token: new_token(),
            epoch: 0,
        })))
    }

    /// Issue a fresh token for a new session and retire the previous one.
    pub(crate) fn rotate(&self) -> String {
        let token = new_token();
        let mut gate = self.lock();
        gate.token = token.clone();
        gate.epoch += 1;
        token
    }

    fn current(&self) -> (String, u64) {
        let gate = self.lock();
        (gate.token.clone(), gate.epoch)
    }

    fn epoch(&self) -> u64 {
        self.lock().epoch
    }

    /// A poisoned mutex means a panic while rotating, which cannot leave the
    /// token half-written — the fields are replaced together under the lock.
    fn lock(&self) -> std::sync::MutexGuard<'_, Gate> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

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
async fn serve(
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

/// Read one newline-delimited frame, refusing one that would exceed
/// [`MAX_FRAME_BYTES`].
///
/// Byte-by-byte against a cap rather than `AsyncBufReadExt::lines`, which
/// accumulates without bound: a peer sending an endless line with no newline
/// would otherwise drive the allocator, *before* presenting a token.
async fn read_frame<R>(reader: &mut R) -> std::io::Result<Option<String>>
where
    R: futures::AsyncBufRead + Unpin,
{
    let mut buf = Vec::new();
    loop {
        let mut byte = [0u8; 1];
        let read = {
            use futures::AsyncReadExt as _;
            reader.read(&mut byte).await?
        };
        if read == 0 {
            return Ok(if buf.is_empty() {
                None
            } else {
                Some(decode(buf)?)
            });
        }
        if byte[0] == b'\n' {
            return Ok(Some(decode(buf)?));
        }
        if buf.len() >= MAX_FRAME_BYTES {
            return Err(std::io::Error::other("frame over the size limit"));
        }
        buf.push(byte[0]);
    }
}

fn decode(mut buf: Vec<u8>) -> std::io::Result<String> {
    // Tolerate CRLF: the frame is the line without its terminator.
    if buf.last() == Some(&b'\r') {
        buf.pop();
    }
    String::from_utf8(buf).map_err(|_| std::io::Error::other("frame is not UTF-8"))
}

/// [`read_frame`] with a deadline. `Err` on timeout, so the caller drops the
/// connection rather than waiting on a peer that may never speak.
async fn read_frame_deadlined<R>(
    reader: &mut R,
    within: std::time::Duration,
) -> std::io::Result<Option<String>>
where
    R: futures::AsyncBufRead + Unpin,
{
    use futures::FutureExt as _;

    // ALLOW: what this bounds is how long *another process* may stay silent,
    // so the clock has to be the wall one — a virtual clock cannot describe a
    // peer that is not ours to schedule. Threading a `BackgroundExecutor` in
    // would also couple `serve` to gpui, which this layer is deliberately free
    // of because gpui's test scheduler cannot drive OS socket I/O (see the
    // module docs). Tests pass a short explicit `within`.
    #[allow(clippy::disallowed_methods)]
    let deadline = smol::Timer::after(within);
    futures::select! {
        frame = read_frame(reader).fuse() => frame,
        _ = deadline.fuse() => {
            Err(std::io::Error::from(std::io::ErrorKind::TimedOut))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_holder_of_the_lock_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let first = Ownership::acquire(dir.path()).expect("first");
        assert!(matches!(
            Ownership::acquire(dir.path()),
            Err(SocketError::AlreadyRunning)
        ));
        drop(first);
        // Dropping the guard releases the lock, so a fresh run can take it.
        if let Err(e) = Ownership::acquire(dir.path()) {
            panic!("dropping the claim must release the lock, but: {e}");
        }
    }

    #[test]
    fn a_leftover_socket_is_unlinked_once_the_lock_is_held() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sock = socket_path(dir.path());
        std::fs::write(&sock, b"stale").expect("write");
        let own = Ownership::acquire(dir.path()).expect("acquired");
        // Holding the lock proves no live process owns that inode.
        assert!(!sock.exists(), "stale socket must be removed before bind");
        assert_eq!(own.socket(), sock);
    }

    /// The claim has to be released even when the process crashed rather than
    /// exiting — which for an advisory lock means "when the file handle dies".
    #[test]
    fn dropping_the_claim_clears_its_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let own = Ownership::acquire(dir.path()).expect("acquired");
        std::fs::write(own.socket(), b"bound").expect("write");
        own.publish("rt-1").expect("published");
        assert!(runtime_path(dir.path()).exists());
        drop(own);
        assert!(!socket_path(dir.path()).exists());
        assert!(!runtime_path(dir.path()).exists());
    }

    #[test]
    fn a_path_over_the_platform_limit_fails_loudly() {
        let deep = std::path::PathBuf::from("/tmp").join("x".repeat(200));
        assert!(matches!(
            validate_socket_path(&deep.join(SOCKET_FILE)),
            Err(SocketError::PathTooLong { .. })
        ));
    }

    #[test]
    fn a_path_within_the_limit_is_accepted() {
        assert!(validate_socket_path(std::path::Path::new("/tmp/d/control.sock")).is_ok());
    }

    /// The runtime file is what the shim reads, so it must round-trip — and it
    /// must not carry the token.
    #[test]
    fn the_runtime_file_round_trips_and_holds_no_secret() {
        let dir = tempfile::tempdir().expect("tempdir");
        let own = Ownership::acquire(dir.path()).expect("acquired");
        own.publish("rt-1").expect("published");
        let text = std::fs::read_to_string(runtime_path(dir.path())).expect("read");
        let back: Runtime = serde_json::from_str(&text).expect("json");
        assert_eq!(back.runtime_id, "rt-1");
        assert_eq!(back.socket, socket_path(dir.path()));
        assert!(
            !text.contains("token"),
            "the token travels in the session environment, never here: {text}"
        );
    }

    #[test]
    fn a_handshake_with_the_wrong_token_is_refused_without_a_reason() {
        let (ok, out) = handshake_reply(r#"{"token":"nope"}"#, "real", "rt-1");
        assert!(!ok, "the decision travels beside the reply");
        let v: serde_json::Value = serde_json::from_str(&out).expect("json");
        assert_eq!(v["ok"], false);
        assert_eq!(v["error"], "unauthorized");
        assert!(v.get("detail").is_none(), "no probing hints");
        assert!(v.get("runtime_id").is_none(), "and nothing to fingerprint");
    }

    #[test]
    fn a_handshake_with_the_right_token_carries_the_runtime_id() {
        let (ok, out) = handshake_reply(r#"{"token":"real"}"#, "real", "rt-1");
        assert!(ok);
        let v: serde_json::Value = serde_json::from_str(&out).expect("json");
        assert_eq!(v["ok"], true);
        assert_eq!(v["runtime_id"], "rt-1");
    }

    #[test]
    fn a_malformed_or_tokenless_handshake_is_refused() {
        for frame in [
            "{not json",
            "{}",
            "[]",
            r#"{"token":null}"#,
            r#"{"token":7}"#,
        ] {
            let (ok, out) = handshake_reply(frame, "real", "rt-1");
            assert!(!ok, "{frame}");
            let v: serde_json::Value = serde_json::from_str(&out).expect("json");
            assert_eq!(v["ok"], false, "{frame}");
        }
    }

    /// A prefix of the real token must not pass — the comparison is on the
    /// whole string, not the first differing byte.
    #[test]
    fn a_prefix_of_the_token_is_refused() {
        let (ok, out) = handshake_reply(r#"{"token":"rea"}"#, "real", "rt-1");
        assert!(!ok);
        let v: serde_json::Value = serde_json::from_str(&out).expect("json");
        assert_eq!(v["ok"], false);
    }

    /// Await `f`, or `None` once a short deadline passes. Turns a would-be
    /// hang into an assertion failure with a message.
    async fn within<T>(f: impl Future<Output = T>) -> Option<T> {
        use futures::FutureExt as _;

        // ALLOW: same reason as `read_frame_deadlined` — these tests drive a
        // real socket on a real executor, where the only clock is the wall
        // one. Two seconds is a ceiling the passing path never reaches.
        #[allow(clippy::disallowed_methods)]
        let deadline = smol::Timer::after(std::time::Duration::from_secs(2));
        futures::select! {
            value = f.fuse() => Some(value),
            _ = deadline.fuse() => None,
        }
    }

    /// A gate that accepts exactly `token`, for tests that need to know it.
    fn gate_holding(token: &str) -> TokenGate {
        let gate = TokenGate::new();
        gate.rotate();
        gate.lock().token = token.to_owned();
        gate
    }

    /// `serve` over a real connected pair, driven by a real executor.
    ///
    /// Not a `#[gpui::test]`: gpui's test scheduler is deterministic and does
    /// not drive OS I/O, so a socket round-trip inside one deadlocks. `serve`
    /// is a plain async fn precisely so this is testable without gpui — the
    /// only thing `Server::start` adds on top is binding and the task handle.
    fn drive_serve(
        token: &str,
        client: impl FnOnce(
            futures::io::Lines<futures::io::BufReader<smol::net::unix::UnixStream>>,
            smol::net::unix::UnixStream,
            smol::channel::Receiver<Inbound>,
        ) -> std::pin::Pin<Box<dyn Future<Output = ()>>>,
    ) {
        use futures::AsyncBufReadExt as _;
        let (a, b) = std::os::unix::net::UnixStream::pair().expect("pair");
        let server_side = smol::net::unix::UnixStream::try_from(a).expect("async");
        let client_side = smol::net::unix::UnixStream::try_from(b).expect("async");
        let (inbound_tx, inbound_rx) = smol::channel::unbounded();
        let gate = gate_holding(token);

        smol::block_on(async move {
            let served = smol::spawn(async move {
                serve(server_side, &gate, "rt-1", &inbound_tx, 1).await;
            });
            let reader = futures::io::BufReader::new(client_side.clone()).lines();
            client(reader, client_side, inbound_rx).await;
            drop(served);
        });
    }

    /// Connect, handshake, send a frame, get its reply — where a framing
    /// mistake shows up.
    #[test]
    fn a_client_handshakes_and_gets_its_frames_answered() {
        use futures::AsyncWriteExt as _;
        use futures::StreamExt as _;

        drive_serve("tok", |mut lines, mut writer, inbound| {
            Box::pin(async move {
                // Stand in for the app's foreground drain: echo the frame.
                let echo = smol::spawn(async move {
                    while let Ok(msg) = inbound.recv().await {
                        let _ = msg.reply.send(format!("echo:{}", msg.frame)).await;
                    }
                });

                writer
                    .write_all(b"{\"token\":\"tok\"}\n")
                    .await
                    .expect("handshake sent");
                let reply = lines.next().await.expect("reply").expect("line");
                let v: serde_json::Value = serde_json::from_str(&reply).expect("json");
                assert_eq!(v["ok"], true);
                assert_eq!(v["runtime_id"], "rt-1");

                writer.write_all(b"hello\n").await.expect("frame sent");
                assert_eq!(
                    lines.next().await.expect("reply").expect("line"),
                    "echo:hello"
                );

                // A blank line is framing noise, not a frame.
                writer
                    .write_all(b"\n{\"x\":1}\n")
                    .await
                    .expect("frame sent");
                assert_eq!(
                    lines.next().await.expect("reply").expect("line"),
                    "echo:{\"x\":1}"
                );
                drop(echo);
            })
        });
    }

    /// A wrong token gets one reply and then nothing: the connection is not a
    /// place to keep guessing, and nothing reaches the app.
    #[test]
    fn a_client_with_the_wrong_token_is_answered_once_and_dropped() {
        use futures::AsyncWriteExt as _;
        use futures::StreamExt as _;

        drive_serve("tok", |mut lines, mut writer, inbound| {
            Box::pin(async move {
                writer
                    .write_all(b"{\"token\":\"wrong\"}\n")
                    .await
                    .expect("handshake sent");
                let reply = lines.next().await.expect("reply").expect("line");
                let v: serde_json::Value = serde_json::from_str(&reply).expect("json");
                assert_eq!(v["ok"], false);

                let _ = writer.write_all(b"hello\n").await;
                assert!(
                    lines.next().await.is_none(),
                    "the connection closes rather than allowing a retry"
                );
                assert!(inbound.is_empty(), "an unauthorized frame never arrives");
            })
        });
    }

    /// The point of the deferred reply: a slow answer must not stop the next
    /// frame being read. During a five-minute approval the connection has to
    /// keep relaying `ping`s, cancellations and further calls.
    #[test]
    fn a_slow_reply_does_not_stop_the_next_frame() {
        use futures::AsyncWriteExt as _;
        use futures::StreamExt as _;

        drive_serve("tok", |mut lines, mut writer, inbound| {
            Box::pin(async move {
                writer
                    .write_all(b"{\"token\":\"tok\"}\n")
                    .await
                    .expect("handshake sent");
                let _ = lines.next().await.expect("handshake reply");

                writer.write_all(b"slow\n").await.expect("sent");
                let slow = within(inbound.recv())
                    .await
                    .expect("first frame arrived")
                    .expect("channel open");

                // Nothing has answered `slow`, and the second frame still has
                // to reach the app. Deadlined so a regression *fails* here
                // rather than hanging the suite — which is exactly what a
                // read loop that awaits each reply does.
                writer.write_all(b"quick\n").await.expect("sent");
                let quick = within(inbound.recv())
                    .await
                    .expect("a slow reply must not stop the next frame")
                    .expect("channel open");
                assert_eq!(quick.frame, "quick");

                // Answering out of order is legal — JSON-RPC matches on `id`.
                quick.reply.send("quick-reply".into()).await.expect("sent");
                assert_eq!(
                    lines.next().await.expect("reply").expect("line"),
                    "quick-reply"
                );
                slow.reply.send("slow-reply".into()).await.expect("sent");
                assert_eq!(
                    lines.next().await.expect("reply").expect("line"),
                    "slow-reply"
                );
            })
        });
    }

    /// The token belongs to a session, not the process. A new session
    /// mints its own and the previous one stops authenticating.
    #[test]
    fn a_retired_token_no_longer_authenticates() {
        let gate = TokenGate::new();
        let first = gate.rotate();
        let second = gate.rotate();
        assert_ne!(first, second);

        let offer = |token: &str| {
            handshake_reply(
                &serde_json::json!({ "token": token }).to_string(),
                &gate.current().0,
                "rt-1",
            )
            .0
        };
        assert!(offer(&second), "the live session's token is accepted");
        assert!(!offer(&first), "the discarded session's token is not");
    }

    /// Before any session exists there is no usable token — and in particular
    /// an empty one must not match, which is what a zero-value placeholder
    /// would have allowed.
    #[test]
    fn no_session_means_no_token_authenticates() {
        let gate = TokenGate::new();
        for offered in ["", "tok"] {
            let (ok, _) = handshake_reply(
                &serde_json::json!({ "token": offered }).to_string(),
                &gate.current().0,
                "rt-1",
            );
            assert!(!ok, "{offered:?} must not authenticate before a session");
        }
    }

    /// A rotation cannot reach into an established connection, so the one a
    /// discarded session left behind has to stop being served at its next
    /// frame — otherwise its shim keeps every app-driving tool.
    #[test]
    fn a_connection_outliving_its_token_stops_being_served() {
        use futures::AsyncWriteExt as _;
        use futures::StreamExt as _;

        let (a, b) = std::os::unix::net::UnixStream::pair().expect("pair");
        let server_side = smol::net::unix::UnixStream::try_from(a).expect("async");
        let client_side = smol::net::unix::UnixStream::try_from(b).expect("async");
        let (inbound_tx, inbound_rx) = smol::channel::unbounded();
        let gate = gate_holding("tok");
        let rotating = gate.clone();

        smol::block_on(async move {
            use futures::AsyncBufReadExt as _;
            let served = smol::spawn(async move {
                serve(server_side, &gate, "rt-1", &inbound_tx, 1).await;
            });
            let mut lines = futures::io::BufReader::new(client_side.clone()).lines();
            let mut writer = client_side;
            writer
                .write_all(b"{\"token\":\"tok\"}\n")
                .await
                .expect("handshake sent");
            let _ = lines.next().await.expect("handshake reply");

            // Still the live session: the frame reaches the app.
            writer.write_all(b"before\n").await.expect("sent");
            let first = within(inbound_rx.recv())
                .await
                .expect("the live session is served")
                .expect("channel open");
            assert_eq!(first.frame, "before");
            drop(first);

            rotating.rotate();
            writer.write_all(b"after\n").await.expect("sent");
            assert!(
                within(served).await.is_some(),
                "a retired connection must be dropped, not kept open"
            );
            assert!(
                inbound_rx.try_recv().is_err(),
                "and the frame it sent afterwards must not reach the app"
            );
        });
    }

    /// A peer that connects and never speaks must not hold the only served
    /// connection — the accept loop is serial.
    #[test]
    fn a_silent_peer_is_dropped_after_the_handshake_deadline() {
        let (a, b) = std::os::unix::net::UnixStream::pair().expect("pair");
        let server_side = smol::net::unix::UnixStream::try_from(a).expect("async");
        let _client_side = smol::net::unix::UnixStream::try_from(b).expect("async");
        let (inbound_tx, _inbound_rx) = smol::channel::bounded(4);

        smol::block_on(async move {
            // Well under the real deadline, so the test is fast; what it pins
            // is that `serve` returns at all without input.
            let gate = gate_holding("tok");
            let served = smol::spawn(async move {
                serve(server_side, &gate, "rt-1", &inbound_tx, 1).await;
            });
            let raced = futures::FutureExt::fuse(served);
            futures::pin_mut!(raced);
            // ALLOW: as above — the deadline under test is a wall-clock one.
            #[allow(clippy::disallowed_methods)]
            let timer = smol::Timer::after(HANDSHAKE_TIMEOUT + std::time::Duration::from_secs(2));
            let timeout = futures::FutureExt::fuse(timer);
            futures::pin_mut!(timeout);
            futures::select! {
                () = raced => {}
                _ = timeout => panic!("a silent peer held the connection"),
            }
        });
    }

    /// A newline-free stream must not be buffered without bound: the cap has
    /// to stop the read, not merely reject the line afterwards. This is a
    /// *pre-authentication* read, so it is reachable by anyone who can
    /// connect.
    #[test]
    fn an_endless_line_is_refused_rather_than_buffered() {
        smol::block_on(async {
            let endless = std::io::repeat(b'a');
            let mut reader = futures::io::BufReader::new(futures::io::AllowStdIo::new(endless));
            let outcome = read_frame(&mut reader).await;
            assert!(outcome.is_err(), "the cap must end the read");
        });
    }

    /// The socket's file mode *is* its access control — Darwin enforces it on
    /// `connect` — and the default umask would leave it world-connectable.
    #[gpui::test]
    async fn the_socket_is_reachable_only_by_its_owner(cx: &mut gpui::TestAppContext) {
        use std::os::unix::fs::PermissionsExt as _;

        let parent = tempfile::tempdir().expect("tempdir");
        let dir = parent.path().join("state");
        let (_server, _inbound) = cx.update(|cx| Server::start(&dir, cx)).expect("served");

        let socket = std::fs::metadata(socket_path(&dir)).expect("socket");
        assert_eq!(
            socket.permissions().mode() & 0o777,
            OWNER_ONLY_FILE,
            "another local user must not be able to connect"
        );
        let created = std::fs::metadata(&dir).expect("dir");
        assert_eq!(created.permissions().mode() & 0o777, OWNER_ONLY_DIR);
    }

    /// A frame past the cap ends the connection rather than being silently
    /// dropped: it can carry an `id` nobody could ever answer.
    #[test]
    fn an_oversized_frame_ends_the_connection() {
        use futures::AsyncWriteExt as _;
        use futures::StreamExt as _;

        drive_serve("tok", |mut lines, mut writer, inbound| {
            Box::pin(async move {
                writer
                    .write_all(b"{\"token\":\"tok\"}\n")
                    .await
                    .expect("handshake sent");
                let _ = lines.next().await.expect("handshake reply");

                let huge = format!("{}\n", "a".repeat(MAX_FRAME_BYTES + 1));
                writer.write_all(huge.as_bytes()).await.expect("sent");
                let _ = writer.write_all(b"after\n").await;
                assert!(
                    lines.next().await.is_none(),
                    "the connection closes rather than leaving an id unanswered"
                );
                assert!(
                    inbound.is_empty(),
                    "and the oversized frame never reached the app"
                );
            })
        });
    }

    #[test]
    fn each_run_gets_its_own_identity_and_token() {
        assert_ne!(new_runtime_id(), new_runtime_id());
        assert_ne!(new_token(), new_token());
        assert_eq!(new_token().len(), 32, "uuid simple form");
    }
}
