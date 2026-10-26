//! Determine [`SessionStatus`] from the tail of a session JSONL.
//!
//! Ported from c9watch (`src-tauri/src/session/status.rs:38-238`,
//! MIT — see `LICENSE-THIRD-PARTY.md`).
//!
//! Differences from the upstream:
//! - c9watch's `WaitingForInput` is renamed to [`SessionStatus::Idle`]
//!   (daruda's vocabulary).
//! - The permission checker is passed in by reference rather than
//!   stashed in a `OnceLock` global, so callers can swap it for
//!   different `~/.claude/settings.json` snapshots in tests.
//! - Explicit-clock classification also reports the next quiet transition;
//!   the helpers c9watch exposes for `pending_tool_name` are dropped because
//!   the hook channel already carries that information when daruda needs it.

use chrono::{DateTime, Utc};

use crate::SessionStatus;
use crate::jsonl::parser::{AssistantMessage, MessageContent, SessionEntry};
use crate::jsonl::permissions::PermissionChecker;

/// Wall-clock thresholds (seconds). Tuned by c9watch through real
/// usage; reused as-is in daruda.
mod thresholds {
    /// `User` entry must be this fresh to count as `Working` rather
    /// than as a stale prompt that never got picked up.
    pub const USER_RECENT_S: i64 = 30;
    /// Pending `tool_use` keeps the session in `Working` for this
    /// long after the entry timestamp, accommodating tool execution.
    pub const PENDING_TOOL_RECENT_S: i64 = 20;
    /// Streaming text without a `stop_reason` keeps `Working` for
    /// this long. Kept short (5 s) so a session interrupted by Escape
    /// falls back to `Idle` quickly: the file stops being modified, so
    /// the watcher emits no new events, and the last Working event ages
    /// out. During active streaming the file is modified continuously,
    /// so a new event arrives well within this window.
    pub const STREAMING_RECENT_S: i64 = 5;
    /// Anti-flicker delay before treating an `?`-ending text answer
    /// as a question that needs the user.
    pub const ASK_QUESTION_DELAY_S: i64 = 20;
}

/// A status assessment and the next instant at which quiet input changes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StatusClassification {
    pub status: SessionStatus,
    pub next_transition_at: Option<DateTime<Utc>>,
}

/// Classify a fixed transcript snapshot using an explicit wall clock.
/// The deadline is absent once time alone cannot change the status.
pub fn classify_status_at(
    entries: &[SessionEntry],
    permissions: &PermissionChecker,
    now: DateTime<Utc>,
) -> StatusClassification {
    let status = determine_status_at(entries, permissions, now);
    let last = entries.iter().rev().find(|entry| {
        matches!(
            entry,
            SessionEntry::User { .. } | SessionEntry::Assistant { .. }
        )
    });
    let delay = match last {
        Some(SessionEntry::User { .. }) => Some(thresholds::USER_RECENT_S),
        Some(SessionEntry::Assistant { message, .. }) => {
            if is_assistant_asking_question(message) {
                Some(thresholds::ASK_QUESTION_DELAY_S)
            } else if status == SessionStatus::Working && !has_pending_tool_uses(&message.content) {
                Some(thresholds::STREAMING_RECENT_S)
            } else {
                None
            }
        }
        _ => None,
    };
    let next_transition_at = last_meaningful_timestamp(entries)
        .zip(delay)
        .and_then(|(timestamp, seconds)| {
            timestamp.checked_add_signed(chrono::Duration::seconds(seconds))
        })
        .filter(|deadline| {
            *deadline > now && determine_status_at(entries, permissions, *deadline) != status
        });
    StatusClassification {
        status,
        next_transition_at,
    }
}

/// Decide the session's current status from its tail entries.
///
/// `entries` is typically the last 20 lines of the JSONL file
/// (see `super::tail::read_last_n_lines`).
pub fn determine_status(
    entries: &[SessionEntry],
    permissions: &PermissionChecker,
) -> SessionStatus {
    determine_status_at(entries, permissions, Utc::now())
}

fn determine_status_at(
    entries: &[SessionEntry],
    permissions: &PermissionChecker,
    now: DateTime<Utc>,
) -> SessionStatus {
    if entries.is_empty() {
        return SessionStatus::Connecting;
    }

    // Find the last meaningful entry — skip progress / file-history
    // / summary noise so they can't override real status.
    let last_meaningful_idx = entries.iter().rposition(|entry| {
        matches!(
            entry,
            SessionEntry::User { .. } | SessionEntry::Assistant { .. }
        )
    });
    let Some(idx) = last_meaningful_idx else {
        return SessionStatus::Connecting;
    };

    // "Trailing progress" — bash_progress and friends arriving after
    // the last meaningful entry indicate that a tool is still
    // running.
    let has_trailing_progress = entries[idx + 1..]
        .iter()
        .any(|e| matches!(e, SessionEntry::Unknown));

    match &entries[idx] {
        SessionEntry::User { base, message } => {
            if message.is_tool_result {
                if is_recent(&base.timestamp, thresholds::USER_RECENT_S, now) {
                    SessionStatus::Working
                } else {
                    SessionStatus::Idle
                }
            } else if is_recent(&base.timestamp, thresholds::USER_RECENT_S, now) {
                SessionStatus::Working
            } else {
                SessionStatus::Idle
            }
        }

        SessionEntry::Assistant { base, message } => {
            // AskUserQuestion tool fires NeedsAttention immediately.
            if has_pending_ask_user_question(&message.content) {
                return SessionStatus::NeedsAttention;
            }
            // "?"-ending text answer with end_turn — wait the
            // anti-flicker delay so we don't catch a partial stream.
            if is_assistant_asking_question(message)
                && !is_recent(&base.timestamp, thresholds::ASK_QUESTION_DELAY_S, now)
            {
                return SessionStatus::NeedsAttention;
            }

            match analyze_assistant_message(message, permissions) {
                SessionStatus::Working => {
                    if has_pending_tool_uses(&message.content) {
                        // Pending tool: active execution while recent
                        // or trailing progress is present.
                        if has_trailing_progress
                            || is_recent(&base.timestamp, thresholds::PENDING_TOOL_RECENT_S, now)
                        {
                            SessionStatus::ExecutingTool
                        } else {
                            // Stale pending tool — assume still running;
                            // hook channel will correct if finished.
                            SessionStatus::ExecutingTool
                        }
                    } else if is_recent(&base.timestamp, thresholds::STREAMING_RECENT_S, now) {
                        // Streaming text without stop_reason.
                        SessionStatus::Working
                    } else {
                        SessionStatus::Idle
                    }
                }
                SessionStatus::NeedsAttention => SessionStatus::NeedsAttention,
                other => other,
            }
        }

        // Unreachable given the rposition filter above.
        _ => SessionStatus::Idle,
    }
}

/// Return the wall-clock timestamp of the last meaningful (`User` or
/// `Assistant`) entry as a `DateTime<Utc>`, or `None` when the tail
/// contains no such entries or the timestamp string is malformed.
///
/// The JSONL watcher uses this to stamp `JsonlEvent` with the entry's
/// own clock rather than `Utc::now()`. This ensures the store's race
/// policy (hook wins when hook timestamp ≥ JSONL timestamp) correctly
/// orders a `Stop` hook (written at cancellation time T) against a
/// JSONL event derived from an incomplete assistant message that was
/// written at T-N seconds before the user pressed Escape. Without
/// this, `Utc::now()` on every poll would make the JSONL event always
/// appear newer than the hook, silently reverting `Idle` to `Working`.
pub fn last_meaningful_timestamp(entries: &[SessionEntry]) -> Option<DateTime<Utc>> {
    let idx = entries.iter().rposition(|e| {
        matches!(
            e,
            SessionEntry::User { .. } | SessionEntry::Assistant { .. }
        )
    })?;
    let ts = match &entries[idx] {
        SessionEntry::User { base, .. } | SessionEntry::Assistant { base, .. } => &base.timestamp,
        _ => return None,
    };
    DateTime::parse_from_rfc3339(ts)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

/// Pure helper — true if `timestamp` (RFC 3339) is within the last
/// `seconds`. Unparseable timestamps return `false`.
fn is_recent(timestamp: &str, seconds: i64, now: DateTime<Utc>) -> bool {
    let Ok(t) = DateTime::parse_from_rfc3339(timestamp) else {
        return false;
    };
    let age = now.signed_duration_since(t.with_timezone(&Utc));
    age < chrono::Duration::seconds(seconds)
}

fn has_pending_ask_user_question(content: &[MessageContent]) -> bool {
    let completed: Vec<&str> = content
        .iter()
        .filter_map(|c| {
            if let MessageContent::ToolResult { tool_use_id, .. } = c {
                Some(tool_use_id.as_str())
            } else {
                None
            }
        })
        .collect();
    content.iter().any(|c| {
        matches!(
            c,
            MessageContent::ToolUse { id, name, .. }
                if name == "AskUserQuestion" && !completed.contains(&id.as_str())
        )
    })
}

fn is_assistant_asking_question(message: &AssistantMessage) -> bool {
    if has_pending_ask_user_question(&message.content) {
        return true;
    }

    if message.stop_reason.as_deref() != Some("end_turn") {
        return false;
    }

    let last_text = message.content.iter().rev().find_map(|c| {
        if let MessageContent::Text { text } = c {
            Some(text.as_str())
        } else {
            None
        }
    });

    let Some(text) = last_text else {
        return false;
    };
    let trimmed = text.trim();
    trimmed.ends_with('?') || trimmed.ends_with("?)")
}

fn analyze_assistant_message(
    message: &AssistantMessage,
    permissions: &PermissionChecker,
) -> SessionStatus {
    let has_tool_use = message
        .content
        .iter()
        .any(|c| matches!(c, MessageContent::ToolUse { .. }));

    if has_tool_use {
        if check_all_tools_completed(&message.content) {
            // All tool_use entries have a matching tool_result.
            match message.stop_reason.as_deref() {
                Some("tool_use") => SessionStatus::Working,
                _ => SessionStatus::Idle,
            }
        } else if are_pending_tools_auto_approved(&message.content, permissions) {
            SessionStatus::ExecutingTool
        } else {
            SessionStatus::NeedsAttention
        }
    } else {
        match message.stop_reason.as_deref() {
            None => SessionStatus::Working,
            _ => SessionStatus::Idle,
        }
    }
}

fn are_pending_tools_auto_approved(
    content: &[MessageContent],
    permissions: &PermissionChecker,
) -> bool {
    let completed: Vec<&str> = content
        .iter()
        .filter_map(|c| {
            if let MessageContent::ToolResult { tool_use_id, .. } = c {
                Some(tool_use_id.as_str())
            } else {
                None
            }
        })
        .collect();

    for item in content {
        if let MessageContent::ToolUse { id, name, input } = item {
            if completed.contains(&id.as_str()) {
                continue;
            }
            if !permissions.is_auto_approved(name, input) {
                return false;
            }
        }
    }
    true
}

fn has_pending_tool_uses(content: &[MessageContent]) -> bool {
    !check_all_tools_completed(content)
}

fn check_all_tools_completed(content: &[MessageContent]) -> bool {
    let mut tool_use_ids = Vec::new();
    for item in content {
        if let MessageContent::ToolUse { id, .. } = item {
            tool_use_ids.push(id.as_str());
        }
    }
    if tool_use_ids.is_empty() {
        return true;
    }
    let tool_result_ids: Vec<&str> = content
        .iter()
        .filter_map(|c| {
            if let MessageContent::ToolResult { tool_use_id, .. } = c {
                Some(tool_use_id.as_str())
            } else {
                None
            }
        })
        .collect();
    tool_use_ids.iter().all(|id| tool_result_ids.contains(id))
}
