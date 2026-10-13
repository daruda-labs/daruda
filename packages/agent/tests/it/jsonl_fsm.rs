//! Tests for the public `daruda_agent::jsonl::fsm` API.

use chrono::{Duration as CDuration, Utc};
use daruda_agent::SessionStatus;
use daruda_agent::jsonl::fsm::*;
use daruda_agent::jsonl::parser::{AssistantMessage, ImageBlock, SessionEntryBase, UserMessage};
use daruda_agent::jsonl::parser::{MessageContent, SessionEntry};
use daruda_agent::jsonl::permissions::PermissionChecker;

fn rfc3339_ago(age_secs: i64) -> String {
    (Utc::now() - CDuration::seconds(age_secs))
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

fn user_entry(content: &str, is_tool_result: bool, age_secs: i64) -> SessionEntry {
    SessionEntry::User {
        base: base(age_secs),
        message: UserMessage {
            role: "user".into(),
            content: content.into(),
            is_tool_result,
            images: Vec::<ImageBlock>::new(),
        },
    }
}

fn assistant_entry(
    content: Vec<MessageContent>,
    stop: Option<&str>,
    age_secs: i64,
) -> SessionEntry {
    SessionEntry::Assistant {
        base: base(age_secs),
        message: AssistantMessage {
            model: "x".into(),
            id: "m".into(),
            role: "assistant".into(),
            content,
            stop_reason: stop.map(|s| s.into()),
            stop_sequence: None,
            usage: None,
        },
    }
}

fn base(age_secs: i64) -> SessionEntryBase {
    SessionEntryBase {
        uuid: "u".into(),
        timestamp: rfc3339_ago(age_secs),
        session_id: None,
        cwd: None,
        version: None,
        git_branch: None,
        parent_uuid: None,
        is_sidechain: None,
        slug: None,
    }
}

fn empty_perms() -> PermissionChecker {
    PermissionChecker::default()
}

#[test]
fn empty_yields_connecting() {
    assert_eq!(
        determine_status(&[], &empty_perms()),
        SessionStatus::Connecting
    );
}

#[test]
fn fresh_user_prompt_is_working() {
    let entries = vec![user_entry("hi", false, 5)];
    assert_eq!(
        determine_status(&entries, &empty_perms()),
        SessionStatus::Working
    );
}

#[test]
fn old_user_prompt_is_idle() {
    let entries = vec![user_entry("hi", false, 60)];
    assert_eq!(
        determine_status(&entries, &empty_perms()),
        SessionStatus::Idle
    );
}

#[test]
fn fresh_tool_result_is_working() {
    let entries = vec![user_entry("[result]", true, 5)];
    assert_eq!(
        determine_status(&entries, &empty_perms()),
        SessionStatus::Working
    );
}

#[test]
fn assistant_end_turn_text_is_idle() {
    let entries = vec![assistant_entry(
        vec![MessageContent::Text {
            text: "all done".into(),
        }],
        Some("end_turn"),
        5,
    )];
    assert_eq!(
        determine_status(&entries, &empty_perms()),
        SessionStatus::Idle
    );
}

#[test]
fn assistant_streaming_text_is_working() {
    let entries = vec![assistant_entry(
        vec![MessageContent::Text {
            text: "thinking…".into(),
        }],
        None,
        2, // 2 s old — within STREAMING_RECENT_S (5 s)
    )];
    assert_eq!(
        determine_status(&entries, &empty_perms()),
        SessionStatus::Working
    );
}

#[test]
fn stale_streaming_text_is_idle() {
    // After STREAMING_RECENT_S expires, an incomplete assistant
    // message (stop_reason: None) with no file activity falls to
    // Idle. This is the JSONL-only path for Escape-interrupted
    // sessions: the file stops being modified, so no new watcher
    // event arrives and the last Working state ages out quickly.
    let entries = vec![assistant_entry(
        vec![MessageContent::Text {
            text: "thinking…".into(),
        }],
        None,
        10, // 10 s old — past STREAMING_RECENT_S (5 s)
    )];
    assert_eq!(
        determine_status(&entries, &empty_perms()),
        SessionStatus::Idle
    );
}

#[test]
fn pending_tool_needing_permission_is_needs_attention() {
    let entries = vec![assistant_entry(
        vec![MessageContent::ToolUse {
            id: "t1".into(),
            name: "Bash".into(),
            input: serde_json::json!({"command": "rm -rf /"}),
        }],
        None,
        2,
    )];
    assert_eq!(
        determine_status(&entries, &empty_perms()),
        SessionStatus::NeedsAttention
    );
}

#[test]
fn pending_auto_approved_tool_is_executing_tool() {
    let entries = vec![assistant_entry(
        vec![MessageContent::ToolUse {
            id: "t1".into(),
            name: "Read".into(),
            input: serde_json::json!({"file_path": "/etc/hosts"}),
        }],
        None,
        2,
    )];
    assert_eq!(
        determine_status(&entries, &empty_perms()),
        SessionStatus::ExecutingTool
    );
}

#[test]
fn ask_user_question_immediate_needs_attention() {
    let entries = vec![assistant_entry(
        vec![MessageContent::ToolUse {
            id: "q1".into(),
            name: "AskUserQuestion".into(),
            input: serde_json::json!({}),
        }],
        None,
        1,
    )];
    assert_eq!(
        determine_status(&entries, &empty_perms()),
        SessionStatus::NeedsAttention
    );
}

#[test]
fn question_mark_text_with_delay() {
    // Recent — no transition yet (anti-flicker).
    let recent = vec![assistant_entry(
        vec![MessageContent::Text {
            text: "What do you want?".into(),
        }],
        Some("end_turn"),
        5,
    )];
    assert_eq!(
        determine_status(&recent, &empty_perms()),
        SessionStatus::Idle
    );
    // Past the delay — transitions to NeedsAttention.
    let old = vec![assistant_entry(
        vec![MessageContent::Text {
            text: "What do you want?".into(),
        }],
        Some("end_turn"),
        30,
    )];
    assert_eq!(
        determine_status(&old, &empty_perms()),
        SessionStatus::NeedsAttention
    );
}

#[test]
fn trailing_progress_keeps_pending_tool_executing() {
    let entries = vec![
        assistant_entry(
            vec![MessageContent::ToolUse {
                id: "t1".into(),
                name: "Read".into(),
                input: serde_json::json!({}),
            }],
            None,
            300, // very old
        ),
        // Trailing progress entry of unknown type.
        SessionEntry::Unknown,
    ];
    assert_eq!(
        determine_status(&entries, &empty_perms()),
        SessionStatus::ExecutingTool
    );
}

#[test]
fn completed_tool_with_end_turn_is_idle() {
    let entries = vec![assistant_entry(
        vec![
            MessageContent::ToolUse {
                id: "t1".into(),
                name: "Read".into(),
                input: serde_json::json!({}),
            },
            MessageContent::ToolResult {
                tool_use_id: "t1".into(),
                content: "data".into(),
                is_error: None,
            },
        ],
        Some("end_turn"),
        5,
    )];
    assert_eq!(
        determine_status(&entries, &empty_perms()),
        SessionStatus::Idle
    );
}

#[test]
fn last_meaningful_timestamp_returns_entry_time() {
    let entries = vec![assistant_entry(
        vec![MessageContent::Text { text: "hi".into() }],
        Some("end_turn"),
        10, // 10 seconds ago
    )];
    let ts = last_meaningful_timestamp(&entries);
    assert!(ts.is_some());
    let age = Utc::now().signed_duration_since(ts.unwrap());
    // Should be ~10 seconds old, not 0 (i.e. not Utc::now()).
    assert!(age.num_seconds() >= 9 && age.num_seconds() <= 12);
}

#[test]
fn last_meaningful_timestamp_none_for_empty() {
    assert!(last_meaningful_timestamp(&[]).is_none());
    assert!(last_meaningful_timestamp(&[SessionEntry::Unknown]).is_none());
}

#[test]
fn last_meaningful_timestamp_skips_trailing_unknown() {
    // Unknown entries after the last real entry should not affect
    // which timestamp is returned.
    let entries = vec![
        assistant_entry(
            vec![MessageContent::Text {
                text: "done".into(),
            }],
            Some("end_turn"),
            15,
        ),
        SessionEntry::Unknown,
    ];
    let ts = last_meaningful_timestamp(&entries);
    assert!(ts.is_some());
    let age = Utc::now().signed_duration_since(ts.unwrap());
    assert!(age.num_seconds() >= 14 && age.num_seconds() <= 17);
}
