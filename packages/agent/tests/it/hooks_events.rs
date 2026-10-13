//! Tests for the public `daruda_agent::hooks::events` API.

use daruda_agent::hooks::events::*;
use std::path::PathBuf;

fn parse(json: &str) -> HookEvent {
    serde_json::from_str(json).expect("parse hook event")
}

fn common_json() -> &'static str {
    r#""session_id":"abc-123","cwd":"/tmp/x","transcript_path":"/tmp/t.jsonl","permission_mode":"default""#
}

#[test]
fn session_start_with_source() {
    let json = format!(
        r#"{{"hook_event_name":"SessionStart",{},"source":"startup","model":"claude-sonnet-4-6"}}"#,
        common_json()
    );
    let ev = parse(&json);
    match ev {
        HookEvent::SessionStart {
            source,
            model,
            common,
        } => {
            assert_eq!(source, SessionStartSource::Startup);
            assert_eq!(model.as_deref(), Some("claude-sonnet-4-6"));
            assert_eq!(common.session_id, "abc-123");
            assert_eq!(common.cwd, PathBuf::from("/tmp/x"));
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

#[test]
fn session_end_with_reason() {
    let json = format!(
        r#"{{"hook_event_name":"SessionEnd",{},"end_reason":"prompt_input_exit"}}"#,
        common_json()
    );
    let ev = parse(&json);
    assert!(matches!(
        ev,
        HookEvent::SessionEnd {
            end_reason: SessionEndReason::PromptInputExit,
            ..
        }
    ));
}

#[test]
fn user_prompt_submit() {
    let json = format!(
        r#"{{"hook_event_name":"UserPromptSubmit",{},"prompt":"hello"}}"#,
        common_json()
    );
    let ev = parse(&json);
    assert!(matches!(ev, HookEvent::UserPromptSubmit { .. }));
    assert_eq!(ev.name(), "UserPromptSubmit");
}

#[test]
fn pre_tool_use_with_input() {
    let json = format!(
        r#"{{"hook_event_name":"PreToolUse",{},"tool_name":"Bash","tool_input":{{"command":"ls"}},"tool_use_id":"tu_1"}}"#,
        common_json()
    );
    let ev = parse(&json);
    assert_eq!(ev.tool_name(), Some("Bash"));
    if let HookEvent::PreToolUse {
        tool_input: Some(input),
        ..
    } = ev
    {
        assert_eq!(input["command"], "ls");
    } else {
        panic!("expected PreToolUse with input");
    }
}

#[test]
fn post_tool_use_failure_carries_output() {
    let json = format!(
        r#"{{"hook_event_name":"PostToolUseFailure",{},"tool_name":"Edit","tool_output":"file not found"}}"#,
        common_json()
    );
    let ev = parse(&json);
    match ev {
        HookEvent::PostToolUseFailure {
            tool_output: Some(out),
            ..
        } => assert_eq!(out, "file not found"),
        other => panic!("wrong variant: {other:?}"),
    }
}

#[test]
fn permission_request_carries_tool() {
    let json = format!(
        r#"{{"hook_event_name":"PermissionRequest",{},"tool_name":"Bash","tool_input":{{"command":"rm -rf /"}}}}"#,
        common_json()
    );
    let ev = parse(&json);
    assert_eq!(ev.tool_name(), Some("Bash"));
    assert!(matches!(ev, HookEvent::PermissionRequest { .. }));
}

#[test]
fn notification_filters_to_known_types() {
    for (raw, expected) in [
        ("permission_prompt", NotificationType::PermissionPrompt),
        ("idle_prompt", NotificationType::IdlePrompt),
        ("auth_success", NotificationType::AuthSuccess),
        ("elicitation_dialog", NotificationType::ElicitationDialog),
        (
            "elicitation_complete",
            NotificationType::ElicitationComplete,
        ),
        (
            "elicitation_response",
            NotificationType::ElicitationResponse,
        ),
    ] {
        let json = format!(
            r#"{{"hook_event_name":"Notification",{},"notification_type":"{raw}"}}"#,
            common_json()
        );
        let ev = parse(&json);
        match ev {
            HookEvent::Notification {
                notification_type, ..
            } => assert_eq!(notification_type, expected),
            _ => panic!("expected Notification"),
        }
    }
}

#[test]
fn notification_unknown_type_falls_back() {
    let json = format!(
        r#"{{"hook_event_name":"Notification",{},"notification_type":"future_thing"}}"#,
        common_json()
    );
    let ev = parse(&json);
    assert!(matches!(
        ev,
        HookEvent::Notification {
            notification_type: NotificationType::Unknown,
            ..
        }
    ));
}

#[test]
fn stop_with_response() {
    let json = format!(
        r#"{{"hook_event_name":"Stop",{},"response":"done"}}"#,
        common_json()
    );
    let ev = parse(&json);
    assert!(matches!(ev, HookEvent::Stop { .. }));
}

#[test]
fn stop_failure_parses_and_names() {
    // `StopFailure` (turn ended due to an API error) carries only the
    // common fields; any error payload Claude adds is ignored.
    let json = format!(r#"{{"hook_event_name":"StopFailure",{}}}"#, common_json());
    let ev = parse(&json);
    assert!(matches!(ev, HookEvent::StopFailure { .. }));
    assert_eq!(ev.name(), "StopFailure");
    assert_eq!(ev.common().session_id, "abc-123");
}

#[test]
fn permission_mode_camel_case() {
    let json = r#"{"hook_event_name":"UserPromptSubmit","session_id":"x","cwd":"/t","permission_mode":"bypassPermissions"}"#.to_string();
    let ev = parse(&json);
    assert_eq!(
        ev.common().permission_mode,
        Some(PermissionMode::BypassPermissions)
    );
}

#[test]
fn unknown_hook_event_name_fails_to_parse() {
    let json = r#"{"hook_event_name":"WorktreeCreate","session_id":"x","cwd":"/t"}"#.to_string();
    // The hook handler treats an unparseable event as a no-op (exit 0).
    // Here we just confirm it doesn't silently match a known variant.
    let res: Result<HookEvent, _> = serde_json::from_str(&json);
    assert!(res.is_err());
}

#[test]
fn unknown_session_start_source_falls_back() {
    let json = r#"{"hook_event_name":"SessionStart","session_id":"x","cwd":"/t","source":"future-source-kind"}"#.to_string();
    let ev = parse(&json);
    match ev {
        HookEvent::SessionStart { source, .. } => {
            assert_eq!(source, SessionStartSource::Unknown)
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn missing_optional_fields_ok() {
    let json = r#"{"hook_event_name":"Stop","session_id":"x","cwd":"/t"}"#.to_string();
    let ev = parse(&json);
    match ev {
        HookEvent::Stop { common, response } => {
            assert_eq!(common.session_id, "x");
            assert!(response.is_none());
            assert!(common.transcript_path.is_none());
            assert!(common.permission_mode.is_none());
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn common_accessor_works() {
    let json = format!(
        r#"{{"hook_event_name":"PreToolUse",{},"tool_name":"Read"}}"#,
        common_json()
    );
    let ev = parse(&json);
    assert_eq!(ev.common().session_id, "abc-123");
}
