//! Hook event serde models — 10 of Claude Code's 28 events that daruda subscribes to.
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

/// The hook events daruda processes. Tagged on `hook_event_name`,
/// which Claude Code includes in every payload.
///
/// Events outside this set (the rest of Claude Code's hook catalogue)
/// won't deserialize here — deserialization is fallible and the caller
/// (`hooks::handler`) just exits 0 on parse errors so unrecognized
/// events are no-ops.
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
    /// Turn ended due to an API error (rather than a clean finish).
    /// Carries only the common fields; any error detail Claude attaches
    /// is ignored. Treated like `Stop` for status purposes — the turn is
    /// over, so the session returns to `Idle`.
    StopFailure {
        #[serde(flatten)]
        common: CommonFields,
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
            | Self::Stop { common, .. }
            | Self::StopFailure { common, .. } => common,
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
            Self::StopFailure { .. } => "StopFailure",
        }
    }
}
