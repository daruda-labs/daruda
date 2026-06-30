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
    ///
    /// `message_id` is the ACP message this text belongs to (when the agent
    /// supplies one): consecutive chunks sharing it accrue into this one item,
    /// and a change in `message_id` starts a new item — so distinct agent
    /// messages within a turn stay separate even with no tool call between
    /// them. `None` when the agent omits it; then chunks merge by adjacency
    /// (legacy behaviour). The renderer uses the last `AssistantText` of a
    /// response run as the turn's "conclusion".
    AssistantText {
        text: String,
        streaming: bool,
        message_id: Option<String>,
    },
    /// Agent internal reasoning. `streaming` / `message_id` mirror
    /// [`ChatItem::AssistantText`].
    Thinking {
        text: String,
        streaming: bool,
        message_id: Option<String>,
    },
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
    /// The turn was stopped before this tool call settled. Set host-side by
    /// [`crate::cancel_pending_tools`] when the user cancels — agents never
    /// emit it, so it has no `ToolCallStatus` mapping.
    Cancelled,
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

/// A single entry of the agent's execution plan (mirror of the protocol's
/// `PlanEntry`). The agent replaces the whole plan on each update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanEntryView {
    pub content: String,
    /// Mapped from protocol but not yet consumed by the UI — kept for future
    /// priority-based sorting or visual differentiation.
    pub priority: PlanPriority,
    pub status: PlanStatus,
}

/// Execution status of a plan entry (mirror of the protocol's
/// `PlanEntryStatus`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanStatus {
    Pending,
    InProgress,
    Completed,
}

/// Priority level of a plan entry (mirror of the protocol's
/// `PlanEntryPriority`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanPriority {
    High,
    Medium,
    Low,
}

impl From<&agent_client_protocol::schema::v1::PlanEntry> for PlanEntryView {
    fn from(e: &agent_client_protocol::schema::v1::PlanEntry) -> Self {
        use agent_client_protocol::schema::v1::{PlanEntryPriority, PlanEntryStatus};
        let priority = match &e.priority {
            PlanEntryPriority::High => PlanPriority::High,
            PlanEntryPriority::Medium => PlanPriority::Medium,
            PlanEntryPriority::Low => PlanPriority::Low,
            // `PlanEntryPriority` is `#[non_exhaustive]` — the wildcard is required
            // for forward-compat with future variants added upstream.
            #[allow(unreachable_patterns)]
            _ => PlanPriority::Medium,
        };
        let status = match &e.status {
            PlanEntryStatus::Pending => PlanStatus::Pending,
            PlanEntryStatus::InProgress => PlanStatus::InProgress,
            PlanEntryStatus::Completed => PlanStatus::Completed,
            // `PlanEntryStatus` is `#[non_exhaustive]` — the wildcard is required
            // for forward-compat with future variants added upstream.
            #[allow(unreachable_patterns)]
            _ => PlanStatus::Pending,
        };
        PlanEntryView {
            content: e.content.clone(),
            priority,
            status,
        }
    }
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

/// A slash command the agent advertises via `AvailableCommandsUpdate`
/// (mirror of the protocol's `AvailableCommand`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlashCommand {
    pub name: String,
    pub description: String,
    pub input: SlashCommandInput,
}

/// Whether a command takes an argument. Mirrors the protocol's
/// `Option<AvailableCommandInput>` as an explicit enum so the no-arg case is
/// not a `None` sentinel (makes the invalid state unrepresentable).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlashCommandInput {
    /// Command takes no argument — sending `/name` is complete.
    NoInput,
    /// Command takes free-text after the name; `hint` is shown as guidance.
    FreeText { hint: String },
}

impl From<&agent_client_protocol::schema::v1::AvailableCommand> for SlashCommand {
    fn from(c: &agent_client_protocol::schema::v1::AvailableCommand) -> Self {
        use agent_client_protocol::schema::v1::AvailableCommandInput;
        let input = match &c.input {
            None => SlashCommandInput::NoInput,
            Some(AvailableCommandInput::Unstructured(u)) => SlashCommandInput::FreeText {
                hint: u.hint.clone(),
            },
            // Forward-compat fallback: unknown future variants map to NoInput so
            // the host is not broken by a protocol extension it doesn't know about.
            #[allow(unreachable_patterns)]
            Some(_) => SlashCommandInput::NoInput,
        };
        SlashCommand {
            name: c.name.clone(),
            description: c.description.clone(),
            input,
        }
    }
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

/// Semantic category of a session config option (mirror of the protocol's
/// `SessionConfigOptionCategory`). Drives which input chip renders the option;
/// the host groups options by this rather than by id. `Mode` is also surfaced
/// via [`ModeStateView`] (the adapter advertises mode on both paths), so the
/// host renders mode through the existing mode chip and uses config options for
/// `Model` / `ThoughtLevel` / `ModelConfig` / `Other`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigOptionCategoryView {
    /// Permission / session mode selector.
    Mode,
    /// Model selector.
    Model,
    /// Model-related parameter (e.g. context window).
    ModelConfig,
    /// Reasoning / thinking effort level.
    ThoughtLevel,
    /// Uncategorized, custom (`_`-prefixed), or a category added by a newer
    /// protocol than this build knows — rendered generically.
    Other,
}

/// One selectable value of a config option (mirror of the protocol's
/// `SessionConfigSelectOption`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigChoiceView {
    /// Stable id submitted via `set_config_option` to pick this value.
    pub value: String,
    /// Human-readable label shown in the dropdown.
    pub name: String,
    /// Optional longer description, shown as secondary text / tooltip.
    pub description: Option<String>,
}

/// An advertised session config option of the `select` kind (mirror of the
/// protocol's `SessionConfigOption`). Advertised at connect time in
/// `NewSessionResponse.config_options` and replaced wholesale by
/// `SetSessionConfigOptionResponse`. Non-select kinds (e.g. boolean) are dropped
/// at the mapping boundary, so this type always describes a dropdown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigOptionView {
    /// Stable identifier used to set this option via `set_config_option`.
    pub id: String,
    /// Human-readable label (e.g. "Model", "Effort").
    pub name: String,
    /// Optional longer description, shown in a tooltip.
    pub description: Option<String>,
    /// Semantic category, used to route the option to the right chip.
    pub category: ConfigOptionCategoryView,
    /// `value` of the currently-selected choice.
    pub current_value: String,
    /// The selectable choices, flattened across any protocol grouping.
    pub options: Vec<ConfigChoiceView>,
}

impl ConfigOptionView {
    /// Map a protocol `SessionConfigOption` to a view, or `None` when it is not
    /// a select-kind option daruda renders as a dropdown (boolean / future
    /// kinds are skipped — forward-compatible with protocol extensions).
    pub fn from_protocol(
        o: &agent_client_protocol::schema::v1::SessionConfigOption,
    ) -> Option<Self> {
        use agent_client_protocol::schema::v1::{
            SessionConfigKind, SessionConfigOptionCategory, SessionConfigSelectOptions,
        };
        let select = match &o.kind {
            SessionConfigKind::Select(s) => s,
            // Boolean (unstable) and any future non-select kind aren't
            // dropdowns — skip. `SessionConfigKind` is `#[non_exhaustive]`.
            #[allow(unreachable_patterns)]
            _ => return None,
        };
        let options = match &select.options {
            SessionConfigSelectOptions::Ungrouped(opts) => opts.iter().map(config_choice).collect(),
            // Grouped options are flattened (daruda's chip is a flat dropdown);
            // group headers are dropped. The current adapter sends ungrouped.
            SessionConfigSelectOptions::Grouped(groups) => groups
                .iter()
                .flat_map(|g| g.options.iter())
                .map(config_choice)
                .collect(),
            // `SessionConfigSelectOptions` is `#[non_exhaustive]`.
            #[allow(unreachable_patterns)]
            _ => Vec::new(),
        };
        let category = match &o.category {
            Some(SessionConfigOptionCategory::Mode) => ConfigOptionCategoryView::Mode,
            Some(SessionConfigOptionCategory::Model) => ConfigOptionCategoryView::Model,
            Some(SessionConfigOptionCategory::ModelConfig) => ConfigOptionCategoryView::ModelConfig,
            Some(SessionConfigOptionCategory::ThoughtLevel) => {
                ConfigOptionCategoryView::ThoughtLevel
            }
            // None, Other(_), or a future category → render generically.
            _ => ConfigOptionCategoryView::Other,
        };
        Some(ConfigOptionView {
            id: o.id.to_string(),
            name: o.name.clone(),
            description: o.description.clone(),
            category,
            current_value: select.current_value.to_string(),
            options,
        })
    }
}

fn config_choice(
    o: &agent_client_protocol::schema::v1::SessionConfigSelectOption,
) -> ConfigChoiceView {
    ConfigChoiceView {
        value: o.value.to_string(),
        name: o.name.clone(),
        description: o.description.clone(),
    }
}

#[cfg(test)]
mod tests {
    use agent_client_protocol::schema::v1::{
        AvailableCommand, PlanEntry, PlanEntryPriority, PlanEntryStatus, SessionMode,
        SessionModeState, UnstructuredCommandInput,
    };

    use super::*;

    #[test]
    fn plan_entry_view_maps_all_statuses_and_a_priority() {
        let pending = PlanEntry::new("task A", PlanEntryPriority::High, PlanEntryStatus::Pending);
        let view = PlanEntryView::from(&pending);
        assert_eq!(view.content, "task A");
        assert_eq!(view.priority, PlanPriority::High);
        assert_eq!(view.status, PlanStatus::Pending);

        let in_progress = PlanEntry::new(
            "task B",
            PlanEntryPriority::Medium,
            PlanEntryStatus::InProgress,
        );
        let view = PlanEntryView::from(&in_progress);
        assert_eq!(view.content, "task B");
        assert_eq!(view.priority, PlanPriority::Medium);
        assert_eq!(view.status, PlanStatus::InProgress);

        let completed =
            PlanEntry::new("task C", PlanEntryPriority::Low, PlanEntryStatus::Completed);
        let view = PlanEntryView::from(&completed);
        assert_eq!(view.content, "task C");
        assert_eq!(view.priority, PlanPriority::Low);
        assert_eq!(view.status, PlanStatus::Completed);
    }

    #[test]
    fn slash_command_from_no_input() {
        let cmd = AvailableCommand::new("compact", "Compact conversation history");
        let view = SlashCommand::from(&cmd);
        assert_eq!(view.name, "compact");
        assert_eq!(view.description, "Compact conversation history");
        assert_eq!(view.input, SlashCommandInput::NoInput);
    }

    #[test]
    fn slash_command_from_unstructured_input() {
        use agent_client_protocol::schema::v1::AvailableCommandInput;
        let cmd = AvailableCommand::new("search", "Search the codebase").input(
            AvailableCommandInput::Unstructured(UnstructuredCommandInput::new("search query")),
        );
        let view = SlashCommand::from(&cmd);
        assert_eq!(view.name, "search");
        assert_eq!(view.description, "Search the codebase");
        assert_eq!(
            view.input,
            SlashCommandInput::FreeText {
                hint: "search query".to_string()
            }
        );
    }

    #[test]
    fn slash_command_free_text_preserves_name_description_and_hint() {
        // Verifies that a FreeText command round-trips all three fields
        // (name, description, hint) together — not covered by the NoInput
        // or unstructured-input tests which check fields in isolation.
        use agent_client_protocol::schema::v1::AvailableCommandInput;
        let cmd = AvailableCommand::new("explain", "Explain selected code").input(
            AvailableCommandInput::Unstructured(UnstructuredCommandInput::new("what to explain")),
        );
        let view = SlashCommand::from(&cmd);
        assert_eq!(view.name, "explain");
        assert_eq!(view.description, "Explain selected code");
        assert_eq!(
            view.input,
            SlashCommandInput::FreeText {
                hint: "what to explain".to_string()
            }
        );
    }

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

    #[test]
    fn config_option_select_maps_id_value_and_choices() {
        use agent_client_protocol::schema::v1::{
            SessionConfigOption, SessionConfigOptionCategory, SessionConfigSelectOption,
        };
        let opt = SessionConfigOption::select(
            "model",
            "Model",
            "default",
            vec![
                SessionConfigSelectOption::new("default", "Default (recommended)"),
                SessionConfigSelectOption::new("sonnet", "Sonnet")
                    .description("Efficient for routine tasks"),
            ],
        )
        .category(SessionConfigOptionCategory::Model);

        let view = ConfigOptionView::from_protocol(&opt).expect("select option maps");
        assert_eq!(view.id, "model");
        assert_eq!(view.name, "Model");
        assert_eq!(view.category, ConfigOptionCategoryView::Model);
        assert_eq!(view.current_value, "default");
        assert_eq!(view.options.len(), 2);
        assert_eq!(view.options[0].value, "default");
        assert_eq!(view.options[1].value, "sonnet");
        assert_eq!(
            view.options[1].description.as_deref(),
            Some("Efficient for routine tasks")
        );
    }

    #[test]
    fn config_option_thought_level_category_maps() {
        use agent_client_protocol::schema::v1::{
            SessionConfigOption, SessionConfigOptionCategory, SessionConfigSelectOption,
        };
        let opt = SessionConfigOption::select(
            "effort",
            "Effort",
            "high",
            vec![SessionConfigSelectOption::new("high", "High")],
        )
        .category(SessionConfigOptionCategory::ThoughtLevel);
        let view = ConfigOptionView::from_protocol(&opt).expect("maps");
        assert_eq!(view.category, ConfigOptionCategoryView::ThoughtLevel);
    }

    #[test]
    fn config_option_without_category_is_other() {
        use agent_client_protocol::schema::v1::{SessionConfigOption, SessionConfigSelectOption};
        let opt = SessionConfigOption::select(
            "agent",
            "Agent",
            "default",
            vec![SessionConfigSelectOption::new("default", "Default")],
        );
        let view = ConfigOptionView::from_protocol(&opt).expect("maps");
        assert_eq!(view.category, ConfigOptionCategoryView::Other);
    }
}
