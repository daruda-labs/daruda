//! Hook event serde models — 9 of Claude Code's 28 events that daruda subscribes to.
//!
//! Schema source: <https://code.claude.com/docs/en/hooks.md>.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Fields present in every hook payload.
///
/// `session_id`, `cwd`, `transcript_path` are guaranteed by the official
/// docs; the rest are best-effort optionals so a future Claude Code
/// version that drops or renames a field still deserializes.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CommonFields {
    pub session_id: String,
    pub cwd: PathBuf,
    #[serde(default)]
    pub transcript_path: Option<PathBuf>,
    #[serde(default)]
    pub permission_mode: Option<PermissionMode>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub agent_type: Option<String>,
}

/// `permission_mode` — affects whether `PermissionRequest` ever fires.
/// `BypassPermissions` skips the dialog entirely.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PermissionMode {
    Default,
    Plan,
    AcceptEdits,
    Auto,
    DontAsk,
    BypassPermissions,
    /// Forward-compat: unknown mode strings deserialize here.
    #[serde(other)]
    Unknown,
}

/// `SessionStart.source` — distinguishes a fresh launch from a `/clear`
/// or `/compact` continuation that keeps the same `session_id`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionStartSource {
    Startup,
    Resume,
    Clear,
    Compact,
    #[serde(other)]
    Unknown,
}

/// `SessionEnd.end_reason`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionEndReason {
    Clear,
    Resume,
    Logout,
    PromptInputExit,
    BypassPermissionsDisabled,
    Other,
    #[serde(other)]
    Unknown,
}

/// `Notification.notification_type` — subtype carried on `Notification`
/// events. Blocking subtypes (`PermissionPrompt` / `IdlePrompt` /
/// `ElicitationDialog`) surface as a one-shot desktop push on the app
/// side; the others are informational. No subtype changes the persisted
/// session status (see `hooks::fsm`).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationType {
    PermissionPrompt,
    IdlePrompt,
    AuthSuccess,
    ElicitationDialog,
    ElicitationComplete,
    ElicitationResponse,
    #[serde(other)]
    Unknown,
}

/// All 9 hook events daruda processes. Tagged on `hook_event_name`,
/// which Claude Code includes in every payload.
///
/// Events outside this set (the other 19 of 28) won't deserialize
/// here — deserialization is fallible and the caller (`hooks::handler`)
/// just exits 0 on parse errors so unrecognized events are no-ops.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "hook_event_name")]
pub enum HookEvent {
    SessionStart {
        #[serde(flatten)]
        common: CommonFields,
        source: SessionStartSource,
        #[serde(default)]
        model: Option<String>,
    },
    SessionEnd {
        #[serde(flatten)]
        common: CommonFields,
        end_reason: SessionEndReason,
    },
    UserPromptSubmit {
        #[serde(flatten)]
        common: CommonFields,
        #[serde(default)]
        prompt: Option<String>,
    },
    PreToolUse {
        #[serde(flatten)]
        common: CommonFields,
        tool_name: String,
        #[serde(default)]
        tool_input: Option<serde_json::Value>,
        #[serde(default)]
        tool_use_id: Option<String>,
    },
    PostToolUse {
        #[serde(flatten)]
        common: CommonFields,
        tool_name: String,
        #[serde(default)]
        tool_input: Option<serde_json::Value>,
        #[serde(default)]
        tool_output: Option<String>,
        #[serde(default)]
        tool_use_id: Option<String>,
    },
    PostToolUseFailure {
        #[serde(flatten)]
        common: CommonFields,
        tool_name: String,
        #[serde(default)]
        tool_input: Option<serde_json::Value>,
        #[serde(default)]
        tool_output: Option<String>,
        #[serde(default)]
        tool_use_id: Option<String>,
    },
    PermissionRequest {
        #[serde(flatten)]
        common: CommonFields,
        tool_name: String,
        #[serde(default)]
        tool_input: Option<serde_json::Value>,
    },
    Notification {
        #[serde(flatten)]
        common: CommonFields,
        notification_type: NotificationType,
    },
    Stop {
        #[serde(flatten)]
        common: CommonFields,
        #[serde(default)]
        response: Option<String>,
    },
}

impl HookEvent {
    /// Common fields shared by every variant.
    pub fn common(&self) -> &CommonFields {
        match self {
            Self::SessionStart { common, .. }
            | Self::SessionEnd { common, .. }
            | Self::UserPromptSubmit { common, .. }
            | Self::PreToolUse { common, .. }
            | Self::PostToolUse { common, .. }
            | Self::PostToolUseFailure { common, .. }
            | Self::PermissionRequest { common, .. }
            | Self::Notification { common, .. }
            | Self::Stop { common, .. } => common,
        }
    }

    /// Convenience: the `tool_name` if this variant carries one.
    pub fn tool_name(&self) -> Option<&str> {
        match self {
            Self::PreToolUse { tool_name, .. }
            | Self::PostToolUse { tool_name, .. }
            | Self::PostToolUseFailure { tool_name, .. }
            | Self::PermissionRequest { tool_name, .. } => Some(tool_name),
            _ => None,
        }
    }

    /// Variant name as a static string — handy for logging and the
    /// `last_event` field of the persisted status file.
    pub fn name(&self) -> &'static str {
        match self {
            Self::SessionStart { .. } => "SessionStart",
            Self::SessionEnd { .. } => "SessionEnd",
            Self::UserPromptSubmit { .. } => "UserPromptSubmit",
            Self::PreToolUse { .. } => "PreToolUse",
            Self::PostToolUse { .. } => "PostToolUse",
            Self::PostToolUseFailure { .. } => "PostToolUseFailure",
            Self::PermissionRequest { .. } => "PermissionRequest",
            Self::Notification { .. } => "Notification",
            Self::Stop { .. } => "Stop",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let json =
            r#"{"hook_event_name":"WorktreeCreate","session_id":"x","cwd":"/t"}"#.to_string();
        // A-5 handler treats this as a no-op (exit 0). Here we just
        // confirm it doesn't silently match a known variant.
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
}
