//! Terminology (keep distinct): `Tab bar` = top strip, one per workspace;
//! `Tab cell` = one entry in it; `Pane` = a leaf in the split tree
//! (PTY + TerminalView); `Pane header` = per-pane title bar, split mode only.
//! Never use bare `header` / `title_bar` — always `tab_*` or `pane_*`.
//! `terminal_title()` (daruda_terminal) feeds both tab cells and pane headers.

mod accessors;
mod account_login_ops;
mod account_ops;
pub(crate) mod accounts_global;
mod actions;
mod agent_outcome_ops;
mod agent_usage;
pub(crate) mod agent_vocabulary_global;
mod annotation_dialog;
mod annotation_ops;
pub(crate) mod auth_status_global;
mod availability_ops;
mod claude_session_ops;
mod claude_status_aggregate;
pub(in crate::workspace) mod command;
mod config_ops;
mod config_passes;
mod config_sync;
mod construct;
mod control_flow_ops;
mod control_lane_ops;
mod control_ops;
mod control_task_ops;
mod notification_ops;
/// The external control surface's vocabulary answer — see [`SlashClaim`].
/// Re-exported because `telegram::global`'s poll loop folds per-window answers
/// and lives outside `crate::workspace`.
pub(crate) use control_ops::{ChatLabel, SlashClaim};
pub(crate) use control_task_ops::ControlTaskStart;
mod close_guard_ops;
pub(crate) mod delete_project_modal;
pub(crate) mod dialog_helpers;
mod dirty_items;
mod dnd_ops;
mod dock_badge_ops;
mod dock_refresh;
mod durable;
pub(in crate::workspace) mod error;
pub(in crate::workspace) mod flow_ask_modal;
mod flow_browser;
pub(in crate::workspace) mod flow_cache;
mod flow_context;
mod flow_events;
mod flow_file_ops;
pub(in crate::workspace) mod flow_history;
mod flow_ops;
mod flow_paths;
mod flow_pins;
pub(in crate::workspace) mod flow_request;
mod flow_rows;
pub(in crate::workspace) mod flow_runs;
mod group_ops;
pub(in crate::workspace) mod group_select_modal;
mod lane_ops;
mod lane_scoped;
pub(in crate::workspace) mod layout;
mod left_dock;
mod lifetimes;
pub(in crate::workspace) mod main_area;
pub(in crate::workspace) mod modal_view;
pub(crate) mod open_project_modal;
mod orchestrator_ops;
mod pages;
mod palette_dispatch;
mod path_drag;
mod persistence;
mod project_ops;
mod project_palette_ops;
mod render;
mod right_dock;
mod root_menu;
#[cfg(feature = "screenshot")]
pub(crate) mod screenshot_scenario;
mod settings_ops;
mod settings_update;
mod spawn_helpers;
pub(crate) mod status_bar;
mod status_bar_ops;
pub(in crate::workspace) mod sync;
mod tab_indicator;
#[cfg(test)]
mod tests;
mod toast_layer;
mod unseen_outcomes;
mod update_ops;
mod usage_labels;
mod window_close_ops;
mod window_runtime;

pub(in crate::workspace) use config_sync::ConfigMirrors;
pub(in crate::workspace) use modal_view::ModalView;
pub(in crate::workspace) use persistence::LaneRuntime;

use std::collections::{HashMap, HashSet};

use gpui::{Context, actions};
// Test modules reach `Window` through `use super::*`.
#[cfg(test)]
use gpui::Window;

// ----------------------------------------------------------------
// Actions
// ----------------------------------------------------------------

/// Parameterized action fired when a user-defined macro shortcut is
/// pressed. Carries the shortcut string so the handler resolves it back
/// to the macro at dispatch time (bound by
/// `main_area::bottom_dock::macro_ops::register_macro_shortcuts`).
/// `no_json` skips the keymap-deserialize derive — constructed only at
/// runtime by the registrar, never loaded from keymap.json.
#[derive(Clone, PartialEq, Debug, gpui::Action)]
#[action(namespace = workspace, no_json)]
pub struct RunMacroByShortcut(pub gpui::SharedString);

/// Open the Settings window on the requested page. Carries
/// [`daruda_config::BuiltinSection`] so each menu / palette entry lands
/// directly on the right page instead of General. `no_json`: daruda's
/// keybinding override path lives in `surface::action_map` (see the
/// `open_settings.<slug>` matching in `bind!`), not GPUI's keymap.json.
#[derive(Clone, PartialEq, Debug, gpui::Action)]
#[action(namespace = workspace, no_json)]
pub struct OpenSettings(pub daruda_config::BuiltinSection);

/// Switch the focused pane's managed account. Carries an
/// [`daruda_store::accounts::AccountSelection`] so the dropdown's
/// per-account menu item can dispatch a concrete managed-account target
/// ([`AccountSelection::Managed`]) while its "System default" entry
/// dispatches [`AccountSelection::SystemDefault`] — reverting the pane back
/// to `~/.claude`. There is no sensible default account to bind a keyboard
/// shortcut to, so this has no `SHORTCUT_*` const (see
/// `surface::keybindings`) — dispatched only from the status-bar account
/// dropdown (`status_bar::build_account_menu`) via `window.dispatch_action`
/// / a direct `Workspace::switch_pane_account` call. `no_json`: never
/// loaded from keymap.json.
#[derive(Clone, PartialEq, Debug, gpui::Action)]
#[action(namespace = workspace, no_json)]
pub struct SwitchPaneAccount(pub daruda_store::accounts::AccountSelection);

/// Start a headless add-account login. Carries the [`AccountRecipeId`] bucket
/// the resulting account is filed under; the login *command* is
/// resolved for that same domain (`Workspace::login_cli_for_recipe`),
/// so the two can't disagree. `no_json`:
/// dispatched only from the status-bar "+ Add account…" menu item, never
/// from keymap.json.
///
/// [`AccountRecipeId`]: daruda_store::accounts::AccountRecipeId
#[derive(Clone, PartialEq, Debug, gpui::Action)]
#[action(namespace = workspace, no_json)]
pub struct AddManagedAccount(pub daruda_store::accounts::AccountRecipeId);

/// Active lane's branch state, derived once per render and shared
/// by the status bar (text label + inline detached chip) and the
/// macOS window title (text only — chip cannot ride along).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::workspace) enum BranchStatus {
    /// Git-backed lane with an attached branch.
    Branch(String),
    /// Git-backed lane on a detached HEAD (no branch).
    Detached,
    /// Non-git project (default-kind lane) or no active project.
    NotGit,
}

actions!(
    workspace,
    [
        NewTab,
        CloseTab,
        ClosePane,
        NextTab,
        PrevTab,
        SplitRight,
        SplitDown,
        FocusNextPane,
        FocusPrevPane,
        FocusPaneLeft,
        FocusPaneRight,
        FocusPaneUp,
        FocusPaneDown,
        MoveTabLeft,
        MoveTabRight,
        ActivateTab1,
        ActivateTab2,
        ActivateTab3,
        ActivateTab4,
        ActivateTab5,
        ActivateTab6,
        ActivateTab7,
        ActivateTab8,
        ActivateTab9,
        ToggleLeftDock,
        ToggleGitChangesFocus,
        ToggleFilesFocus,
        ToggleBottomDock,
        ToggleRightDock,
        ToggleCommandPalette,
        ToggleLaneSwitcher,
        RunFlow,
        ValidateFlow,
        ShowFlowGraph,
        ReloadFlowGraph,
        ActivateLane1,
        ActivateLane2,
        ActivateLane3,
        ActivateLane4,
        ActivateLane5,
        ActivateLane6,
        ActivateLane7,
        ActivateLane8,
        ActivateLane9,
        ShowLeftDockLanes,
        ShowLeftDockGit,
        ShowLeftDockFiles,
        SwitchRightPanelUsage,
        SwitchRightPanelSkills,
        SwitchRightPanelTools,
        SwitchRightPanelTasks,
        SwitchRightPanelFlows,
        NewSkill,
        FocusSkillSearch,
        InvokeSkillPalette,
        RefreshGitStatus,
        CommitChanges,
        CommitAmend,
        PushChanges,
        FetchChanges,
        PullChanges,
        FileViewerSearchOpen,
        FileViewerSearchNext,
        FileViewerSearchPrev,
        FilesToggleHidden,
        FilesSelectNext,
        FilesSelectPrev,
        FilesActivate,
        FilesExpand,
        FilesCollapse,
        FilesRefresh,
        GitChangesSelectNext,
        GitChangesSelectPrev,
        GitChangesToggleStage,
        GitChangesActivate,
        OpenProjectConfig,
        InstallAgentHooks,
        UninstallAgentHooks,
        MinimizeWindow,
        ZoomWindow,
        /// Ask the window to close, honouring the dirty-draft prompt the
        /// platform's own close path runs. Dispatched by the app-drawn
        /// caption button, which cannot reach that callback directly.
        CloseWindow,
        ToggleFullScreen,
        EditWindowTitle,
        OpenCommandHistory,
        CloseOtherTabs,
        CloseTabsToRight,
        ToggleZoomPane,
        /// Open a fresh task draft on the Tasks page (no task_id). Wired
        /// from the Command Palette `new_task`; keybinding via
        /// `[keybindings] new_task`.
        NewTask,
        /// Open the task picker for all tasks; the pick gets a TaskEdit
        /// pane. Keybinding via `[keybindings] edit_task`.
        EditTask,
        /// Prompt for a group name and create an empty group in the
        /// shared tab-order pool. Wired from `new_group`.
        NewGroup,
        /// Rename the active workspace project via a single-field modal.
        /// Wired from `rename_project`.
        RenameActiveProject,
        /// Move the active project into a group by name (blank =
        /// ungrouped, unknown name creates one). Wired from
        /// `move_project_to_group`.
        MoveActiveProjectToGroup,
        /// Save the focused file-view pane to disk. Wired to `cmd-s` in
        /// the `FileViewer` focus scope.
        SaveFilePane,
        /// Open a fresh Agent chat (ACP) pane in a new tab at the active
        /// lane. Wired from `new_agent_chat`; keybinding via
        /// `[keybindings] new_agent_chat`.
        OpenAgentChat,
    ]
);

// ----------------------------------------------------------------
// Workspace
// ----------------------------------------------------------------

const TITLE_BAR_HEIGHT: f32 = crate::ui::theme::TITLE_BAR_HEIGHT;
const TAB_BAR_HEIGHT: f32 = crate::ui::theme::TAB_BAR_HEIGHT;

pub struct Workspace {
    /// Stable cross-session identifier — matches the UUID stored on disk
    /// at `workspaces/<uuid>.json`. Minted at construction, then replaced
    /// once by `restore_from_disk` when this window adopts a saved
    /// workspace; never changes after that.
    pub(in crate::workspace) uuid: daruda_store::project::WorkspaceUuid,
    /// TabBar + PaneTree runtime state. Holds every lane's runtime
    /// (tabs / panes / focus, keyed by `LaneRef`) plus the
    /// drag/context-menu overlays.
    pub(in crate::workspace) main_area: main_area::MainAreaContext,
    /// See [`orchestrator_ops::OrchestratorHost`].
    orchestrator: orchestrator_ops::OrchestratorHost,
    next_id: u64,
    /// The window this workspace lives in — see
    /// [`window_runtime::WindowRuntime`].
    pub(in crate::workspace) window_runtime: window_runtime::WindowRuntime,
    /// The three docks and which view each side shows — see
    /// [`layout::docks::Docks`].
    pub(in crate::workspace) docks: layout::docks::Docks,
    pub(in crate::workspace) workspace_page: Option<pages::PageState>,
    /// What each page keeps while it is not on screen — see
    /// [`pages::detail::PageDetails`].
    pub(in crate::workspace) pages: pages::detail::PageDetails,
    /// Provider-neutral usage, activity, session titles, and their poll tasks.
    pub(in crate::workspace) agent_usage: agent_usage::AgentUsageState,
    /// Claude-specific hooks, status, PTY tracking, and JSONL fallback tasks.
    pub(in crate::workspace) claude_sessions: claude_session_ops::ClaudeSessionState,
    /// Outcomes of turns that ended while their pane was not seen — what
    /// the tab strip's result dot reads. See `agent_outcome_ops`.
    pub(in crate::workspace) unseen_outcomes: unseen_outcomes::UnseenOutcomes,
    /// Runtime projects in this workspace. Zero entries = empty (Landing
    /// screen). Each project owns its own lanes, reached via
    /// `projects[i].lanes`. `tabs`/`panes` live on the active lane's
    /// `MainAreaContext` slot.
    pub(in crate::workspace) projects: crate::project::Projects,
    /// Active (project, lane) pair. When `projects` is non-empty,
    /// always points at a live entry — kept normalized by
    /// `activate_lane` and `finalize_remove_*` paths.
    pub(in crate::workspace) active: daruda_store::project::LaneRef,
    /// User-defined groups in the left dock. Each project's `group_id`
    /// links into this list.
    pub(in crate::workspace) groups: Vec<daruda_store::project::SerializedGroup>,
    /// Monotonic counter for the next `ProjectId` minted by
    /// [`Workspace::add_project`]. Deleted ids are never reused.
    pub(in crate::workspace) next_project_id: daruda_store::project::ProjectId,
    /// Monotonic counter for the next `GroupId` minted by Group CRUD.
    pub(in crate::workspace) next_group_id: daruda_store::project::GroupId,
    /// User policy for the "Open Project…" affordance. Read by the
    /// global `OpenFolder` handler to decide between adding to the
    /// current window vs. opening a new one; mutated when the user
    /// ticks "Don't ask again" inside the chooser modal.
    pub(in crate::workspace) window_open_policy: daruda_store::project::WindowOpenPolicy,
    /// What the right dock's tabs remember — see
    /// [`right_dock::context::RightDockViews`].
    pub(in crate::workspace) right_views: right_dock::context::RightDockViews,
    /// The bottom dock's shared input and each pane's draft — see
    /// [`main_area::bottom_dock::input_dock::InputDock`].
    pub(in crate::workspace) input_dock: main_area::bottom_dock::input_dock::InputDock,
    /// The Settings view this window is showing, if any. `Some` *is* settings
    /// mode: it swaps the body and gates the workspace actions, so there is no
    /// separate flag to disagree with it.
    pub(in crate::workspace) settings: Option<settings_ops::SettingsHost>,
    /// Command palette, lane switcher and flow picker — see
    /// [`command::overlays::CommandOverlays`].
    pub(in crate::workspace) overlays: command::overlays::CommandOverlays,
    /// Flow runs and the Flows tab — see [`flow_context::FlowContext`].
    pub(in crate::workspace) flows: flow_context::FlowContext,
    /// Lazy per-lane state, removed together by lane and project teardown.
    pub(in crate::workspace) lane_scoped:
        HashMap<daruda_store::project::LaneRef, lane_scoped::LaneScoped>,
    /// Shared Files panel polling, focus, selection, and scrolling.
    /// Per-lane data and watchers live in `lane_scoped`.
    pub(in crate::workspace) file_tree: left_dock::file_tree_context::FileTreeContext,
    /// Single source of truth for live-reloaded config mirror state.
    /// `apply_config` is the only update site (other than per-field
    /// toggle methods like `toggle_files_show_hidden`).
    pub(in crate::workspace) mirrors: ConfigMirrors,
    /// Scroll handle for the Lanes view card list — shared with the
    /// scrollbar overlay. The view header stays outside the scroll
    /// region, so only the group / project cards move.
    pub(in crate::workspace) lanes_scroll_handle: gpui::ScrollHandle,
    /// Inline status-bar message surface for the **pane-spawn** failure
    /// path (`report_pane_error`). All git / hooks / mcp / skills / files
    /// ops route through the 3-layer error pipeline (toast → details
    /// modal → NDJSON log) via `report_error`; do not reach for
    /// `last_error` for those. New IO/exec failures belong in the
    /// pipeline. Cleared by the next successful spawn — the retry.
    pub(in crate::workspace) last_error: Option<gpui::SharedString>,
    /// Every report `report_error` surfaced, newest-first. Test-only: the
    /// toast queue caps and merges reports, so tests that count them read
    /// this instead.
    #[cfg(test)]
    pub(in crate::workspace) error_history:
        Vec<daruda_store::observability::error_report::ErrorReport>,
    /// Toast notification layer — owns the live queue, the 1 Hz expiry
    /// sweep, and the overlay renderer. Isolated into its own entity so
    /// a toast change triggers only the toast layer's repaint, not a
    /// full Workspace repaint.
    pub(in crate::workspace) toast_layer: gpui::Entity<toast_layer::ToastLayer>,
    /// Memoized status-bar "project config layer exists" flag, keyed by the
    /// active project root. `render()` runs every frame, so statting per
    /// render burned syscalls; recompute only when the root changes.
    /// `reload_config` clears it so a freshly created layer surfaces
    /// immediately.
    pub(in crate::workspace) cached_project_config: Option<(std::path::PathBuf, bool)>,
    /// The agent the user most recently opened a chat pane under (session-local,
    /// not persisted). A fresh pane defaults to this so switching agents "sticks"
    /// for the window; falls back to the catalog default when unset or stale.
    pub(in crate::workspace) last_agent_id: Option<String>,
    /// AgentChat view ids that were Working on the previous status-pulse
    /// tick. Lets the pump paint one final settled frame for a just-settled
    /// view whose own settling notify can miss a cached view (gpui
    /// `detect_accessed_entities` lost-wakeup, see
    /// `lane_switch_scroll_dead_rootcause`). Runtime-only; never serialized.
    pub(in crate::workspace) agent_pulse_prev: Vec<gpui::EntityId>,
    /// Git Changes view state and the git locks / watchers every git op
    /// shares — see [`left_dock::git_ops::context::GitContext`].
    pub(in crate::workspace) git: left_dock::git_ops::context::GitContext,
    /// Debounce for the left dock's arrow-key preview, shared by the Git and
    /// Files panels. Holds at most one armed timer; re-arming drops it, which
    /// cancels the row the cursor left.
    pub(in crate::workspace) left_dock_preview: Option<gpui::Task<()>>,
    /// Compatibility root for configuration, accounts and other feature data.
    /// Injected at construction time: production passes `default_data_dir()`,
    /// tests pass a per-test temp directory.
    pub(in crate::workspace) data_dir: std::path::PathBuf,
    /// Workspace state persistence, independent of configuration and account paths.
    pub(in crate::workspace) workspace_store: daruda_store::project::WorkspaceStore,
    /// Where flow run locks live. Injected for the same reason `data_dir`
    /// is, and it has to be a second field rather than derived from it:
    /// production wants the **shared, profile-independent** root, so that a
    /// release build and a debug build running flows in one checkout
    /// exclude each other, and no rule turns one profile's data directory
    /// into that. A test wants the opposite — its own — because taking
    /// locks in the developer's real root litters it with mirrors of
    /// temporary paths, and a suite that thought it was isolated was not.
    pub(in crate::workspace) lock_root: std::path::PathBuf,
    /// Customizable bottom-dock panels — user-managed tabs of macro
    /// widgets. Loaded from `panels.json` on construction (or seeded
    /// with Claude/Codex/Gemini on first launch). Mutations route
    /// through `main_area::bottom_dock::macro_ops` and persist via
    /// `main_area::bottom_dock::macro_ops::save_panels`.
    pub(in crate::workspace) panels: daruda_store::panels::PanelsState,
    /// Per-agent mode / model option lists, as last advertised by each
    /// adapter. Mirrored from the app-wide owner; ACP event pumps in every
    /// Workspace can update that shared snapshot without overwriting each
    /// other's observations.
    pub(in crate::workspace) agent_vocabulary: daruda_store::agent_vocabulary::AgentVocabularyCache,
    /// Global observers installed once — see [`lifetimes::GlobalObservers`].
    _observers: lifetimes::GlobalObservers,
    /// Managed accounts across every auth domain — the catalog a pane's
    /// `AccountSelection` resolves against, plus the per-domain default seeded
    /// onto a freshly-created pane (see
    /// `Workspace::default_account_selection_for_new_pane`); resolution itself
    /// never falls back to that default, so an explicit `SystemDefault` always
    /// means the ambient environment.
    ///
    /// Read-cache of the app-wide [`accounts_global::AccountsGlobal`] (the
    /// single source of truth): seeded at construction, refreshed *only* by
    /// its observer in [`lifetimes::GlobalObservers`]. Cached as a field so
    /// the many cx-free read sites (`main_area::pane::resolve_pane_account`
    /// callers, `focused_account`, the status-bar slot) stay cx-free, exactly
    /// like the config fields cache `SettingsStore`.
    pub(in crate::workspace) accounts: daruda_store::accounts::AccountsState,
    /// The login this window has in flight — see
    /// [`account_login_ops::LoginState`].
    pub(in crate::workspace) login: account_login_ops::LoginState,
    /// Set by a screenshot scenario: the fixtures it seeds into this live
    /// workspace must never reach the profile the window was restored from.
    pub(in crate::workspace) persistence_suspended: bool,
    /// Per-repo lock that prevents two concurrent `start_task`
    /// invocations from racing on `git worktree add` against the same
    /// repository. Cleared after `finalize_create_lane` returns.
    pub(in crate::workspace) pending_lane_creates: HashSet<std::path::PathBuf>,
    /// Background watches and their pumps — see [`lifetimes::Pumps`].
    pub(in crate::workspace) pumps: lifetimes::Pumps,
    /// Last version surfaced via the "update available" toast. Guards
    /// against re-toasting the same version on repeated `Available`
    /// notifies.
    last_update_toast_version: Option<semver::Version>,
    /// Latest listening-TCP-port scan — see [`sync::ports::PortsState`].
    pub(in crate::workspace) ports: sync::ports::PortsState,
}

impl Workspace {
    /// Notify that workspace state has changed and should be persisted.
    ///
    /// Always deferred: `save_state` reads all three dock entities, and this
    /// method is reachable from Dock event listeners (dock clicks). At that
    /// point the Dock entity is still in `EntityState::Mut` on the GPUI call
    /// stack, so a synchronous `.read(cx)` on the same entity would panic.
    /// Deferring schedules the persist for the next effect cycle, by which time
    /// all entity borrows have been released.
    pub(in crate::workspace) fn mark_dirty_and_save(&mut self, cx: &mut Context<Self>) {
        let weak = cx.weak_entity();
        cx.defer(move |cx| {
            weak.update(cx, |ws, cx| ws.persist_state(cx)).ok();
        });
        // Persistence-worthy mutations always change something the UI
        // renders too — left-dock tree, status bar, window title. Without
        // this notify the dock keeps the stale snapshot until an unrelated
        // event fires. Keep it so every call site gets a render for free.
        cx.notify();
    }

    fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
}
