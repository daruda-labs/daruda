//! The settings surface for common daruda config options.
//!
//! Hosted by a `Workspace` in place of its body, one per window; re-opening an
//! already-open one routes through [`SettingsView::focus_section`] instead of
//! building a second. Two windows can therefore show this at once:
//! [`SettingsView::sync_external_settings`] keeps them coherent by adopting an
//! outside change only when there is no local draft, so a view holding one
//! shows a stale value until that draft clears — and a stale re-commit is
//! refused as a conflict rather than overwriting. Builtin sections pair a `BuiltinSection` variant with
//! nav/header strings and a `render_<section>` method. The view draws no
//! chrome of its own — the host window owns the title bar — and asks to be
//! dismissed by emitting [`SettingsEvent::Close`] rather than acting on the
//! window itself.

mod bindings;
mod catalog;
mod confirm;
mod copy;
mod hosts;
mod initialize;
mod layout;
mod navigation;
mod page;
mod persistence;
mod presentation;
mod render;
mod reset;
mod search;
mod sections;
mod spec;

#[cfg(test)]
mod tests;

use catalog::{AgentCatalogRow, CardFold};
use hosts::{SessionHostRow, reconcile_session_host_tombstones};
use std::collections::{HashMap, HashSet};

use crate::ui::theme;
use daruda_config::BuiltinSection;
use gpui::{
    Context, Entity, EventEmitter, FocusHandle, Focusable as _, IntoElement, SharedString,
    Subscription, Task, Window, div, prelude::*, px,
};

use crate::lane::session_host;
use crate::surface::strings as s;
use crate::transcript::display_filter::DisplayFilter;
use crate::transcript::editor::state::{FilterEditorState, FoldEditorState};
use crate::transcript::fold_mode::FoldMode;
use crate::ui::select::{self, SelectOption, SelectState};
use crate::ui::{InputEvent, InputState};
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;

fn settings_button(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
) -> crate::ui::Button {
    crate::ui::button(id, label).tab_stop(true)
}

fn settings_button_danger(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
) -> crate::ui::Button {
    crate::ui::button_danger(id, label).tab_stop(true)
}

/// "Copied!" feedback for one copy-to-clipboard button: whether the label is
/// currently swapped, and the timer that swaps it back.
///
/// One type rather than a `bool` + `Option<Task<()>>` pair per button — the
/// two fields are always set together, and there are now two buttons that want
/// exactly this behaviour.
#[derive(Default)]
pub(super) struct CopyFeedback {
    copied: bool,
    /// Re-created on every click, so a rapid second click restarts the window
    /// rather than inheriting the first one's remaining time. Dropping the
    /// previous `Task` cancels it.
    _revert: Option<Task<()>>,
}

impl CopyFeedback {
    /// Whether to show the confirmation label instead of "Copy".
    pub(super) fn copied(&self) -> bool {
        self.copied
    }
}

/// What the view asks of whichever `Workspace` is showing it. Kept as an
/// event rather than a handle so this module never points back at
/// `crate::workspace` — the dependency runs one way.
pub enum SettingsEvent {
    /// The user is done: Escape, or the sidebar's back button.
    Close,
    /// A login to run. The machinery lives on `Workspace`, beside the panes
    /// the resulting credentials are for, and it owns the process handle and
    /// the Cancel that goes with it.
    Login(LoginRequest),
    /// Open the active project's config file, where `[shell]` can be
    /// overridden per project.
    OpenProjectConfig,
}

/// Which login an account row asked for.
pub enum LoginRequest {
    /// Add a managed account under `recipe`. The login command comes from the
    /// host's agent catalog, which is why the host resolves it.
    AddAccount(daruda_store::accounts::AccountRecipeId),
    /// Re-run the login for an account that already exists.
    Reauthenticate(daruda_store::accounts::AccountId),
    /// Re-run the login for `recipe`'s ambient home — the credentials a pane
    /// with no managed account uses, which has no account id to name.
    ReauthenticateSystem(daruda_store::accounts::AccountRecipeId),
}

impl EventEmitter<SettingsEvent> for SettingsView {}

#[cfg(feature = "screenshot")]
pub(crate) use sections::agent_transcript::editor::EditorShot;

pub struct SettingsView {
    panel_focus_handle: FocusHandle,
    /// Last config this window observed after opening or successfully writing.
    /// Used only to detect a same-field external edit before applying a draft.
    base_config: daruda_config::Config,
    /// Section currently rendered by the body.
    active_section: BuiltinSection,
    sidebar_search_input: Entity<InputState>,
    sidebar_focus_handles: HashMap<BuiltinSection, FocusHandle>,
    /// Each page's Advanced header, so Tab reaches it and Enter/Space folds it.
    advanced_focus_handles: HashMap<BuiltinSection, FocusHandle>,
    /// Whether the host window has a project, told by the host: the project
    /// config button has nothing to open without one.
    project_open: bool,
    /// Per-section input focus handles, in tab-cycle order. `focus_section`
    /// jumps to the first handle when entering a section from outside the
    /// window (sidebar click, external open); `focus_next_input` cycles the
    /// full list on Tab. Single source for both — previously a first-handle
    /// map plus a separately hand-maintained match in `focus_next_input`,
    /// which could drift out of sync when a field was added.
    section_focus_targets: HashMap<BuiltinSection, Vec<FocusHandle>>,
    // ---- form fields ----
    // General page.
    language_select: Entity<SelectState>,
    // Theme (rendered inside Appearance & Window).
    // `terminal_preset_select` controls the cell palette (16-color
    // ANSI + fg/bg); `ui_preset_select` controls the chrome palette
    // (workspace, sidebar, modal, status bar). The two axes are
    // independent — see `daruda_config::ThemeConfig`.
    terminal_preset_select: Entity<SelectState>,
    ui_preset_select: Entity<SelectState>,
    // Font
    terminal_font_family_select: Entity<SelectState>,
    terminal_font_size_input: Entity<InputState>,
    terminal_line_height_input: Entity<InputState>,
    terminal_cell_width_input: Entity<InputState>,
    editor_font_family_select: Entity<SelectState>,
    editor_font_size_input: Entity<InputState>,
    editor_line_height_input: Entity<InputState>,
    agent_chat_font_family_select: Entity<SelectState>,
    agent_chat_font_size_input: Entity<InputState>,
    agent_chat_line_height_input: Entity<InputState>,
    // Cursor
    cursor_style_select: Entity<SelectState>,
    /// Whether a fresh agent-chat pane holds its prose to the reading column.
    /// On the General page beside the themes: it is a reading preference, not
    /// an agent one, and applies to every agent.
    agent_use_reading_width: bool,
    /// Whether tool-group summaries spell each category out beside its icon.
    agent_tool_summary_labels: bool,
    // Agent
    /// Filters the catalog's available and needs-install lists by name or id.
    agent_catalog_search: Entity<InputState>,
    agent_use_modifier_to_send: bool,
    /// The agent catalog in `config.toml` order, editable and non-editable
    /// entries in one list. Reach it through [`Self::agent_catalog_is_empty`] /
    /// [`Self::agent_editable_rows`] / [`Self::agent_unresolved_entries`]:
    /// holding the two kinds in separate vectors previously let each operation
    /// decide on its own whether "the catalog" included the non-editable half,
    /// and the two that answered "no" disagreed with the rest.
    agent_catalog: Vec<AgentCatalogItem>,
    /// What each agent last advertised on the mode / model axes. Mirrored from
    /// the app-wide vocabulary Global so rows update while Settings remains
    /// open. A row falls back to [`daruda_config::agent_vocabulary_seed`] per
    /// axis when this has nothing for its current id and command.
    pub(super) agent_vocabulary: daruda_store::agent_vocabulary::AgentVocabularyCache,
    /// The hosting workspace's data dir — the store `agent_vocabulary` mirrors.
    data_dir: std::path::PathBuf,
    /// The session host registry (`[[session_hosts]]`) in `config.toml`
    /// order — named, reusable SSH/Docker targets a lane's `session_host`
    /// can reference by id instead of repeating the same target/container
    /// as free text on every lane. See [`SessionHostRow`].
    session_host_rows: Vec<SessionHostRow>,
    // Accounts snapshot loaded from `accounts.json` at construction;
    // every write goes through the section's own
    // `set_default_account`/`remove_account` handlers, which persist
    // immediately and broadcast the new state to every open
    // `Workspace` window. See `sections/accounts.rs`'s module doc.
    accounts: daruda_store::accounts::AccountsState,
    account_login_busy: bool,
    /// How each set of credentials was signed in, mirrored from
    /// `auth_status_global`. A scope absent here has not been read yet, which
    /// the rows show as nothing rather than as a claim.
    auth_statuses: std::collections::HashMap<
        crate::workspace::auth_status_global::LoginTarget,
        daruda_agent::accounts::auth_status::AuthStatus,
    >,
    // Render
    max_fps_select: Entity<SelectState>,
    // Shell
    close_pane_on_exit: bool,
    // Window
    opacity_input: Entity<InputState>,
    window_blur: bool,
    // Terminal
    scrollback_input: Entity<InputState>,
    inset_x_input: Entity<InputState>,
    inset_y_input: Entity<InputState>,
    // Sidebar
    files_show_hidden: bool,
    files_use_gitignore: bool,
    update_auto_check: bool,
    claude_status_stale_input: Entity<InputState>,
    claude_status_ttl_input: Entity<InputState>,
    usage_limits_poll_input: Entity<InputState>,
    usage_status_poll_input: Entity<InputState>,
    ports_poll_input: Entity<InputState>,
    logs_retention_input: Entity<InputState>,
    logs_max_size_input: Entity<InputState>,
    presence_grace_input: Entity<InputState>,
    presence_idle_input: Entity<InputState>,
    presence_idle_foreground_input: Entity<InputState>,
    agent_input_max_rows_input: Entity<InputState>,
    agent_reading_width_input: Entity<InputState>,
    flow_timeout_minutes_input: Entity<InputState>,
    flow_max_node_runs_input: Entity<InputState>,
    flow_max_cost_input: Entity<InputState>,
    flow_cost_currency_input: Entity<InputState>,
    left_collapsed_by_default: bool,
    preview_tab: bool,
    left_default_width_input: Entity<InputState>,
    shell_program_input: Entity<InputState>,
    /// Settings pages whose Advanced card the user has opened this session.
    advanced_open: std::collections::HashSet<BuiltinSection>,
    shell_natural_text_editing: bool,
    notify_osc9: bool,
    notify_osc777: bool,
    notify_attention: bool,
    notify_long_running: bool,
    notify_skip_focused_pane: bool,
    clipboard_copy_on_select: bool,
    git_confirm_commit: bool,
    git_confirm_push: bool,
    git_default_commit_message: bool,
    notify_hook: bool,
    notify_agent_completion: bool,
    notify_agent_waiting: bool,
    telegram_only_when_away: bool,
    notify_long_running_threshold_input: Entity<InputState>,
    // File Viewer
    syntax_theme_select: Entity<SelectState>,
    // Clipboard
    clipboard_streaming_input: Entity<InputState>,
    // External Editor
    editor_select: Entity<SelectState>,
    file_icon_color_select: Entity<SelectState>,
    // Panels (bottom-dock macro grid)
    panels_grid_columns_input: Entity<InputState>,
    // Claude Status
    claude_status_enable: bool,
    // Notifications (Telegram)
    telegram_enabled: bool,
    remote_channel_settings: Entity<crate::remote_channel::settings::ChannelSettings>,
    orchestrator_enabled: bool,
    /// Empty value = follow the catalog's first entry; see
    /// `sections::orchestrator`.
    orchestrator_agent_select: Entity<SelectState>,
    /// Empty value = the system default account.
    orchestrator_account_select: Entity<SelectState>,
    telegram_token_input: Entity<InputState>,
    /// Presence-only cache of whether a token is currently stored in
    /// the Keychain — seeded once at construction, updated by the
    /// Save/Clear button handlers. Never holds the token itself.
    telegram_token_configured: bool,
    /// The paired chat, mirrored from `SettingsStore` for rendering. Pairing is
    /// written by the bridge's poll loop, not by any editor here, so it is not
    /// one of [`Self::settings_ui_patches`] and the window would otherwise
    /// never learn a phone paired while it was open. Single update site:
    /// [`Self::adopt_external_settings`]. [`Self::validate`] deliberately does
    /// not read this — it re-reads the store, which cannot lag an unflushed
    /// observer effect the way this field can.
    telegram_authorized_chat_id: Option<i64>,
    /// Transient UI-only state (never persisted): the pairing code
    /// most recently generated by "Generate Pairing Code", shown with
    /// the `/pair <code>` instructions until the window closes.
    telegram_pair_code: Option<String>,
    /// "Copied!" feedback for the `/pair <code>` button.
    telegram_pair_command_copy: CopyFeedback,
    /// The same, for the BotFather `/setcommands` block.
    telegram_botfather_copy: CopyFeedback,
    scroll_handle: gpui::ScrollHandle,
    sidebar_scroll_handle: gpui::ScrollHandle,
    _input_subscriptions: Vec<Subscription>,
    error: Option<SharedString>,
    conflict: Option<daruda_config::SettingsPatch>,
    /// Plugin ids (`<plugin>@<marketplace>`) with an `install` /
    /// `uninstall` CLI invocation currently spawned on the
    /// `background_executor`. Used by the Plugin section to show a
    /// transient `Installing…` / `Uninstalling…` label and to swallow
    /// duplicate clicks while the request is in flight.
    pub(super) plugin_ops_in_flight: std::collections::HashSet<String>,
    /// Installed-plugin manifest snapshot. Refreshed when `SkillsState`
    /// changes so rendering the Plugin section never performs file I/O.
    pub(super) plugin_installs:
        std::collections::BTreeMap<String, crate::agent::skills::plugins::PluginInstall>,
    /// `<plugin>@<marketplace>` of the plugin whose detail pane is on
    /// the right side of the master-detail layout. `None` shows the
    /// "select a plugin" placeholder.
    pub(super) plugin_selected: Option<String>,
    /// When `Some`, the right pane swaps from the plugin detail to a
    /// SKILL.md viewer. Cleared by the `← Back` button.
    pub(super) plugin_view_skill: Option<PluginSkillView>,
    /// Subscription that calls `cx.notify()` whenever the app-wide
    /// `SkillsState` Global changes — so the Plugin page reflects
    /// install / uninstall completions (and external `claude plugin`
    /// CLI runs) without polling.
    _skills_global_subscription: Subscription,
    /// Window-aware observer for file-watcher and cross-window settings
    /// changes. Clean forms reload immediately; a local draft is preserved
    /// for the same-field conflict flow.
    _settings_global_subscription: Subscription,
    /// Refreshes every agent row's pickers when a Workspace observes a new
    /// live vocabulary, including while this Settings window stays open.
    _agent_vocabulary_global_subscription: Subscription,
    /// Subscription that refreshes the `accounts` mirror + repaints whenever
    /// the app-wide `AccountsGlobal` changes — so an add/reauth/default/
    /// delete in any Workspace window shows here without a restart.
    _accounts_global_subscription: Subscription,
    /// Held, not dropped: an `observe_global` unsubscribes when its
    /// `Subscription` falls out of scope, and the readings this one waits for
    /// arrive *after* construction — every probe is a background subprocess.
    _auth_status_subscription: Subscription,
    /// Subscription that calls `cx.notify()` whenever the `Updater`
    /// entity changes status — so the About page reflects check /
    /// download / install progress reactively. `None` when the updater
    /// global never registered (unparseable version). Observing the
    /// entity (not the global) is deliberate: the global holder is set
    /// once at init and never replaced, so `observe_global` would never
    /// fire; the entity self-notifies on every status transition.
    _updater_subscription: Option<Subscription>,
}

/// In-Settings SKILL.md viewer state. The body load is async (disk
/// read on the background executor) so the variant tracks the three
/// observable phases — request in flight, body ready, body failed.
#[derive(Clone)]
pub(super) struct PluginSkillView {
    /// `display_name_for_invocation(skill)` — the namespaced form
    /// shown in the header (`<plugin>:<skill>`).
    pub(super) display_name: String,
    /// Absolute path to the SKILL.md file being viewed. Captured at
    /// open time so the async loader knows exactly which file to read
    /// even if the underlying `SkillsState` reshuffles mid-load.
    pub(super) skill_md_path: std::path::PathBuf,
    pub(super) body: PluginSkillBodyState,
}

#[derive(Clone)]
pub(super) enum PluginSkillBodyState {
    Loading,
    Loaded(SharedString),
    Error(SharedString),
}

/// One entry of the Settings agent catalog. An entry that resolves gets
/// editable fields; one naming a preset daruda cannot launch has no fields to
/// edit and is carried verbatim, so both kinds live in a single ordered list —
/// position survives a save, and no operation can see one kind without the
/// other being in reach.
///
/// `large_enum_variant` is allowed for the same reason as `PaneContent`'s: the
/// list holds one item per configured agent, so Box-ing the row only adds a
/// heap hop to every render read for negligible savings.
#[derive(Clone)]
#[allow(clippy::large_enum_variant)]
enum AgentCatalogItem {
    Editable(AgentCatalogRow),
    Unresolved(daruda_config::AgentEntry),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TextSetting {
    TerminalFontSize,
    TerminalLineHeight,
    TerminalCellWidth,
    EditorFontSize,
    EditorLineHeight,
    AgentChatFontSize,
    AgentChatLineHeight,
    WindowOpacity,
    ScrollbackMaxRows,
    TerminalInsetX,
    TerminalInsetY,
    ClipboardStreamingMaxBytes,
    PanelsGridColumns,
    ClaudeStatusStaleSecs,
    ClaudeStatusFileTtlDays,
    UsageLimitsPollSecs,
    UsageStatusPollSecs,
    PortsPollSecs,
    LogsRetentionDays,
    LogsMaxFileSizeMb,
    PresenceGraceSecs,
    PresenceIdleSecs,
    PresenceIdleForegroundSecs,
    AgentInputMaxRows,
    AgentReadingWidth,
    FlowTimeoutMinutes,
    FlowMaxNodeRuns,
    FlowMaxCost,
    FlowCostCurrency,
    LeftDefaultWidth,
    ShellProgram,
    NotifyLongRunningThresholdSecs,
}

/// Select values for `left_dock.file_icon_color_mode`, matching its
/// `snake_case` config spelling.
const ICON_COLOR: &str = "color";
const ICON_MONOCHROME: &str = "monochrome";

fn icon_color_value(mode: &daruda_config::IconColorMode) -> &'static str {
    match mode {
        daruda_config::IconColorMode::Color => ICON_COLOR,
        daruda_config::IconColorMode::Monochrome => ICON_MONOCHROME,
    }
}

fn icon_color_from_value(value: &str) -> Option<daruda_config::IconColorMode> {
    match value {
        ICON_COLOR => Some(daruda_config::IconColorMode::Color),
        ICON_MONOCHROME => Some(daruda_config::IconColorMode::Monochrome),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelectSetting {
    Language,
    TerminalPreset,
    UiPreset,
    TerminalFontFamily,
    EditorFontFamily,
    AgentChatFontFamily,
    CursorStyle,
    RenderMaxFps,
    SyntaxTheme,
    PreferredEditor,
    FileIconColorMode,
    OrchestratorAgent,
    OrchestratorAccount,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BoolSetting {
    AgentUseModifierToSend,
    AgentUseReadingWidth,
    AgentToolSummaryLabels,
    ShellClosePaneOnExit,
    WindowBlur,
    FilesShowHidden,
    FilesUseGitignore,
    ClaudeStatusEnabled,
    UpdateAutoCheck,
    LeftCollapsedByDefault,
    PreviewTab,
    ShellNaturalTextEditing,
    NotifyOsc9,
    NotifyOsc777,
    NotifyAttention,
    NotifyLongRunning,
    NotifySkipFocusedPane,
    NotifyHook,
    NotifyAgentCompletion,
    NotifyAgentWaiting,
    TelegramOnlyWhenAway,
    ClipboardCopyOnSelect,
    GitConfirmCommit,
    GitConfirmPush,
    GitDefaultCommitMessage,
    TelegramEnabled,
    OrchestratorEnabled,
}

// Rust has no reflection over enum variants, and the `spec` tables are looked
// up by variant, so `ALL` is what lets a test prove every variant has exactly
// one row. `ALL` is itself hand-written, so each is paired with an exhaustive
// `match` that the compiler rejects until a new variant is added to the list
// too — otherwise the missing row would only surface as a panic on the first
// click of the new widget.
impl TextSetting {
    #[cfg(test)]
    const ALL: [Self; 32] = [
        Self::TerminalFontSize,
        Self::TerminalLineHeight,
        Self::TerminalCellWidth,
        Self::EditorFontSize,
        Self::EditorLineHeight,
        Self::AgentChatFontSize,
        Self::AgentChatLineHeight,
        Self::WindowOpacity,
        Self::ScrollbackMaxRows,
        Self::TerminalInsetX,
        Self::TerminalInsetY,
        Self::ClipboardStreamingMaxBytes,
        Self::PanelsGridColumns,
        Self::ClaudeStatusStaleSecs,
        Self::ClaudeStatusFileTtlDays,
        Self::UsageLimitsPollSecs,
        Self::UsageStatusPollSecs,
        Self::PortsPollSecs,
        Self::LogsRetentionDays,
        Self::LogsMaxFileSizeMb,
        Self::PresenceGraceSecs,
        Self::PresenceIdleSecs,
        Self::PresenceIdleForegroundSecs,
        Self::AgentInputMaxRows,
        Self::AgentReadingWidth,
        Self::FlowTimeoutMinutes,
        Self::FlowMaxNodeRuns,
        Self::FlowMaxCost,
        Self::FlowCostCurrency,
        Self::LeftDefaultWidth,
        Self::ShellProgram,
        Self::NotifyLongRunningThresholdSecs,
    ];

    /// Compile-time guard for `ALL`: this match is exhaustive, so a new
    /// variant fails to build until it is listed above as well.
    #[cfg(test)]
    fn _all_is_exhaustive(self) {
        match self {
            Self::TerminalFontSize => (),
            Self::TerminalLineHeight => (),
            Self::TerminalCellWidth => (),
            Self::EditorFontSize => (),
            Self::EditorLineHeight => (),
            Self::AgentChatFontSize => (),
            Self::AgentChatLineHeight => (),
            Self::WindowOpacity => (),
            Self::ScrollbackMaxRows => (),
            Self::TerminalInsetX => (),
            Self::TerminalInsetY => (),
            Self::ClipboardStreamingMaxBytes => (),
            Self::PanelsGridColumns => (),
            Self::ClaudeStatusStaleSecs => (),
            Self::ClaudeStatusFileTtlDays => (),
            Self::UsageLimitsPollSecs => (),
            Self::UsageStatusPollSecs => (),
            Self::PortsPollSecs => (),
            Self::LogsRetentionDays => (),
            Self::LogsMaxFileSizeMb => (),
            Self::PresenceGraceSecs => (),
            Self::PresenceIdleSecs => (),
            Self::PresenceIdleForegroundSecs => (),
            Self::AgentInputMaxRows => (),
            Self::AgentReadingWidth => (),
            Self::FlowTimeoutMinutes => (),
            Self::FlowMaxNodeRuns => (),
            Self::FlowMaxCost => (),
            Self::FlowCostCurrency => (),
            Self::LeftDefaultWidth => (),
            Self::ShellProgram => (),
            Self::NotifyLongRunningThresholdSecs => (),
        }
    }
}

impl SelectSetting {
    #[cfg(test)]
    const ALL: [Self; 13] = [
        Self::Language,
        Self::TerminalPreset,
        Self::UiPreset,
        Self::TerminalFontFamily,
        Self::EditorFontFamily,
        Self::AgentChatFontFamily,
        Self::CursorStyle,
        Self::RenderMaxFps,
        Self::SyntaxTheme,
        Self::PreferredEditor,
        Self::FileIconColorMode,
        Self::OrchestratorAgent,
        Self::OrchestratorAccount,
    ];

    /// Compile-time guard for `ALL`: this match is exhaustive, so a new
    /// variant fails to build until it is listed above as well.
    #[cfg(test)]
    fn _all_is_exhaustive(self) {
        match self {
            Self::Language => (),
            Self::TerminalPreset => (),
            Self::UiPreset => (),
            Self::TerminalFontFamily => (),
            Self::EditorFontFamily => (),
            Self::AgentChatFontFamily => (),
            Self::CursorStyle => (),
            Self::RenderMaxFps => (),
            Self::SyntaxTheme => (),
            Self::PreferredEditor => (),
            Self::FileIconColorMode => (),
            Self::OrchestratorAgent => (),
            Self::OrchestratorAccount => (),
        }
    }
}

impl BoolSetting {
    #[cfg(test)]
    const ALL: [Self; 27] = [
        Self::AgentUseModifierToSend,
        Self::AgentUseReadingWidth,
        Self::AgentToolSummaryLabels,
        Self::ShellClosePaneOnExit,
        Self::WindowBlur,
        Self::FilesShowHidden,
        Self::FilesUseGitignore,
        Self::UpdateAutoCheck,
        Self::LeftCollapsedByDefault,
        Self::PreviewTab,
        Self::ShellNaturalTextEditing,
        Self::NotifyOsc9,
        Self::NotifyOsc777,
        Self::NotifyAttention,
        Self::NotifyLongRunning,
        Self::NotifySkipFocusedPane,
        Self::NotifyHook,
        Self::NotifyAgentCompletion,
        Self::NotifyAgentWaiting,
        Self::TelegramOnlyWhenAway,
        Self::ClipboardCopyOnSelect,
        Self::GitConfirmCommit,
        Self::GitConfirmPush,
        Self::GitDefaultCommitMessage,
        Self::ClaudeStatusEnabled,
        Self::TelegramEnabled,
        Self::OrchestratorEnabled,
    ];

    /// Compile-time guard for `ALL`: this match is exhaustive, so a new
    /// variant fails to build until it is listed above as well.
    #[cfg(test)]
    fn _all_is_exhaustive(self) {
        match self {
            Self::AgentUseModifierToSend => (),
            Self::AgentUseReadingWidth => (),
            Self::AgentToolSummaryLabels => (),
            Self::ShellClosePaneOnExit => (),
            Self::WindowBlur => (),
            Self::FilesShowHidden => (),
            Self::FilesUseGitignore => (),
            Self::UpdateAutoCheck => (),
            Self::LeftCollapsedByDefault => (),
            Self::PreviewTab => (),
            Self::ShellNaturalTextEditing => (),
            Self::NotifyOsc9 => (),
            Self::NotifyOsc777 => (),
            Self::NotifyAttention => (),
            Self::NotifyLongRunning => (),
            Self::NotifySkipFocusedPane => (),
            Self::NotifyHook => (),
            Self::NotifyAgentCompletion => (),
            Self::NotifyAgentWaiting => (),
            Self::TelegramOnlyWhenAway => (),
            Self::ClipboardCopyOnSelect => (),
            Self::GitConfirmCommit => (),
            Self::GitConfirmPush => (),
            Self::GitDefaultCommitMessage => (),
            Self::ClaudeStatusEnabled => (),
            Self::TelegramEnabled => (),
            Self::OrchestratorEnabled => (),
        }
    }
}

impl SettingsView {
    pub fn new(data_dir: std::path::PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::new_with_section(BuiltinSection::default(), data_dir, window, cx)
    }

    /// The host's answer to whether it has a project to configure.
    pub(crate) fn set_project_open(&mut self, open: bool, cx: &mut Context<Self>) {
        if self.project_open != open {
            self.project_open = open;
            cx.notify();
        }
    }

    /// Go to `section` from a nav row or a search result: the query is done
    /// with, so it clears and the page shows in full.
    pub(super) fn open_section(
        &mut self,
        section: BuiltinSection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.sidebar_search_input.read(cx).value().is_empty() {
            self.sidebar_search_input.update(cx, |input, cx| {
                input.set_value(String::new(), window, cx);
            });
        }
        self.focus_section(section, window, cx);
    }

    /// Switch the active page and land focus on its first visible input.
    /// Also what `Workspace::open_settings` calls when Settings is already up.
    pub fn focus_section(
        &mut self,
        section: BuiltinSection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active_section != section {
            self.active_section = section;
            // Reset scroll so the new section starts at the top —
            // otherwise switching from a long page leaves the new page
            // mid-scroll which is confusing.
            self.scroll_handle.set_offset(gpui::point(px(0.), px(0.)));
        }
        // The page's first visible input in layout order, then whatever the
        // page draws by hand; constructor order knows nothing of either.
        if let Some(fh) = self.first_visible_input(section, cx).or_else(|| {
            self.section_focus_targets
                .get(&section)
                .and_then(|handles| handles.first())
                .cloned()
        }) {
            fh.focus(window, cx);
        } else {
            self.panel_focus_handle.focus(window, cx);
        }
        cx.notify();
    }

    #[cfg(test)]
    pub(crate) fn project_open(&self) -> bool {
        self.project_open
    }

    pub fn active_section(&self) -> BuiltinSection {
        self.active_section
    }

    /// Ask the host to take this view down. Committing what is in flight is
    /// the host's job — every exit it knows about (this one, and the window
    /// closing underneath) has to land the same edits, so the funnel lives
    /// there rather than being repeated per gesture.
    fn dismiss(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        cx.emit(SettingsEvent::Close);
    }

    /// The banner text, for the screenshot scenario's test — the one caller
    /// outside this module, which cannot reach a private field.
    #[cfg(all(test, feature = "screenshot"))]
    pub(crate) fn error_for_test(&self) -> Option<&SharedString> {
        self.error.as_ref()
    }

    /// Type a fixed query into the sidebar search — the
    /// `--screenshot-scenario settings-search` entry point. `away` lands on a
    /// parent switch, its dependent row and a folded Advanced card's rows.
    #[cfg(feature = "screenshot")]
    pub(crate) fn seed_search_for_shot(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_search_input.update(cx, |input, cx| {
            input.set_value("away".to_string(), window, cx);
        });
        cx.notify();
    }

    /// Open the first catalog card and its advanced block, narrow the preset
    /// lists with a query, and scroll to them — the `--screenshot-scenario
    /// agent-catalog-expanded` entry point: every part of a card on one screen.
    #[cfg(feature = "screenshot")]
    pub(crate) fn seed_agent_catalog_for_shot(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let first = self.agent_editable_rows().map(|(index, _)| index).next();
        if let Some(index) = first
            && let Some(row) = self.agent_editable_row_mut(index)
        {
            row.fold = CardFold {
                expanded: true,
                advanced: true,
            };
        }
        self.agent_catalog_search.update(cx, |input, cx| {
            input.set_value("gem".to_string(), window, cx);
        });
        self.scroll_handle.scroll_to_bottom();
        cx.notify();
    }

    /// Open the first catalog card with one transcript editor showing — the
    /// `--screenshot-scenario agent-catalog-editor:<axis>` entry point. The
    /// fold axis is taken off the built-in in memory only, with its tool
    /// categories open, so the override pin, a hand-edited matrix and the
    /// nested rows are all on screen; nothing is written to the config.
    #[cfg(feature = "screenshot")]
    pub(crate) fn seed_agent_catalog_editor_for_shot(
        &mut self,
        axis: sections::agent_transcript::editor::EditorShot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::transcript::fold_mode::{BlockRule, FoldBlock, FoldPreset, TurnPosition};
        let first = self.agent_editable_rows().map(|(index, _)| index).next();
        if let Some(index) = first
            && let Some(row) = self.agent_editable_row_mut(index)
        {
            row.fold = CardFold {
                expanded: true,
                advanced: true,
            };
            row.fold_mode = Some(FoldPreset::Summary.mode().with_rule(
                TurnPosition::Last,
                FoldBlock::Thinking,
                BlockRule::Collapsed,
            ));
            row.fold_editor.toggle_tools(TurnPosition::Last);
            row.shot_editor = Some(axis);
        }
        // The same scroll as `agent-catalog-expanded`: it puts the card's
        // transcript fields high on screen, with room below for the panel.
        self.agent_catalog_search.update(cx, |input, cx| {
            input.set_value("gem".to_string(), window, cx);
        });
        self.scroll_handle.scroll_to_bottom();
        cx.notify();
    }

    /// Raise the failure banner with a representative message — the
    /// `--screenshot-scenario settings-error` entry point. No action is
    /// actually attempted; a capture must not depend on a write failing.
    #[cfg(feature = "screenshot")]
    pub(crate) fn seed_error_for_shot(&mut self, cx: &mut Context<Self>) {
        self.error = Some(SharedString::from(s::settings::err_telegram_unpair(
            "Permission denied (os error 13)",
        )));
        cx.notify();
    }

    /// Put a failed action on screen and in the log. The one place a Settings
    /// action reports a failure the user asked for and did not get.
    pub(super) fn report_section_error(
        &mut self,
        text: String,
        report: daruda_store::observability::error_report::ErrorReportBuilder,
        cx: &mut Context<Self>,
    ) {
        LogWriter::log(report.build());
        self.error = Some(SharedString::from(text));
        cx.notify();
    }

    #[cfg(test)]
    fn focus_next_input(&self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        if forward {
            window.focus_next(cx);
        } else {
            window.focus_prev(cx);
        }
    }

    pub(super) fn section_label(
        label: impl Into<gpui::SharedString>,
        cx: &gpui::App,
    ) -> impl IntoElement {
        div()
            .text_size(px(theme::MODAL_BODY_FONT_SIZE))
            .font_weight(gpui::FontWeight::MEDIUM)
            .text_color(theme::current(cx).text_primary)
            .mt(px(theme::MODAL_FOOTER_MARGIN_TOP))
            .child(label.into())
    }
}

/// Curated syntax palettes. Value = config key resolved by
/// `syntax::SyntaxPalette::from_config_name`; labels are i18n'd via
/// [`syntax_theme_label`]. The recommended Daruda palette is first.
const SYNTAX_THEMES: &[&str] = &[
    "daruda",
    "one-dark",
    "tokyo-night",
    "catppuccin-mocha",
    "dracula",
    "github-dark",
    "material-palenight",
    "monokai",
    "nord",
    "gruvbox-dark",
    "solarized-dark",
    "ayu-mirage",
    "night-owl",
    "darcula",
];

/// Localized display label for a syntax-palette config value.
fn syntax_theme_label(value: &str) -> String {
    match value {
        "one-dark" => s::settings::syntax_theme_one_dark(),
        "tokyo-night" => s::settings::syntax_theme_tokyo_night(),
        "catppuccin-mocha" => s::settings::syntax_theme_catppuccin_mocha(),
        "dracula" => s::settings::syntax_theme_dracula(),
        "github-dark" => s::settings::syntax_theme_github_dark(),
        "material-palenight" => s::settings::syntax_theme_material_palenight(),
        "monokai" => s::settings::syntax_theme_monokai(),
        "nord" => s::settings::syntax_theme_nord(),
        "gruvbox-dark" => s::settings::syntax_theme_gruvbox_dark(),
        "solarized-dark" => s::settings::syntax_theme_solarized_dark(),
        "ayu-mirage" => s::settings::syntax_theme_ayu_mirage(),
        "night-owl" => s::settings::syntax_theme_night_owl(),
        "darcula" => s::settings::syntax_theme_darcula(),
        _ => s::settings::syntax_theme_daruda(),
    }
}

/// Seconds since the Unix epoch, clamped to `0` on a clock error — mirrors
/// `sections::accounts::now_unix`. Duplicated locally rather than shared:
/// both are trivial, single-use timestamp helpers that would gain nothing
/// from a shared home.
fn now_unix() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The registry editor's rows have no natural field label beyond their
/// position, so the validator's own reason is wrapped with the row's 1-based
/// ordinal — a multi-row save can still point at which one failed.
fn session_host_validation_message(
    index: usize,
    err: session_host::SessionHostError,
) -> SharedString {
    SharedString::from(s::settings::err_session_host_field(
        index + 1,
        s::session_host::host_error(err),
    ))
}

/// Return all font family names available on this system, sorted alphabetically.
/// Every configured family remains selectable even if it is no longer installed.
fn all_font_names(cx: &gpui::App, current: &[&str]) -> Vec<String> {
    let mut names = cx.text_system().all_font_names();
    names.sort();
    names.dedup();
    let include_system_ui = current.contains(&daruda_config::SYSTEM_UI_FONT_FAMILY);
    names.retain(|name| name != daruda_config::SYSTEM_UI_FONT_FAMILY || include_system_ui);
    for family in current.iter().rev().filter(|family| !family.is_empty()) {
        if !names.iter().any(|name| name == *family) {
            names.insert(0, (*family).to_owned());
        }
    }
    names
}

fn font_select_option(name: &str) -> SelectOption {
    let label = if name == daruda_config::SYSTEM_UI_FONT_FAMILY {
        s::settings::font_system_ui()
    } else {
        name.to_owned()
    };
    SelectOption::new(name.to_owned(), label)
}

fn font_select_options(cx: &gpui::App, current: &[&str]) -> Vec<SelectOption> {
    all_font_names(cx, current)
        .iter()
        .map(|name| font_select_option(name))
        .collect()
}

// `render::*` is just the `impl Render for SettingsView` block —
// no items are re-exported, but keeping the module declaration above
// is what makes the impl visible to external callers.
