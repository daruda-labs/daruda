//! The `--screenshot-scenario` CLI names and their mapping to a
//! [`ScreenshotScenario`].

use super::super::main_area::agent_chat_host::agent_chat_ops::ThoughtsShot;
use super::super::pages::tasks::editor::TaskEditorShot;
use super::ScreenshotScenario;
use daruda_chat::view::ActivityOptionsTab;
use daruda_config::BuiltinSection;

/// CLI token for the command-palette scenario.
const NAME_COMMAND_PALETTE: &str = "command-palette";
/// CLI token for the Lane switcher, seeded with a long clipped label.
const NAME_LANE_SWITCHER: &str = "lane-switcher";
/// CLI token for the error-report-modal scenario.
const NAME_ERROR_MODAL: &str = "error-modal";
/// CLI token for the error-toast scenario.
const NAME_TOAST: &str = "toast";
const NAME_SESSION_NOTICES: &str = "session-notices";
const NAME_SCRATCH_TAB: &str = "scratch-tab";
/// CLI token for the Landing view — a workspace emptied of its projects.
const NAME_LANDING: &str = "landing";
/// CLI token for the Settings-window scenario. Bare opens the default section;
/// `settings:<slug>` opens a specific section (e.g. `settings:font`).
const NAME_SETTINGS: &str = "settings";
/// CLI token for the Settings window showing a failed action's banner.
const NAME_LEFT_DOCK_GIT: &str = "left-dock-git";
const NAME_SETTINGS_ERROR: &str = "settings-error";
/// CLI token for Settings with a search query typed: the results page.
const NAME_SETTINGS_SEARCH: &str = "settings-search";
/// CLI token for the agent catalog with one card fully open.
const NAME_AGENT_CATALOG_EXPANDED: &str = "agent-catalog-expanded";
/// CLI token for a catalog row with one transcript editor open. Bare opens the
/// fold editor; `agent-catalog-editor:<axis>` names `fold`, `filter` or `range`.
const NAME_AGENT_CATALOG_EDITOR: &str = "agent-catalog-editor";
/// CLI token for the app-drawn window chrome. Forces the Client arm on a host
/// that would resolve to Native, so the layout is reviewable off its platform.
const NAME_CLIENT_CHROME: &str = "client-chrome";
/// CLI token for the pane context-menu scenario.
const NAME_PANE_CONTEXT_MENU: &str = "pane-context-menu";
/// CLI token for the mermaid-diagram lightbox scenario.
const NAME_MERMAID_LIGHTBOX: &str = "mermaid-lightbox";
/// CLI token for the Flows page's graph scenario.
const NAME_FLOW_GRAPH: &str = "flow-graph";
/// CLI token for the Flows page's graph with a run colouring it.
const NAME_FLOW_GRAPH_RUNNING: &str = "flow-graph-running";
/// CLI token for the Flows page's graph with a node selected and its inspector up.
const NAME_FLOW_GRAPH_FORM: &str = "flow-graph-form";
/// CLI token for the inspector showing why a save was refused.
const NAME_FLOW_GRAPH_FORM_REFUSED: &str = "flow-graph-form-refused";
const NAME_FLOW_GRAPH_PINNED: &str = "flow-graph-pinned";
/// CLI token for the card affordances that only exist while authoring: an
/// issue count, a failure policy, a dropped pin's reason, inherited defaults.
const NAME_FLOW_GRAPH_AUTHORING: &str = "flow-graph-authoring";
/// CLI token for a past run's report as the Flows page's detail.
const NAME_FLOW_RUN_DETAIL: &str = "flow-run-detail";
const NAME_AGENT_CHAT_FAILURE: &str = "agent-chat-failure";
const NAME_AGENT_CHAT_TRANSPORT_CLOSED: &str = "agent-chat-transport-closed";
const NAME_AGENT_CHAT_PACKAGE_NETWORK: &str = "agent-chat-package-network";
const NAME_AGENT_CHAT_PACKAGE_INVALID: &str = "agent-chat-package-invalid";
/// CLI token for an empty agent-chat pane with its view options open.
const NAME_AGENT_CHAT_EMPTY: &str = "agent-chat-empty";
/// CLI token for the settled-transcript scenario.
const NAME_AGENT_CHAT: &str = "agent-chat";
const NAME_AGENT_CHAT_COMPACTION: &str = "agent-chat-compaction";
const NAME_AGENT_CHAT_ATTACHMENTS: &str = "agent-chat-attachments";
const NAME_ORCHESTRATOR_CHIP: &str = "orchestrator-chip";
const NAME_ORCHESTRATOR_TAB: &str = "orchestrator-tab";
const NAME_LEFT_DOCK_SESSIONS_SIX: &str = "left-dock-sessions-six";
const NAME_LEFT_DOCK_SESSIONS: &str = "left-dock-sessions";
const NAME_LEFT_DOCK_GROUPS: &str = "left-dock-groups";
const NAME_LEFT_DOCK_GROUP_STATUS: &str = "left-dock-group-status";
const NAME_LEFT_DOCK_GROUP_HOVER: &str = "left-dock-group-hover";
const NAME_LEFT_DOCK_PROJECT_ALIAS: &str = "left-dock-project-alias";
const NAME_LEFT_DOCK_WORKTREES: &str = "left-dock-worktrees";
const NAME_LEFT_DOCK_RAIL: &str = "left-dock-rail";
const NAME_LEFT_DOCK_EXPANDED: &str = "left-dock-expanded";
const NAME_TAB_INDICATORS: &str = "tab-indicators";
/// CLI token for the mid-turn transcript, before the agent writes its answer.
const NAME_AGENT_CHAT_WORKING: &str = "agent-chat-working";
/// CLI token for the same transcript with the filter and tail chips engaged.
const NAME_AGENT_CHAT_NARROWED: &str = "agent-chat-narrowed";
/// CLI token for the transcript with the custom fold editor open.
const NAME_AGENT_CHAT_FOLD: &str = "agent-chat-fold";
const NAME_AGENT_CHAT_INTERRUPTED: &str = "agent-chat-interrupted";
/// CLI token for a turn stopped to avoid extra-usage billing, banner raised.
const NAME_AGENT_CHAT_OVERAGE_STOPPED: &str = "agent-chat-overage-stopped";
/// CLI tokens for a CLI task's read-only transcript, running and exited.
const NAME_AGENT_CHAT_CLI_RUNNING: &str = "agent-chat-cli-running";
const NAME_AGENT_CHAT_CLI_ENDED: &str = "agent-chat-cli-ended";
/// CLI token for the queued-prompt strip holding a queue a Stop parked.
const NAME_AGENT_CHAT_QUEUE_PARKED: &str = "agent-chat-queue-parked";
/// CLI token for the same strip once an empty Enter armed the resume gesture.
const NAME_AGENT_CHAT_QUEUE_ARMED: &str = "agent-chat-queue-armed";
/// CLI token for a transcript of prose-only replies and their own folds.
const NAME_AGENT_CHAT_SOLE_REPLY: &str = "agent-chat-sole-reply";
const NAME_AGENT_CHAT_PLAN: &str = "agent-chat-plan";
const NAME_AGENT_CHAT_RUNNING_TOOL: &str = "agent-chat-running-tool";
const NAME_AGENT_CHAT_PLAN_STOPPED: &str = "agent-chat-plan-stopped";
/// CLI tokens for a stretch of work narrated thought by thought — settled, mid-turn
/// on a thought with no call after it yet, and filtered down to its thoughts.
const NAME_AGENT_CHAT_THOUGHTS: &str = "agent-chat-thoughts";
const NAME_AGENT_CHAT_THOUGHTS_WORKING: &str = "agent-chat-thoughts-working";
const NAME_AGENT_CHAT_THOUGHTS_ONLY: &str = "agent-chat-thoughts-only";
/// CLI token for the tail window's boundary row, closed.
const NAME_AGENT_CHAT_TAIL: &str = "agent-chat-tail";
/// CLI token for the same boundary row, open.
const NAME_AGENT_CHAT_TAIL_OPEN: &str = "agent-chat-tail-open";
/// CLI token for a tool group's own boundary row, closed.
const NAME_AGENT_CHAT_GROUP_TAIL: &str = "agent-chat-group-tail";
/// CLI token for the same in-group boundary row, open.
const NAME_AGENT_CHAT_GROUP_TAIL_OPEN: &str = "agent-chat-group-tail-open";
/// CLI token for a subagent card's own boundary, closed.
const NAME_AGENT_CHAT_SUBAGENT_TAIL: &str = "agent-chat-subagent-tail";
/// CLI token for the same in-card boundary, open.
const NAME_AGENT_CHAT_SUBAGENT_TAIL_OPEN: &str = "agent-chat-subagent-tail-open";
/// CLI token prefix for the compact bar's combined options popover, suffixed
/// with an [`ActivityOptionsTab`] token (`agent-chat-options:filter`). Bare
/// `agent-chat-options` opens the Fold tab.
const NAME_AGENT_CHAT_OPTIONS: &str = "agent-chat-options";
/// CLI token for the flow delete confirmation, on the repository's copy.
const NAME_FLOW_DELETE_CONFIRM: &str = "flow-delete-confirm";

const NAME_FLOW_PICKER: &str = "flow-picker";
const NAME_FLOW_PROFILE_PICKER: &str = "flow-profile-picker";
const NAME_FLOW_RESUMABLE: &str = "flow-resumable";
const NAME_FLOW_RUNNING: &str = "flow-running";
const NAME_FLOW_ASKING: &str = "flow-asking";

impl ScreenshotScenario {
    /// Map a CLI token to a scenario. Unknown tokens return `None`.
    /// `settings:<slug>` selects a Settings section; bare `settings` uses the
    /// default section.
    pub(crate) fn from_cli_name(name: &str) -> Option<Self> {
        use super::super::right_dock::quick_entry_shot::QuickEntryShot;
        match name {
            "agent-chat-diff" => Some(Self::AgentChatDiff),
            "status-ports" => Some(Self::StatusBar(
                super::super::status_bar::screenshot::StatusBarShot::Ports,
            )),
            "status-flows" => Some(Self::StatusBar(
                super::super::status_bar::screenshot::StatusBarShot::Flows,
            )),
            "keep-awake" => Some(Self::StatusBar(
                super::super::status_bar::screenshot::StatusBarShot::KeepAwake,
            )),
            "keep-awake-failed" => Some(Self::StatusBar(
                super::super::status_bar::screenshot::StatusBarShot::KeepAwakeFailed,
            )),
            "status-usage" => Some(Self::StatusBar(
                super::super::status_bar::screenshot::StatusBarShot::Usage,
            )),
            "status-account" => Some(Self::StatusBar(
                super::super::status_bar::screenshot::StatusBarShot::Account,
            )),
            "group-create" => Some(Self::QuickEntry(QuickEntryShot::GroupCreate)),
            "worktree-create" => Some(Self::QuickEntry(QuickEntryShot::WorktreeCreate)),
            "mcp-paste" => Some(Self::QuickEntry(QuickEntryShot::McpPaste)),
            "mcp-review" => Some(Self::QuickEntry(QuickEntryShot::McpReview)),
            "mcp-stdio" => Some(Self::QuickEntry(QuickEntryShot::McpStdio)),
            "mcp-saved" => Some(Self::QuickEntry(QuickEntryShot::McpSaved)),
            "mcp-error" => Some(Self::QuickEntry(QuickEntryShot::McpError)),
            "mcp-codex-tail" => Some(Self::QuickEntry(QuickEntryShot::McpCodexTail)),
            "mcp-codex-review" => Some(Self::QuickEntry(QuickEntryShot::McpCodexReview)),
            "library-claude" => Some(Self::QuickEntry(QuickEntryShot::LibraryClaude)),
            "library-codex" => Some(Self::QuickEntry(QuickEntryShot::LibraryCodex)),
            "skill-paste" => Some(Self::QuickEntry(QuickEntryShot::SkillPaste)),
            "skill-review" => Some(Self::QuickEntry(QuickEntryShot::SkillReview)),
            "skill-codex-review" => Some(Self::QuickEntry(QuickEntryShot::SkillCodexReview)),
            "library-delete" => Some(Self::QuickEntry(QuickEntryShot::DeleteConfirm)),
            "flow-create" => Some(Self::FlowCreate),
            "flows" => Some(Self::FlowBrowser(
                super::super::flow_browser::screenshot::FlowBrowserShot::Definitions,
            )),
            "flows-grouped" => Some(Self::FlowBrowser(
                super::super::flow_browser::screenshot::FlowBrowserShot::Grouped,
            )),
            "flows-empty" => Some(Self::FlowBrowser(
                super::super::flow_browser::screenshot::FlowBrowserShot::Empty,
            )),
            "flow-runs" => Some(Self::FlowBrowser(
                super::super::flow_browser::screenshot::FlowBrowserShot::Runs,
            )),
            "flow-requests" => Some(Self::FlowBrowser(
                super::super::flow_browser::screenshot::FlowBrowserShot::Asking,
            )),
            "tasks" => Some(Self::Tasks),
            "tasks-all" => Some(Self::TasksAll),
            "tasks-no-results" => Some(Self::TasksNoResults),
            "tasks-by-status" => Some(Self::TasksByStatus),
            "tasks-by-project" => Some(Self::TasksByProject),
            "task-saved" => Some(Self::TaskSaved),
            "task-editor" => Some(Self::TaskEditor(TaskEditorShot::Form)),
            "task-editor-preview" => Some(Self::TaskEditor(TaskEditorShot::Preview)),
            "task-editor-saving" => Some(Self::TaskEditor(TaskEditorShot::Saving)),
            "task-editor-running" => Some(Self::TaskEditor(TaskEditorShot::Running)),
            "task-editor-disk-copy" => Some(Self::TaskEditor(TaskEditorShot::DiskCopy)),
            NAME_ORCHESTRATOR_CHIP => Some(Self::OrchestratorChip),
            NAME_ORCHESTRATOR_TAB => Some(Self::OrchestratorTab),
            NAME_LEFT_DOCK_SESSIONS_SIX => Some(Self::LeftDockSessionsSix),
            NAME_LEFT_DOCK_SESSIONS => Some(Self::LeftDockSessions),
            NAME_LEFT_DOCK_GROUPS => Some(Self::LeftDockGroups),
            NAME_LEFT_DOCK_GROUP_STATUS => Some(Self::LeftDockGroupStatus),
            NAME_LEFT_DOCK_GROUP_HOVER => Some(Self::LeftDockGroupHover),
            NAME_LEFT_DOCK_PROJECT_ALIAS => Some(Self::LeftDockProjectAlias),
            NAME_LEFT_DOCK_GIT => Some(Self::LeftDockGit),
            NAME_LEFT_DOCK_WORKTREES => Some(Self::LeftDockWorktrees),
            NAME_LEFT_DOCK_RAIL => Some(Self::LeftDockRail),
            NAME_LEFT_DOCK_EXPANDED => Some(Self::LeftDockExpanded),
            NAME_TAB_INDICATORS => Some(Self::TabIndicators),
            NAME_COMMAND_PALETTE => Some(Self::CommandPalette),
            NAME_LANE_SWITCHER => Some(Self::LaneSwitcher),
            NAME_CLIENT_CHROME => Some(Self::ClientChrome),
            NAME_ERROR_MODAL => Some(Self::ErrorModal),
            NAME_TOAST => Some(Self::Toast),
            NAME_SESSION_NOTICES => Some(Self::SessionNotices),
            NAME_SCRATCH_TAB => Some(Self::ScratchTab),
            NAME_LANDING => Some(Self::Landing),
            NAME_SETTINGS => Some(Self::Settings(BuiltinSection::default())),
            NAME_SETTINGS_ERROR => Some(Self::SettingsError),
            NAME_SETTINGS_SEARCH => Some(Self::SettingsSearch),
            NAME_AGENT_CATALOG_EXPANDED => Some(Self::AgentCatalogExpanded),
            NAME_AGENT_CATALOG_EDITOR => {
                Some(Self::AgentCatalogEditor(daruda_settings::EditorShot::Fold))
            }
            NAME_PANE_CONTEXT_MENU => Some(Self::PaneContextMenu),
            NAME_MERMAID_LIGHTBOX => Some(Self::MermaidLightbox),
            NAME_FLOW_GRAPH => Some(Self::FlowGraph),
            NAME_FLOW_GRAPH_RUNNING => Some(Self::FlowGraphRunning),
            NAME_FLOW_GRAPH_FORM => Some(Self::FlowGraphForm),
            NAME_FLOW_GRAPH_FORM_REFUSED => Some(Self::FlowGraphFormRefused),
            NAME_FLOW_GRAPH_PINNED => Some(Self::FlowGraphPinned),
            NAME_FLOW_GRAPH_AUTHORING => Some(Self::FlowGraphAuthoring),
            NAME_FLOW_RUN_DETAIL => Some(Self::FlowRunDetail),
            NAME_AGENT_CHAT_FAILURE => Some(Self::AgentChatFailure),
            NAME_AGENT_CHAT_TRANSPORT_CLOSED => Some(Self::AgentChatTransportClosed),
            NAME_AGENT_CHAT_PACKAGE_NETWORK => Some(Self::AgentChatPreparationFailure(
                daruda_acp::preparation::PreparationKind::Network,
            )),
            NAME_AGENT_CHAT_PACKAGE_INVALID => Some(Self::AgentChatPreparationFailure(
                daruda_acp::preparation::PreparationKind::InvalidPackage,
            )),
            NAME_AGENT_CHAT_EMPTY => Some(Self::AgentChatEmpty),
            NAME_AGENT_CHAT => Some(Self::AgentChat),
            NAME_AGENT_CHAT_COMPACTION => Some(Self::AgentChatCompaction),
            NAME_AGENT_CHAT_ATTACHMENTS => Some(Self::AgentChatAttachments),
            NAME_AGENT_CHAT_WORKING => Some(Self::AgentChatWorking),
            NAME_AGENT_CHAT_NARROWED => Some(Self::AgentChatNarrowed),
            NAME_AGENT_CHAT_FOLD => Some(Self::AgentChatFold),
            NAME_AGENT_CHAT_INTERRUPTED => Some(Self::AgentChatInterrupted),
            NAME_AGENT_CHAT_OVERAGE_STOPPED => Some(Self::AgentChatOverageStopped),
            NAME_AGENT_CHAT_CLI_RUNNING => Some(Self::AgentChatCliRunning),
            NAME_AGENT_CHAT_CLI_ENDED => Some(Self::AgentChatCliEnded),
            NAME_AGENT_CHAT_QUEUE_PARKED => Some(Self::AgentChatQueueParked),
            NAME_AGENT_CHAT_QUEUE_ARMED => Some(Self::AgentChatQueueArmed),
            NAME_AGENT_CHAT_SOLE_REPLY => Some(Self::AgentChatSoleReply),
            NAME_AGENT_CHAT_PLAN => Some(Self::AgentChatPlan),
            NAME_AGENT_CHAT_RUNNING_TOOL => Some(Self::AgentChatRunningTool),
            NAME_AGENT_CHAT_PLAN_STOPPED => Some(Self::AgentChatPlanStopped),
            NAME_AGENT_CHAT_TAIL => Some(Self::AgentChatTail),
            NAME_AGENT_CHAT_TAIL_OPEN => Some(Self::AgentChatTailOpen),
            NAME_AGENT_CHAT_GROUP_TAIL => Some(Self::AgentChatGroupTail),
            NAME_AGENT_CHAT_GROUP_TAIL_OPEN => Some(Self::AgentChatGroupTailOpen),
            NAME_AGENT_CHAT_SUBAGENT_TAIL => Some(Self::AgentChatSubagentTail),
            NAME_AGENT_CHAT_SUBAGENT_TAIL_OPEN => Some(Self::AgentChatSubagentTailOpen),
            NAME_AGENT_CHAT_OPTIONS => Some(Self::AgentChatOptions(ActivityOptionsTab::Fold)),
            NAME_AGENT_CHAT_THOUGHTS => Some(Self::AgentChatThoughts(ThoughtsShot::Settled)),
            NAME_AGENT_CHAT_THOUGHTS_WORKING => {
                Some(Self::AgentChatThoughts(ThoughtsShot::Working))
            }
            NAME_AGENT_CHAT_THOUGHTS_ONLY => {
                Some(Self::AgentChatThoughts(ThoughtsShot::ThoughtsOnly))
            }
            NAME_FLOW_PICKER => Some(Self::FlowPicker),
            NAME_FLOW_PROFILE_PICKER => Some(Self::FlowProfilePicker),
            NAME_FLOW_RESUMABLE => Some(Self::FlowResumable),
            NAME_FLOW_RUNNING => Some(Self::FlowRunning),
            NAME_FLOW_ASKING => Some(Self::FlowAsking),
            NAME_FLOW_DELETE_CONFIRM => Some(Self::FlowDeleteConfirm),
            _ => name
                .strip_prefix(NAME_AGENT_CHAT_OPTIONS)
                .and_then(|rest| rest.strip_prefix(':'))
                .and_then(ActivityOptionsTab::from_token)
                .map(Self::AgentChatOptions)
                .or_else(|| {
                    name.strip_prefix(NAME_AGENT_CATALOG_EDITOR)
                        .and_then(|rest| rest.strip_prefix(':'))
                        .and_then(daruda_settings::EditorShot::from_token)
                        .map(Self::AgentCatalogEditor)
                })
                .or_else(|| Self::settings_section_from_cli_name(name)),
        }
    }

    fn settings_section_from_cli_name(name: &str) -> Option<Self> {
        name.strip_prefix(concat!("settings", ":"))
            .and_then(BuiltinSection::from_slug)
            .map(Self::Settings)
    }
}
