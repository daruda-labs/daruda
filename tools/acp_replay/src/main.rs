//! Serve a recorded ACP wire log back to a client.
//!
//! Spawned as an agent command, this stands in for the real adapter: the
//! client's JSON-RPC goes to its stdin and the recorded answers come back on
//! its stdout. Replaying at the transport rather than behind a `NodeRunner`
//! is the whole point — everything between the wire and a node's verdict (the
//! session state machine, the update mapping, the adapter's vendor reads) runs
//! for real, and those are the layers a fake runner skips.
//!
//! ## Recording one
//!
//! ```text
//! DARUDA_ACP_WIRE_LOG=/tmp/acp.log \
//! DARUDA_ACP_WIRE_LOG_MAX_FIELD=0 \
//!   target/debug/daruda
//! ```
//!
//! `MAX_FIELD=0` is not optional. The tap's default lifts any field over 512
//! bytes into a sidecar and leaves a marker behind, which reads well and
//! replays as nonsense; `0` turns that off so the log is the traffic.
//!
//! ## Replaying
//!
//! ```text
//! acp_replay /tmp/acp.log
//! ```
//!
//! Ids are the one thing not replayed verbatim. A recorded response carries
//! the id of the request *that run* made, and the client makes fresh ones — so
//! a response is emitted under the id of the live request it answers. An
//! agent-to-client request (a permission ask) keeps its recorded id, because
//! the client is about to echo it back.

use std::io::{BufRead, Write};

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: acp_replay <wire-log>");
        std::process::exit(2);
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) => {
            eprintln!("acp_replay: cannot read {path}: {e}");
            std::process::exit(2);
        }
    };
    let script = Script::read(&text);
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    serve(script, stdin.lock().lines().map_while(Result::ok), &mut out);
}

/// One line the agent sent, in the order it sent it.
#[derive(Debug, Clone, PartialEq)]
enum Sent {
    /// No id: a `session/update` and its kin. Nobody is waiting on it.
    Notification(String),
    /// Answers a client request. Emitted under the live id, so the method the
    /// recorded id belonged to is what decides which live request it answers.
    Response { method: String, line: String },
    /// The agent asking the client something (a permission request). Kept
    /// verbatim, id included — the client's reply quotes it back.
    Request(String),
}

/// What the agent said, ready to be said again.
#[derive(Debug, Default)]
struct Script {
    sent: Vec<Sent>,
}

impl Script {
    fn read(text: &str) -> Self {
        // A response names no method, so the method it answered is only
        // knowable from the request it quoted — hence one pass to learn the
        // client's ids and a second to classify what came back.
        let mut methods: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        for (dir, payload) in text.lines().filter_map(split_line) {
            if dir != Dir::In {
                continue;
            }
            if let (Some(id), Some(method)) = (id_of(payload), method_of(payload)) {
                methods.insert(id, method);
            }
        }
        let sent = text
            .lines()
            .filter_map(split_line)
            .filter(|(dir, _)| *dir == Dir::Out)
            .filter_map(|(_, payload)| {
                let line = payload.trim();
                if line.is_empty() || serde_json::from_str::<serde_json::Value>(line).is_err() {
                    return None;
                }
                Some(match (id_of(line), method_of(line)) {
                    (Some(_), Some(_)) => Sent::Request(line.to_string()),
                    (Some(id), None) => Sent::Response {
                        method: methods.get(&id).cloned().unwrap_or_default(),
                        line: line.to_string(),
                    },
                    _ => Sent::Notification(line.to_string()),
                })
            })
            .collect();
        Self { sent }
    }
}

/// Walk the recording, one client message at a time.
///
/// Single-threaded, and the protocol is what allows it: a client does not send
/// its next request before the last one is answered, so everything the agent
/// said in between can be flushed when that next message arrives. An answer the
/// *client* owes counts as a message too — the recorded turn did not finish
/// until it came.
///
/// What is left when stdin closes stays unsaid: it belongs to a turn this
/// client never asked for.
fn serve(script: Script, lines: impl Iterator<Item = String>, out: &mut impl Write) {
    let mut cursor = 0;
    // The request still owed an answer, so a client response can pick the walk
    // back up where an agent-side ask interrupted it.
    let mut awaiting: Option<(String, Option<String>)> = None;
    for line in lines {
        let asked = match method_of(&line) {
            Some(method) => Some((method, id_of(&line))),
            // The client answering an agent request: carry on with whatever was
            // still owed.
            None => awaiting.clone(),
        };
        let Some((method, live_id)) = asked else {
            continue;
        };
        let (next, answered) =
            flush_until_answer(&script, cursor, &method, live_id.as_deref(), out);
        cursor = next;
        awaiting = if answered {
            None
        } else {
            Some((method, live_id))
        };
    }
}

/// Emit everything up to and including the answer to `method`, and report where
/// that left the cursor. A request with no recorded answer flushes what came
/// before it and stops — the client then waits, which is what a hung adapter
/// does, and a test asserting a timeout wants exactly that.
fn flush_until_answer(
    script: &Script,
    from: usize,
    method: &str,
    live_id: Option<&str>,
    out: &mut impl Write,
) -> (usize, bool) {
    let mut cursor = from;
    while let Some(sent) = script.sent.get(cursor) {
        cursor += 1;
        match sent {
            Sent::Response { method: m, .. } if m == method => {
                write_sent(sent, live_id, out);
                return (cursor, true);
            }
            // An answer to something else: the recording and the live client
            // disagree about order, so leave it for whoever asks.
            Sent::Response { .. } => return (cursor - 1, false),
            // The agent asking the client something. Emitted, and then this
            // stops: the recorded turn did not finish until the client
            // answered, and running on to the answer would hand the client a
            // stop reason for a question it has not been asked yet.
            Sent::Request(_) => {
                write_sent(sent, None, out);
                return (cursor, false);
            }
            other => write_sent(other, None, out),
        }
    }
    (cursor, false)
}

fn write_sent(sent: &Sent, live_id: Option<&str>, out: &mut impl Write) {
    let line = match sent {
        Sent::Notification(line) | Sent::Request(line) => line.clone(),
        Sent::Response { line, .. } => match live_id {
            Some(id) => with_id(line, id),
            None => line.clone(),
        },
    };
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

/// Re-address a recorded answer to the request actually asking.
///
/// Through `Value`, so an id that is a number in the recording and a string
/// live re-addresses cleanly. Field order is not preserved by this and does
/// not need to be: a JSON object is unordered, and the client on the other
/// end parses rather than matches text.
fn with_id(line: &str, live_id: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(line) {
        Ok(mut value) => {
            if let Some(map) = value.as_object_mut() {
                map.insert(
                    "id".to_string(),
                    serde_json::Value::String(live_id.to_string()),
                );
            }
            value.to_string()
        }
        Err(_) => line.to_string(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dir {
    In,
    Out,
}

/// `<millis> -> stdin  {json}` / `<millis> <- stdout {json}`. Stderr lines and
/// anything else shaped differently are not traffic and are dropped.
fn split_line(line: &str) -> Option<(Dir, &str)> {
    let rest = line.split_once(' ').map(|(_, rest)| rest)?;
    if let Some(payload) = rest.strip_prefix("-> stdin ") {
        return Some((Dir::In, payload));
    }
    rest.strip_prefix("<- stdout").map(|p| (Dir::Out, p))
}

fn id_of(line: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    match value.get("id")? {
        serde_json::Value::String(id) => Some(id.clone()),
        other => Some(other.to_string()),
    }
}

fn method_of(line: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    Some(value.get("method")?.as_str()?.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A recording as the tap writes one: the client asks, the agent narrates,
    /// then answers.
    const LOG: &str = "\
1 -> stdin  {\"jsonrpc\":\"2.0\",\"id\":\"old-1\",\"method\":\"initialize\"}
2 <- stdout {\"jsonrpc\":\"2.0\",\"id\":\"old-1\",\"result\":{\"protocolVersion\":1}}
3 -> stdin  {\"jsonrpc\":\"2.0\",\"id\":\"old-2\",\"method\":\"session/prompt\"}
4 <- stdout {\"jsonrpc\":\"2.0\",\"method\":\"session/update\",\"params\":{\"a\":1}}
5 <- stdout {\"jsonrpc\":\"2.0\",\"id\":\"old-2\",\"result\":{\"stopReason\":\"end_turn\"}}
";

    fn served(log: &str, asks: &[&str]) -> Vec<String> {
        let mut out = Vec::new();
        serve(
            Script::read(log),
            asks.iter().map(|s| (*s).to_string()),
            &mut out,
        );
        String::from_utf8(out)
            .expect("utf8")
            .lines()
            .map(str::to_string)
            .collect()
    }

    /// The one thing not replayed verbatim: a recorded answer carries the id of
    /// the run that recorded it, and the live client has made its own.
    #[test]
    fn an_answer_is_readdressed_to_the_request_that_asked() {
        let lines = served(
            LOG,
            &[r#"{"jsonrpc":"2.0","id":"live-9","method":"initialize"}"#],
        );
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].contains(r#""id":"live-9""#), "{}", lines[0]);
        assert!(!lines[0].contains("old-1"), "{}", lines[0]);
        assert!(lines[0].contains(r#""protocolVersion":1"#), "{}", lines[0]);
    }

    /// Everything the agent said while the client waited has to arrive before
    /// the answer, or a turn's updates land after its stop reason.
    #[test]
    fn what_was_said_while_waiting_arrives_before_the_answer() {
        let lines = served(
            LOG,
            &[
                r#"{"jsonrpc":"2.0","id":"a","method":"initialize"}"#,
                r#"{"jsonrpc":"2.0","id":"b","method":"session/prompt"}"#,
            ],
        );
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert!(lines[1].contains("session/update"), "{}", lines[1]);
        assert!(lines[2].contains("end_turn"), "{}", lines[2]);
        assert!(lines[2].contains(r#""id":"b""#), "{}", lines[2]);
    }

    /// A request the recording never answered leaves the client waiting, which
    /// is what a hung adapter does — a test about timeouts needs that shape.
    #[test]
    fn a_request_with_no_recorded_answer_says_nothing() {
        let lines = served(
            LOG,
            &[r#"{"jsonrpc":"2.0","id":"a","method":"session/cancel"}"#],
        );
        assert!(lines.is_empty(), "{lines:?}");
    }

    /// An agent-to-client ask keeps its recorded id: the client quotes it back,
    /// and a rewritten one would answer a question nobody asked.
    #[test]
    fn an_agent_side_request_keeps_the_id_the_client_will_quote() {
        const ASKS: &str = "\
1 -> stdin  {\"jsonrpc\":\"2.0\",\"id\":\"old-1\",\"method\":\"session/prompt\"}
2 <- stdout {\"jsonrpc\":\"2.0\",\"id\":\"ask-7\",\"method\":\"session/request_permission\"}
3 -> stdin  {\"jsonrpc\":\"2.0\",\"id\":\"ask-7\",\"result\":{}}
4 <- stdout {\"jsonrpc\":\"2.0\",\"id\":\"old-1\",\"result\":{\"stopReason\":\"end_turn\"}}
";
        let lines = served(
            ASKS,
            &[
                r#"{"jsonrpc":"2.0","id":"live","method":"session/prompt"}"#,
                r#"{"jsonrpc":"2.0","id":"ask-7","result":{}}"#,
            ],
        );
        assert!(lines[0].contains(r#""id":"ask-7""#), "{}", lines[0]);
        assert!(lines[0].contains("request_permission"), "{}", lines[0]);
        assert!(lines[1].contains(r#""id":"live""#), "{}", lines[1]);
    }

    /// The order is the point, and the previous test could not see it: a
    /// recorded turn did not end until the client answered the agent's ask, so
    /// emitting the stop reason alongside the ask hands the client a verdict
    /// for a question it has not been asked.
    #[test]
    fn a_turn_that_asked_the_client_waits_for_the_answer() {
        const ASKS: &str = "\
1 -> stdin  {\"jsonrpc\":\"2.0\",\"id\":\"old-1\",\"method\":\"session/prompt\"}
2 <- stdout {\"jsonrpc\":\"2.0\",\"id\":\"ask-7\",\"method\":\"session/request_permission\"}
3 -> stdin  {\"jsonrpc\":\"2.0\",\"id\":\"ask-7\",\"result\":{}}
4 <- stdout {\"jsonrpc\":\"2.0\",\"method\":\"session/update\",\"params\":{}}
5 <- stdout {\"jsonrpc\":\"2.0\",\"id\":\"old-1\",\"result\":{\"stopReason\":\"end_turn\"}}
";
        // The prompt alone gets the ask and nothing more.
        let asked = served(
            ASKS,
            &[r#"{"jsonrpc":"2.0","id":"live","method":"session/prompt"}"#],
        );
        assert_eq!(asked.len(), 1, "{asked:?}");
        assert!(asked[0].contains("request_permission"), "{}", asked[0]);

        // The answer is what releases the rest of the turn.
        let whole = served(
            ASKS,
            &[
                r#"{"jsonrpc":"2.0","id":"live","method":"session/prompt"}"#,
                r#"{"jsonrpc":"2.0","id":"ask-7","result":{}}"#,
            ],
        );
        assert_eq!(whole.len(), 3, "{whole:?}");
        assert!(whole[1].contains("session/update"), "{}", whole[1]);
        assert!(whole[2].contains("end_turn"), "{}", whole[2]);
        assert!(whole[2].contains(r#""id":"live""#), "{}", whole[2]);
    }

    /// Stderr and anything not shaped like traffic is not traffic.
    #[test]
    fn only_the_two_traffic_directions_are_read() {
        assert_eq!(split_line("1 !! stderr boom"), None);
        assert_eq!(split_line("nonsense"), None);
        assert_eq!(split_line("1 -> stdin  {}"), Some((Dir::In, " {}")));
    }
}
