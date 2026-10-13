//! Claude Code JSONL session-log entry models.
//!
//! Ported from c9watch (`src-tauri/src/session/parser.rs:34-240`,
//! MIT — see `LICENSE-THIRD-PARTY.md`). Only the entry types the FSM
//! consults are kept; the sessions-index / system-tag detection logic
//! is dropped because daruda receives `transcript_path` directly from
//! hook payloads and never has to discover sessions itself.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// One line from `~/.claude/projects/<encoded>/<session>.jsonl`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum SessionEntry {
    User {
        #[serde(flatten)]
        base: SessionEntryBase,
        message: UserMessage,
    },
    Assistant {
        #[serde(flatten)]
        base: SessionEntryBase,
        message: AssistantMessage,
    },
    #[serde(rename = "file-history-snapshot")]
    FileHistorySnapshot {
        #[serde(rename = "messageId")]
        message_id: String,
        snapshot: serde_json::Value,
        #[serde(rename = "isSnapshotUpdate")]
        is_snapshot_update: bool,
    },
    Summary {
        summary: String,
        #[serde(rename = "leafUuid")]
        leaf_uuid: String,
    },
    #[serde(rename = "custom-title")]
    CustomTitle {
        #[serde(rename = "customTitle")]
        custom_title: String,
        #[serde(rename = "sessionId")]
        session_id: String,
    },
    /// Bash progress, file-watch, future entry kinds, etc. Counts as
    /// "trailing progress" in the FSM but carries no other state.
    #[serde(other)]
    Unknown,
}

/// Common fields shared across `User` and `Assistant` entries.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionEntryBase {
    pub uuid: String,
    pub timestamp: String,
    pub session_id: Option<String>,
    pub cwd: Option<PathBuf>,
    pub version: Option<String>,
    pub git_branch: Option<String>,
    pub parent_uuid: Option<String>,
    pub is_sidechain: Option<bool>,
    pub slug: Option<String>,
}

/// Image attached to a user message.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ImageBlock {
    pub media_type: String,
    pub data: String,
}

/// Custom-deserialised user message.
///
/// In Claude Code's JSONL, `message.content` is either a plain string
/// (a real user prompt) or an array of typed blocks (a tool result
/// being fed back to Claude). The FSM treats these two cases very
/// differently, so we surface a `is_tool_result` boolean.
#[derive(Debug, Clone, Serialize)]
pub struct UserMessage {
    pub role: String,
    pub content: String,
    pub is_tool_result: bool,
    pub images: Vec<ImageBlock>,
}

impl<'de> Deserialize<'de> for UserMessage {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde_json::Value;

        let value = Value::deserialize(deserializer)?;
        let role = value
            .get("role")
            .and_then(|r| r.as_str())
            .unwrap_or("user")
            .to_string();
        let content_value = value.get("content");

        let mut images = Vec::new();
        let (content, is_tool_result) = match content_value {
            Some(Value::String(s)) => (s.clone(), false),
            Some(Value::Array(arr)) => {
                let mut parts = Vec::new();
                let mut has_tool_result = false;
                for item in arr {
                    match item.get("type").and_then(|t| t.as_str()) {
                        Some("tool_result") => {
                            has_tool_result = true;
                            if let Some(content) = item.get("content") {
                                match content {
                                    Value::String(s) => parts.push(s.clone()),
                                    Value::Array(inner) => {
                                        for block in inner {
                                            if let Some(text) =
                                                block.get("text").and_then(|t| t.as_str())
                                            {
                                                parts.push(text.to_string());
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                        Some("text") => {
                            if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                                parts.push(text.to_string());
                            }
                        }
                        Some("image") => {
                            if let Some(source) = item.get("source") {
                                let media_type = source
                                    .get("media_type")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("image/png")
                                    .to_string();
                                if let Some(data) = source.get("data").and_then(|v| v.as_str()) {
                                    images.push(ImageBlock {
                                        media_type,
                                        data: data.to_string(),
                                    });
                                }
                            }
                        }
                        _ => {}
                    }
                }
                let text = if parts.is_empty() && !has_tool_result {
                    String::new()
                } else if parts.is_empty() {
                    "[tool result]".to_string()
                } else {
                    parts.join("\n")
                };
                (text, has_tool_result)
            }
            _ => (String::new(), false),
        };

        Ok(UserMessage {
            role,
            content,
            is_tool_result,
            images,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssistantMessage {
    pub model: String,
    pub id: String,
    pub role: String,
    pub content: Vec<MessageContent>,
    pub stop_reason: Option<String>,
    pub stop_sequence: Option<String>,
    pub usage: Option<Usage>,
}

/// Content block inside an `assistant` message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessageContent {
    Text {
        text: String,
    },
    Thinking {
        thinking: String,
        signature: Option<String>,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: Option<bool>,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
    pub cache_creation_input_tokens: Option<u32>,
    pub cache_read_input_tokens: Option<u32>,
}

/// Parse a sequence of JSONL lines into entries, skipping any that
/// fail to deserialise.
pub fn parse_jsonl_entries(lines: &[String]) -> Vec<SessionEntry> {
    lines
        .iter()
        .filter_map(|line| serde_json::from_str::<SessionEntry>(line).ok())
        .collect()
}
