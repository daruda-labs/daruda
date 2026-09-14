//! `daruda --mcp`: a byte relay between an agent's stdio and the app's socket.
//!
//! It never parses an MCP message. Both sides are newline-delimited JSON with
//! no embedded newlines, so forwarding a line is the whole job — which keeps
//! the protocol in exactly one place (the app) instead of two.
//!
//! Runs before GPUI exists. Instantiating `Application` here would start a
//! Metal context and a Dock presence per agent session, so the subcommand
//! routes out in `bootstrap` alongside `--hook`.
//!
//! Unlike `--hook`, a failure is *not* silent: the agent has to see why its
//! tools vanished, so every refusal goes back as a JSON-RPC error on stdout
//! and the diagnostic goes to the log.

use std::io::{BufRead, Write};
use std::path::Path;

use daruda_core::process_env;
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;

use crate::control::mcp::protocol;
use crate::control::mcp::socket::{MAX_FRAME_BYTES, Runtime, runtime_path};

/// The flag that routes this binary into the shim instead of the GUI.
///
/// One home for both sides: `bootstrap::route_mcp_subcommand` matches on it and
/// `orchestrator::mcp_server` spawns with it. Two literals would let the app
/// hand the agent a command line this binary no longer recognises.
pub(crate) const SUBCOMMAND: &str = "--mcp";

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ShimError {
    /// No runtime file, so daruda is not running in this profile.
    NotRunning,
    /// The runtime file is there but unreadable or malformed.
    RuntimeUnreadable,
    /// `DARUDA_CONTROL_TOKEN` was not set, so there is nothing to present.
    NoToken,
    /// The socket refused the handshake.
    Unauthorized,
    /// The app restarted since the runtime file was read, so this socket
    /// belongs to a different run and its session ids mean nothing here.
    RuntimeChanged,
    /// daruda closed the control connection — it quit, or the orchestrator's
    /// session ended. Distinct from an io failure: nothing went wrong on the
    /// wire, the other end is simply gone.
    Disconnected,
    Io(String),
}

impl std::fmt::Display for ShimError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotRunning => write!(f, "daruda is not running"),
            Self::RuntimeUnreadable => write!(f, "daruda's control runtime file is unreadable"),
            Self::NoToken => write!(f, "{} is not set", process_env::CONTROL_TOKEN.name()),
            Self::Unauthorized => write!(f, "daruda refused the control token"),
            Self::RuntimeChanged => write!(f, "daruda restarted; reconnect"),
            Self::Disconnected => write!(f, "daruda closed the control connection"),
            Self::Io(e) => write!(f, "control socket io: {e}"),
        }
    }
}

/// Where the app's data directory is for this invocation.
///
/// `DARUDA_DATA_DIR` first so a test or a second profile can point the shim at
/// its own app; otherwise the same resolver every other daruda path uses, so
/// a debug build never talks to a release install's socket.
fn data_dir() -> std::path::PathBuf {
    daruda_store::persistence::default_data_dir()
}

/// Read the published endpoint. `NotRunning` when there is no file at all —
/// the common case, and worth telling apart from a corrupt one.
pub(crate) fn load_endpoint(dir: &Path) -> Result<Runtime, ShimError> {
    let path = runtime_path(dir);
    if !path.exists() {
        return Err(ShimError::NotRunning);
    }
    let text = std::fs::read_to_string(&path).map_err(|_| ShimError::RuntimeUnreadable)?;
    serde_json::from_str(&text).map_err(|_| ShimError::RuntimeUnreadable)
}

/// The single handshake frame, newline-terminated like everything else.
pub(crate) fn handshake_frame(token: &str) -> String {
    format!("{}\n", serde_json::json!({ "token": token }))
}

/// Check the handshake reply against the runtime the file named.
///
/// A mismatch means the app restarted between the file read and the connect.
/// Reported rather than accepted: the agent's session ids belong to the old
/// run, so silently adopting the new socket would answer about a different
/// app.
pub(crate) fn check_runtime_id(reply: &str, expected: &str) -> Result<(), ShimError> {
    let value: serde_json::Value =
        serde_json::from_str(reply.trim()).map_err(|_| ShimError::Unauthorized)?;
    if value.get("ok").and_then(serde_json::Value::as_bool) != Some(true) {
        return Err(ShimError::Unauthorized);
    }
    match value.get("runtime_id").and_then(|v| v.as_str()) {
        Some(id) if id == expected => Ok(()),
        _ => Err(ShimError::RuntimeChanged),
    }
}

/// Copy whole lines from `from` to `to` until `from` closes.
///
/// One direction, because production runs *two* of these on separate threads:
/// the agent and the app both speak whenever they like, and a single pump
/// would block one on the other. Generic over the streams so both directions
/// and the tests share this exact code rather than three lookalikes.
///
/// A line over [`MAX_FRAME_BYTES`] is dropped rather than forwarded — the app
/// caps its side too, and forwarding would only move the refusal one hop
/// later. Flushed per line: a batched reply is a reply the peer has not seen.
pub(crate) fn pump<A: BufRead, B: Write>(from: A, mut to: B) -> std::io::Result<()> {
    for line in from.lines() {
        let line = line?;
        if line.len() > MAX_FRAME_BYTES {
            // Logged, not silent: a dropped frame can carry an `id` nobody
            // will ever answer, and a relay has no way to synthesize the
            // error — so the only trace is this line.
            LogWriter::log(
                ErrorReport::new("daruda --mcp dropped a frame over the size limit")
                    .severity(ErrorSeverity::Warning)
                    .at(file!(), line!())
                    .with_context("bytes", line.len().to_string())
                    .dedup("mcp.shim.oversized")
                    .build(),
            );
            continue;
        }
        writeln!(to, "{line}")?;
        to.flush()?;
    }
    Ok(())
}

/// Report a failure to the agent as a JSON-RPC error on stdout, and to the log
/// as a diagnostic.
fn report(error: &ShimError) -> i32 {
    LogWriter::log(
        ErrorReport::new("daruda --mcp could not reach the app")
            .severity(ErrorSeverity::Warning)
            .at(file!(), line!())
            .with_context("reason", error.to_string())
            .dedup("mcp.shim")
            .build(),
    );
    // A null id: this is not the answer to any one request, and a client that
    // has not sent one yet still has to be able to read it.
    let frame = protocol::error_frame(
        serde_json::Value::Null,
        protocol::INVALID_REQUEST,
        &error.to_string(),
    );
    let mut stdout = std::io::stdout().lock();
    let _ = writeln!(stdout, "{frame}");
    let _ = stdout.flush();
    1
}

/// The subcommand body. Returns the process exit code.
pub(crate) fn run() -> i32 {
    // The shim runs before `bootstrap::init_observability`, so it owns the
    // log writer's setup for its own process.
    LogWriter::init(daruda_store::observability::log_writer::LogPolicy::default());
    match connect_and_relay() {
        Ok(()) => 0,
        Err(e) => report(&e),
    }
}

fn connect_and_relay() -> Result<(), ShimError> {
    let token = process_env::CONTROL_TOKEN
        .read_utf8()
        .ok()
        .filter(|t| !t.trim().is_empty())
        .ok_or(ShimError::NoToken)?;
    let dir = data_dir();
    let runtime = load_endpoint(&dir)?;

    let stream = std::os::unix::net::UnixStream::connect(&runtime.socket)
        .map_err(|e| ShimError::Io(e.to_string()))?;
    let mut writer = stream
        .try_clone()
        .map_err(|e| ShimError::Io(e.to_string()))?;
    let shutdown = stream
        .try_clone()
        .map_err(|e| ShimError::Io(e.to_string()))?;
    let mut reader = std::io::BufReader::new(stream);

    writer
        .write_all(handshake_frame(&token).as_bytes())
        .map_err(|e| ShimError::Io(e.to_string()))?;
    writer.flush().map_err(|e| ShimError::Io(e.to_string()))?;

    let mut reply = String::new();
    reader
        .read_line(&mut reply)
        .map_err(|e| ShimError::Io(e.to_string()))?;
    check_runtime_id(&reply, &runtime.runtime_id)?;

    // Two directions, two threads, one pump: the agent and the app both speak
    // whenever they like, and a single-threaded loop would block one on the
    // other.
    //
    // The stdin thread is deliberately *not* joined. It is parked in a
    // blocking `read` on a pipe nothing will close, so joining would hang the
    // process forever — holding the app's only served connection and taking
    // every tool down with it until daruda restarts. Instead each side shuts
    // the socket down when it finishes, which is what lets the other notice.
    std::thread::spawn(move || {
        let outcome = pump(std::io::stdin().lock(), writer);
        if let Err(e) = &outcome {
            LogWriter::log(
                ErrorReport::new("daruda --mcp stopped relaying the agent's stdin")
                    .severity(ErrorSeverity::Warning)
                    .message(e.to_string())
                    .at(file!(), line!())
                    .dedup("mcp.shim.stdin")
                    .build(),
            );
        }
        // The agent is done talking, so the app has nothing more to answer.
        let _ = shutdown.shutdown(std::net::Shutdown::Both);
    });

    pump(reader, std::io::stdout().lock()).map_err(|e| ShimError::Io(e.to_string()))?;
    // Reaching here means the *app* closed the connection, which for an agent
    // mid-turn is a failure, not a clean finish: its tool calls will never be
    // answered, so it has to be told rather than left waiting.
    Err(ShimError::Disconnected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn pumped(input: &str) -> String {
        let mut out = Vec::new();
        pump(Cursor::new(input.as_bytes()), &mut out).expect("pumped");
        String::from_utf8(out).expect("utf8")
    }

    /// A frame crosses untouched. Both directions run this same function, so
    /// one test covers the relay in both.
    #[test]
    fn a_frame_crosses_untouched() {
        let frame = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}\n";
        assert_eq!(pumped(frame), frame);
        let reply = "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n";
        assert_eq!(pumped(reply), reply);
    }

    #[test]
    fn a_frame_over_the_limit_is_dropped_not_forwarded() {
        let huge = format!("{{\"x\":\"{}\"}}\n", "a".repeat(2 * 1024 * 1024));
        assert!(
            pumped(&huge).is_empty(),
            "an oversized frame must not reach the other side"
        );
    }

    /// One oversized frame must not cost the frames around it.
    #[test]
    fn a_dropped_frame_does_not_stop_the_ones_after_it() {
        let stream = format!(
            "{{\"a\":1}}\n{{\"x\":\"{}\"}}\n{{\"b\":2}}\n",
            "a".repeat(2 * 1024 * 1024)
        );
        assert_eq!(pumped(&stream), "{\"a\":1}\n{\"b\":2}\n");
    }

    /// A frame with no trailing newline is still a frame: the peer may close
    /// right after writing it.
    #[test]
    fn a_final_frame_without_a_newline_still_crosses() {
        assert_eq!(pumped("{\"a\":1}"), "{\"a\":1}\n");
    }

    #[test]
    fn the_handshake_frame_carries_the_token_and_is_newline_framed() {
        let sent = handshake_frame("tok-1");
        let v: serde_json::Value = serde_json::from_str(sent.trim()).expect("json");
        assert_eq!(v["token"], "tok-1");
        assert!(sent.ends_with('\n'), "framing is newline-delimited");
        assert_eq!(sent.matches('\n').count(), 1, "exactly one frame");
    }

    #[test]
    fn a_missing_runtime_file_reports_not_running() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(load_endpoint(dir.path()), Err(ShimError::NotRunning));
    }

    /// A file that is there but broken is a different problem from no file:
    /// one means "start daruda", the other means "something is wrong".
    #[test]
    fn a_malformed_runtime_file_is_not_reported_as_absent() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(runtime_path(dir.path()), b"{not json").expect("write");
        assert_eq!(load_endpoint(dir.path()), Err(ShimError::RuntimeUnreadable));
    }

    #[test]
    fn a_published_runtime_file_loads() {
        let dir = tempfile::tempdir().expect("tempdir");
        let own = crate::control::mcp::socket::Ownership::acquire(dir.path()).expect("lock");
        own.publish("rt-1").expect("published");
        let loaded = load_endpoint(dir.path()).expect("loaded");
        assert_eq!(loaded.runtime_id, "rt-1");
        assert_eq!(
            loaded.socket,
            crate::control::mcp::socket::socket_path(dir.path())
        );
    }

    #[test]
    fn a_runtime_id_mismatch_is_a_restart() {
        assert_eq!(
            check_runtime_id(r#"{"ok":true,"runtime_id":"rt-2"}"#, "rt-1"),
            Err(ShimError::RuntimeChanged)
        );
    }

    #[test]
    fn a_matching_runtime_id_is_accepted() {
        assert_eq!(
            check_runtime_id(r#"{"ok":true,"runtime_id":"rt-1"}"#, "rt-1"),
            Ok(())
        );
    }

    /// A refusal must not be read as a restart: the token is what is wrong,
    /// and reconnecting would not help.
    #[test]
    fn a_refused_handshake_is_unauthorized_not_a_restart() {
        for reply in [
            r#"{"ok":false,"error":"unauthorized"}"#,
            "{not json",
            "{}",
            r#"{"ok":"yes"}"#,
        ] {
            assert_eq!(
                check_runtime_id(reply, "rt-1"),
                Err(ShimError::Unauthorized),
                "{reply}"
            );
        }
    }

    /// An `ok` reply with no id is still a restart-shaped answer: it cannot be
    /// matched against what the file said.
    #[test]
    fn an_ok_reply_without_a_runtime_id_is_a_restart() {
        assert_eq!(
            check_runtime_id(r#"{"ok":true}"#, "rt-1"),
            Err(ShimError::RuntimeChanged)
        );
    }

    #[test]
    fn every_failure_has_a_diagnostic() {
        for case in [
            ShimError::NotRunning,
            ShimError::RuntimeUnreadable,
            ShimError::NoToken,
            ShimError::Unauthorized,
            ShimError::RuntimeChanged,
            ShimError::Disconnected,
            ShimError::Io("broken pipe".into()),
        ] {
            assert!(!case.to_string().is_empty(), "{case:?}");
        }
    }
}
