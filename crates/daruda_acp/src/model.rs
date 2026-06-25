//! Chat render model — the GPUI-free shape the `app` view layer renders.
//!
//! ACP protocol traffic (`session/update` notifications and
//! `session/request_permission` requests) is folded into a flat list of
//! [`ChatItem`]s by [`crate::mapping`]. The view reads this list only; it never
//! touches protocol types.

use std::path::PathBuf;

/// One renderable item in the agent conversation, in arrival order.
#[derive(Debug, Clone, PartialEq)]
pub enum ChatItem {
    /// A user prompt (echoed locally on send, or streamed back by the agent).
    UserText(String),
    /// Assistant response text. `streaming` is true while chunks still arrive.
    AssistantText { text: String, streaming: bool },
    /// Agent internal reasoning. `streaming` mirrors [`ChatItem::AssistantText`].
    Thinking { text: String, streaming: bool },
    /// A tool invocation and its evolving status/output.
    ToolCall(ToolCallItem),
    /// A pending or resolved tool-permission request.
    Permission(PermissionItem),
    /// A surfaced error (connection or protocol failure).
    Error(String),
}

/// A tool call, updated in place as `ToolCallUpdate`s arrive.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCallItem {
    /// Stable id used to match later updates to this call.
    pub id: String,
    pub title: String,
    pub kind: ToolKindView,
    pub status: ToolStatusView,
    /// File modifications shown as diffs (rendered via daruda's diff editor).
    pub diffs: Vec<DiffView>,
    /// Plain-text output blocks produced by the tool.
    pub output: Vec<String>,
    /// Raw tool input, kept for an expandable "details" affordance.
    pub raw_input: Option<serde_json::Value>,
}

/// A file modification carried by a tool call.
#[derive(Debug, Clone, PartialEq)]
pub struct DiffView {
    pub path: PathBuf,
    /// `None` for a newly created file.
    pub old_text: Option<String>,
    pub new_text: String,
}

/// Execution status of a tool call (mirror of the protocol's `ToolCallStatus`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolStatusView {
    Pending,
    InProgress,
    Completed,
    Failed,
}

/// Tool category (mirror of the protocol's `ToolKind`) — drives icon/treatment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolKindView {
    Read,
    Edit,
    Delete,
    Move,
    Search,
    Execute,
    Think,
    Fetch,
    SwitchMode,
    Other,
}

/// A tool-permission request rendered as an inline card.
#[derive(Debug, Clone, PartialEq)]
pub struct PermissionItem {
    /// Title of the tool call needing approval, if the agent supplied one.
    pub tool_title: Option<String>,
    /// Choices to render as buttons, in the agent's order.
    pub options: Vec<PermissionChoice>,
    /// How the card was resolved, or `None` while still pending a decision.
    pub resolved: Option<PermissionResolution>,
}

/// How a pending permission card was resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionResolution {
    /// The user chose this option id.
    Chosen(String),
    /// The turn was cancelled before the user decided; the agent received a
    /// `Cancelled` outcome and the card's buttons are disabled.
    Cancelled,
}

/// One selectable permission choice.
#[derive(Debug, Clone, PartialEq)]
pub struct PermissionChoice {
    pub option_id: String,
    pub name: String,
    pub kind: PermissionKindView,
}

/// Hint about a permission choice (mirror of `PermissionOptionKind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionKindView {
    AllowOnce,
    AllowAlways,
    RejectOnce,
    RejectAlways,
}

/// One advertised session mode (mirror of the protocol's `SessionMode`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionModeView {
    /// Stable identifier used to switch into this mode via `set_mode`.
    pub id: String,
    /// Human-readable label shown in the mode chip.
    pub name: String,
    /// Optional longer description, shown in a tooltip or mode picker.
    pub description: Option<String>,
}

/// Mode state advertised at session-connect time and updated by
/// `CurrentModeUpdate` notifications (mirror of the protocol's
/// `SessionModeState`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModeStateView {
    /// The modes the agent supports.
    pub available: Vec<SessionModeView>,
    /// `id` of the mode the agent is currently in.
    pub current: String,
}

impl From<&agent_client_protocol::schema::v1::SessionModeState> for ModeStateView {
    fn from(s: &agent_client_protocol::schema::v1::SessionModeState) -> Self {
        ModeStateView {
            available: s
                .available_modes
                .iter()
                .map(|m| SessionModeView {
                    id: m.id.to_string(),
                    name: m.name.clone(),
                    description: m.description.clone(),
                })
                .collect(),
            current: s.current_mode_id.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use agent_client_protocol::schema::v1::{SessionMode, SessionModeState};

    use super::*;

    #[test]
    fn mode_state_view_from_session_mode_state_maps_fields() {
        let protocol_state = SessionModeState::new(
            "acceptEdits",
            vec![
                SessionMode::new("default", "Default"),
                SessionMode::new("acceptEdits", "Accept Edits")
                    .description("Automatically accept file edits"),
            ],
        );

        let view = ModeStateView::from(&protocol_state);

        assert_eq!(view.current, "acceptEdits");
        assert_eq!(view.available.len(), 2);

        let first = &view.available[0];
        assert_eq!(first.id, "default");
        assert_eq!(first.name, "Default");
        assert_eq!(first.description, None);

        let second = &view.available[1];
        assert_eq!(second.id, "acceptEdits");
        assert_eq!(second.name, "Accept Edits");
        assert_eq!(
            second.description.as_deref(),
            Some("Automatically accept file edits")
        );
    }

    #[test]
    fn mode_state_view_from_empty_available_modes() {
        let protocol_state = SessionModeState::new("default", vec![]);
        let view = ModeStateView::from(&protocol_state);
        assert_eq!(view.current, "default");
        assert!(view.available.is_empty());
    }

    #[test]
    fn into_on_option_ref_matches_from() {
        // Verify the `Into` blanket impl — used in session.rs as
        // `new_session.modes.as_ref().map(Into::into)`.
        let state = SessionModeState::new("plan", vec![SessionMode::new("plan", "Plan")]);
        let via_into: ModeStateView = (&state).into();
        let via_from = ModeStateView::from(&state);
        assert_eq!(via_into, via_from);
    }
}
