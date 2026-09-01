//! The reader half of the wire tap — a captured log replayed into the item list
//! a live chat pane would hold.
//!
//! Two things make this more than a JSON scan. The writer elides any field over
//! its spill threshold, so the reader has to put the payloads back before the
//! mapper sees them — otherwise every long tool title, diff body and image is a
//! 96-byte preview with a marker glued on, and what gets rendered is the log
//! format rather than the conversation. And the strategy is selected the way
//! production selects it — from the program the log's own `initialize` reports
//! — so a capture exercises the same dialect a live pane would read it with.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{Marker, marker_of, payload_sidecar_path};
use crate::adapter::adapter_for;
use crate::model::ChatItem;
use crate::{SessionUpdate, mapping};

/// Placeholder for a prompt whose content blocks carry no text (an
/// attachment-only turn), so the turn still anchors a user row.
const EMPTY_PROMPT: &str = "<prompt>";

/// Why a capture could not be replayed.
#[derive(Debug)]
pub enum ReplayError {
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    /// Parsed fine but held no `session/update` — not an ACP wire log, or a
    /// capture that never got past the handshake.
    NoUpdates { path: PathBuf },
}

impl fmt::Display for ReplayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, source } => {
                write!(f, "cannot read ACP wire log {}: {source}", path.display())
            }
            Self::NoUpdates { path } => write!(
                f,
                "no session/update lines in {} — not an ACP wire log?",
                path.display()
            ),
        }
    }
}

impl std::error::Error for ReplayError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read { source, .. } => Some(source),
            Self::NoUpdates { .. } => None,
        }
    }
}

/// A replayed capture, plus what the caller needs to describe it honestly.
#[derive(Debug, Default)]
pub struct Replay {
    /// The conversation, as a live pane's `items` would hold it.
    pub items: Vec<ChatItem>,
    /// Distinct `sessionId`s in the capture. More than one means the file spans
    /// several sessions, which no single live pane ever holds at once — the
    /// caller should say so rather than present the concatenation as one.
    pub sessions: usize,
    /// Elided fields restored from the sidecar.
    pub rehydrated: usize,
    /// Elided fields left as written, because the capture ran with payloads
    /// disabled or the sidecar is missing.
    pub unresolved: usize,
}

/// Replay the capture at `path`, restoring payloads from its sidecar.
///
/// `catalog_id` only decides the dialect when the log's own `initialize` did not
/// report a program daruda recognises; pass `""` to lean entirely on the log.
pub fn replay_log(path: &Path, catalog_id: &str) -> Result<Replay, ReplayError> {
    let log = std::fs::read_to_string(path).map_err(|source| ReplayError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    // A missing sidecar is not fatal: the protocol skeleton still replays, just
    // with previews where the fat fields were.
    let payloads = load_payloads(&payload_sidecar_path(path));
    let replay = replay_capture(&log, &payloads, catalog_id);
    if replay.items.is_empty() {
        return Err(ReplayError::NoUpdates {
            path: path.to_path_buf(),
        });
    }
    Ok(replay)
}

/// Read a payload sidecar into `id -> full text`. Absent or unreadable → empty.
fn load_payloads(path: &Path) -> HashMap<u64, String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return HashMap::new();
    };
    text.lines()
        .filter_map(|line| {
            let record: Value = serde_json::from_str(line).ok()?;
            let id = record.get("id")?.as_u64()?;
            Some((id, record.get("text")?.as_str()?.to_owned()))
        })
        .collect()
}

/// The `agent_info.name` from the capture's `initialize` response, if it has
/// one. `None` for a capture whose handshake was not recorded.
fn reported_program(log: &str) -> Option<String> {
    log.lines().find_map(|line| {
        let value = parse_line(line)?;
        value
            .get("result")?
            .get("agentInfo")?
            .get("name")?
            .as_str()
            .map(str::to_owned)
    })
}

/// Each log line is `<ts> <direction> <json>`; the JSON starts at the first
/// brace. Adapter stderr and other noise simply does not parse.
fn parse_line(line: &str) -> Option<Value> {
    let brace = line.find('{')?;
    serde_json::from_str(&line[brace..]).ok()
}

/// The pure core: replay `log` with `payloads` already loaded.
fn replay_capture(log: &str, payloads: &HashMap<u64, String>, catalog_id: &str) -> Replay {
    let adapter = adapter_for(reported_program(log).as_deref(), catalog_id);
    let mut out = Replay::default();
    let mut sessions: Vec<String> = Vec::new();

    for line in log.lines() {
        let Some(mut value) = parse_line(line) else {
            continue;
        };
        rehydrate(&mut value, payloads, &mut out);

        if let Some(id) = value.pointer("/params/sessionId").and_then(Value::as_str)
            && !sessions.iter().any(|seen| seen == id)
        {
            sessions.push(id.to_owned());
        }

        match value.get("method").and_then(Value::as_str) {
            Some("session/prompt") => {
                // A prompt closes the previous turn, exactly as sending one
                // does in a live pane — and a turn can end with a tool still
                // in flight, so both halves of the live pane's settle run.
                settle(&mut out.items);
                out.items.push(ChatItem::UserText(prompt_text(&value)));
            }
            Some("session/update") => {
                let Some(update) = value.pointer("/params/update") else {
                    continue;
                };
                if let Ok(su) = serde_json::from_value::<SessionUpdate>(update.clone()) {
                    mapping::apply_update_with(&mut out.items, &su, adapter.as_ref());
                }
            }
            _ => {}
        }
    }
    settle(&mut out.items);
    out.sessions = sessions.len();
    out
}

/// End a turn the way a live pane does. A capture can stop mid-tool — the user
/// pressed Stop, or the turn errored — and a tool left `Pending` would render as
/// a spinner nothing will ever resolve. Mirrors `AgentChatView::settle_items`.
fn settle(items: &mut [ChatItem]) {
    mapping::finalize_streaming(items);
    mapping::cancel_pending_tools(items);
}

/// The user's prompt as typed — every `text` block joined. Non-text blocks
/// (images, resource links) carry no prose and are skipped.
fn prompt_text(value: &Value) -> String {
    let blocks = value
        .pointer("/params/prompt")
        .and_then(Value::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    if blocks.is_empty() {
        return EMPTY_PROMPT.to_owned();
    }
    blocks
}

/// Put spilled payloads back, in place. The inverse of the writer's elision
/// walk: every string that ends in a marker is restored from `payloads`, and one
/// that cannot be resolved is left exactly as written.
fn rehydrate(value: &mut Value, payloads: &HashMap<u64, String>, stats: &mut Replay) {
    match value {
        Value::String(text) => {
            let Some(marker) = marker_of(text) else {
                return;
            };
            match marker {
                Marker::Recorded(id) => match payloads.get(&id) {
                    Some(full) => {
                        text.clear();
                        text.push_str(full);
                        stats.rehydrated += 1;
                    }
                    None => stats.unresolved += 1,
                },
                Marker::Dropped => stats.unresolved += 1,
            }
        }
        Value::Array(items) => {
            for item in items {
                rehydrate(item, payloads, stats);
            }
        }
        Value::Object(fields) => {
            for (_, field) in fields.iter_mut() {
                rehydrate(field, payloads, stats);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire_log::{UNRECORDED_ID, payload_marker};

    fn text_update(text: &str) -> String {
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": "s1",
                "update": {
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": text},
                },
            },
        })
        .to_string()
    }

    fn assistant_text(items: &[ChatItem]) -> Vec<String> {
        items
            .iter()
            .filter_map(|it| match it {
                ChatItem::AssistantText { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_spilled_field_comes_back_whole() {
        // The round trip that matters: what the writer elided is what the
        // reader restores, so the mapper sees the original conversation.
        let full = "x".repeat(4096);
        let elided = format!("preview…{}", payload_marker("7", full.len()));
        let log = format!("123 <- stdout {}", text_update(&elided));
        let payloads = HashMap::from([(7, full.clone())]);

        let replay = replay_capture(&log, &payloads, "");

        assert_eq!(assistant_text(&replay.items), vec![full]);
        assert_eq!(replay.rehydrated, 1);
        assert_eq!(replay.unresolved, 0);
    }

    #[test]
    fn a_marker_with_no_sidecar_record_is_left_as_written() {
        // Better a visible marker than silently passing a truncated preview off
        // as the full text.
        let elided = format!("preview…{}", payload_marker("7", 4096));
        let log = format!("123 <- stdout {}", text_update(&elided));

        let replay = replay_capture(&log, &HashMap::new(), "");

        assert_eq!(assistant_text(&replay.items), vec![elided]);
        assert_eq!(replay.rehydrated, 0);
        assert_eq!(replay.unresolved, 1);
    }

    #[test]
    fn a_capture_taken_with_payloads_disabled_counts_as_unresolved() {
        let elided = format!("preview…{}", payload_marker(UNRECORDED_ID, 4096));
        let log = format!("123 <- stdout {}", text_update(&elided));

        let replay = replay_capture(&log, &HashMap::new(), "");

        assert_eq!(assistant_text(&replay.items), vec![elided]);
        assert_eq!(replay.unresolved, 1);
    }

    #[test]
    fn unelided_text_is_untouched_and_uncounted() {
        let log = format!("123 <- stdout {}", text_update("a short title"));

        let replay = replay_capture(&log, &HashMap::new(), "");

        assert_eq!(assistant_text(&replay.items), vec!["a short title"]);
        assert_eq!((replay.rehydrated, replay.unresolved), (0, 0));
    }

    #[test]
    fn a_prompt_becomes_the_users_own_words() {
        let prompt = serde_json::json!({
            "method": "session/prompt",
            "params": {
                "sessionId": "s1",
                "prompt": [{"type": "text", "text": "why is the chip hidden"}],
            },
        });
        let log = format!("123 -> stdin {prompt}\n124 <- stdout {}", text_update("ok"));

        let replay = replay_capture(&log, &HashMap::new(), "");

        assert_eq!(
            replay.items.first(),
            Some(&ChatItem::UserText("why is the chip hidden".into()))
        );
    }

    #[test]
    fn a_prompt_with_no_prose_still_anchors_a_turn() {
        let prompt = serde_json::json!({
            "method": "session/prompt",
            "params": {"sessionId": "s1", "prompt": [{"type": "image", "data": "AAAA"}]},
        });
        let log = format!("123 -> stdin {prompt}");

        let replay = replay_capture(&log, &HashMap::new(), "");

        assert_eq!(
            replay.items,
            vec![ChatItem::UserText(EMPTY_PROMPT.into())],
            "an attachment-only turn keeps its user row"
        );
    }

    #[test]
    fn several_sessions_in_one_capture_are_reported() {
        // A tap appends every session of a process run to one file, so a
        // capture can hold more than one. The caller has to be able to say so.
        let log = format!(
            "1 <- stdout {}\n2 <- stdout {}",
            text_update("first"),
            text_update("second").replace("\"s1\"", "\"s2\""),
        );

        let replay = replay_capture(&log, &HashMap::new(), "");

        assert_eq!(replay.sessions, 2);
    }

    #[test]
    fn noise_lines_are_skipped() {
        let log = format!(
            "1 !! stderr some adapter warning\n\n2 <- stdout {}",
            text_update("kept")
        );

        let replay = replay_capture(&log, &HashMap::new(), "");

        assert_eq!(assistant_text(&replay.items), vec!["kept"]);
    }

    #[test]
    fn the_dialect_follows_what_the_capture_reports() {
        // `initialize` naming codex must win over an empty catalog id, the same
        // way it does for a live session.
        let init = serde_json::json!({
            "id": 0,
            "result": {"agentInfo": {"name": "@agentclientprotocol/codex-acp"}},
        });
        let log = format!("1 <- stdout {init}");
        assert_eq!(
            reported_program(&log).as_deref(),
            Some("@agentclientprotocol/codex-acp")
        );
    }

    #[test]
    fn a_tool_still_running_when_the_turn_ends_is_cancelled() {
        // A capture can stop mid-tool. Without settling it, the replayed pane
        // shows a spinner that nothing will ever resolve.
        let start = serde_json::json!({
            "method": "session/update",
            "params": {"sessionId": "s1", "update": {
                "sessionUpdate": "tool_call",
                "toolCallId": "t1",
                "title": "git status",
                "status": "in_progress",
            }},
        });
        let log = format!("1 <- stdout {start}");

        let replay = replay_capture(&log, &HashMap::new(), "");

        let [ChatItem::ToolCall(tc)] = &replay.items[..] else {
            panic!("one tool call, got {:?}", replay.items.len());
        };
        assert_eq!(tc.status, crate::model::ToolStatusView::Cancelled);
    }

    #[test]
    fn a_corrupt_sidecar_line_costs_only_its_own_payload() {
        // The sidecar is appended to while a session runs, so its tail can be a
        // half-written line. One bad record must not take the good ones with it.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("acp-wire.log");
        let full = "y".repeat(4096);
        let good = format!("preview…{}", payload_marker("1", full.len()));
        let orphan = format!("preview…{}", payload_marker("2", 99));
        std::fs::write(
            &path,
            format!(
                "1 <- stdout {}\n2 <- stdout {}",
                text_update(&good),
                text_update(&orphan)
            ),
        )
        .expect("write log");
        std::fs::write(
            path.with_extension("payload.jsonl"),
            format!(
                "{}\n{{ not json\n{{\"id\": 2}}\n",
                serde_json::json!({"id": 1, "text": full})
            ),
        )
        .expect("write sidecar");

        let replay = replay_log(&path, "").expect("replayable");

        // Adjacent chunks of one message merge, so both fields land in one item.
        let [text] = &assistant_text(&replay.items)[..] else {
            panic!("one merged message, got {:?}", replay.items.len());
        };
        assert!(text.starts_with(&full), "the readable payload landed whole");
        assert!(
            text.ends_with(&orphan),
            "the field whose record is unreadable stayed exactly as written"
        );
        assert_eq!((replay.rehydrated, replay.unresolved), (1, 1));
    }

    #[test]
    fn a_log_with_no_updates_is_an_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("acp-wire.log");
        std::fs::write(&path, "1 !! stderr nothing useful here\n").expect("write");

        let err = replay_log(&path, "").expect_err("no updates");

        assert!(matches!(err, ReplayError::NoUpdates { .. }), "got {err:?}");
    }

    #[test]
    fn a_missing_log_reports_the_path() {
        let err = replay_log(Path::new("/nonexistent/acp-wire.log"), "").expect_err("missing");

        assert!(matches!(err, ReplayError::Read { .. }), "got {err:?}");
        assert!(err.to_string().contains("acp-wire.log"));
    }
}
