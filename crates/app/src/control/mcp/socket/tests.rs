use super::auth::handshake_reply;
use super::files::{OWNER_ONLY_DIR, OWNER_ONLY_FILE, SOCKET_FILE, validate_socket_path};
use super::frame::read_frame;
use super::server::{HANDSHAKE_TIMEOUT, serve};
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
    let gate = TokenGate::holding(token);

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
    let gate = TokenGate::holding("tok");
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
        let gate = TokenGate::holding("tok");
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

/// A restarted daruda has to be distinguishable from the one a shim
/// connected to, which it only is if the id is fresh per run.
#[test]
fn each_run_gets_its_own_identity() {
    assert_ne!(new_runtime_id(), new_runtime_id());
}
