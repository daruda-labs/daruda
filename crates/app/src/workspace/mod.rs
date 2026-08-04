//! Terminology (keep distinct): `Tab bar` = top strip, one per workspace;
//! `Tab cell` = one entry in it; `Pane` = a leaf in the split tree
//! (PTY + TerminalView); `Pane header` = per-pane title bar, split mode only.
//! Never use bare `header` / `title_bar` — always `tab_*` or `pane_*`.
//! `terminal_title()` (daruda_terminal) feeds both tab cells and pane headers.

mod account_login_ops;
mod account_ops;
pub(crate) mod accounts_global;
mod actions;
mod annotation_dialog;
mod annotation_ops;
mod availability_ops;
mod claude_session_ops;
mod claude_status_aggregate;
pub(in crate::workspace) mod command;
mod config_ops;
mod config_sync;
pub(crate) mod delete_project_modal;
pub(crate) mod dialog_helpers;
mod dnd_ops;
mod durable;
pub(in crate::workspace) mod error;
mod group_ops;
pub(in crate::workspace) mod group_select_modal;
mod lane_ops;
pub(in crate::workspace) mod layout;
mod left_dock;
pub(in crate::workspace) mod main_area;
pub(in crate::workspace) mod modal_view;
pub(crate) mod open_project_modal;
mod path_drag;
mod persistence;
mod project_ops;
mod project_palette_ops;
mod render;
mod right_dock;
#[cfg(feature = "screenshot")]
pub(crate) mod screenshot_scenario;
mod spawn_helpers;
pub(crate) mod status_bar;
mod status_bar_ops;
pub(in crate::workspace) mod sync;
#[cfg(test)]
mod tests;
mod toast_layer;
mod update_ops;
mod usage_labels;
mod window_close_ops;

pub(in crate::workspace) use config_sync::ConfigMirrors;
pub(in crate::workspace) use modal_view::ModalView;
pub(in crate::workspace) use persistence::LaneRuntime;

use std::collections::{HashMap, HashSet};

use gpui::{AppContext, Context, FocusHandle, Window, actions};

use daruda_terminal::TerminalConfig;

use main_area::agent_chat_pane::telegram_ops::partition_deferred;

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

/// Switch the focused pane's managed account (Task 8, A+C hybrid). Carries
/// an [`daruda_store::accounts::AccountSelection`] so the dropdown's
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

/// Start a headless add-account login (Plan B — see
/// `account_login_ops::add_managed_account`). Carries the [`AccountRecipeId`]
/// bucket the resulting account is filed under; the login *command* is
/// resolved for that same domain (`Workspace::login_command_for_recipe`),
/// so the two can't disagree. `no_json`:
/// dispatched only from the status-bar "+ Add account…" menu item, never
/// from keymap.json.
///
/// [`AccountRecipeId`]: daruda_store::accounts::AccountRecipeId
#[derive(Clone, PartialEq, Debug, gpui::Action)]
#[action(namespace = workspace, no_json)]
pub struct AddManagedAccount(pub daruda_store::accounts::AccountRecipeId);

/// Re-run a headless login for an **existing** managed account (Plan B —
/// see `account_login_ops::reauthenticate_account`). Carries the target
/// [`daruda_store::accounts::AccountId`] rather than a provider: unlike
/// [`AddManagedAccount`], this reuses the account's existing config dir
/// and identity row instead of minting a new one, so the concrete account
/// must be known up front. `no_json`: dispatched only from the Settings
/// window's Accounts section "Reauthenticate" button (via
/// `Window::dispatch_action` on the target `Workspace` window resolved
/// through `WindowRegistry::first_workspace`), never from keymap.json.
#[derive(Clone, PartialEq, Debug, gpui::Action)]
#[action(namespace = workspace, no_json)]
pub struct ReauthenticateAccount(pub daruda_store::accounts::AccountId);

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
        ToggleBottomDock,
        ToggleRightDock,
        ToggleCommandPalette,
        ToggleLaneSwitcher,
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
        ToggleFullScreen,
        EditWindowTitle,
        OpenCommandHistory,
        CloseOtherTabs,
        CloseTabsToRight,
        ToggleZoomPane,
        /// Open a fresh TaskEdit pane in draft mode (no task_id). Wired
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

/// State of the Commit split button. `Amend` carries `saved_draft` — the
/// commit-box text captured at the moment amend mode was entered — so
/// "Cancel Amend" restores exactly that (the user's own message, or empty)
/// instead of wiping a draft they meant to commit normally.
#[derive(Debug, Clone)]
pub(in crate::workspace) enum CommitMode {
    Normal,
    Amend { saved_draft: String },
}

/// State of an in-flight headless add-account login (Plan B — see
/// `account_login_ops::add_managed_account`). At most one at a time; a second
/// `AddManagedAccount` while `InProgress` is expected to be blocked by the
/// UI (a disabled "+ Add account" affordance while a login is running),
/// not by this enum itself.
///
/// `InProgress` carries the cancel [`LoginProcessHandle`], the `account_id`
/// the pending login will file under, and the `recipe` it signed into — but
/// not the config dir, which is a pure function of data already on
/// `Workspace` (`daruda_agent::accounts::account_config_dir(&self.data_dir,
/// account_id)`). `recipe` is carried because a cancel has to clean that dir
/// up through the right auth domain, and only the spawning flow knows which
/// one it launched.
///
/// `mode` distinguishes an add-account login (whose `account_id` names a
/// throwaway config dir that only becomes real on success) from a
/// reauthenticate login (whose `account_id` names an *existing*
/// [`daruda_agent::accounts::ManagedAccount`]'s real, permanent config dir
/// and Keychain item). `Workspace::cancel_pending_login` reads it to decide
/// whether cancelling is allowed to delete that directory — for `Reauth`
/// it must not, or cancelling a reauth would delete a good account's
/// credentials.
///
/// `Preparing` covers the window before a login process even exists: the
/// managed-node resolve (`account_login_ops::resolve_node_path_env`) is blocking
/// and, on a first-run machine, downloads Node.js, so it runs on the
/// background executor rather than the UI thread — this variant is what
/// `can_start_login` blocks a second concurrent login on, and what
/// `cancel_pending_login` can still cancel (no handle to kill yet, so it
/// just clears the state), during that async gap before `spawn_login`
/// produces a real [`LoginProcessHandle`] and the state advances to
/// `InProgress`.
#[derive(Debug, Clone)]
pub(in crate::workspace) enum PendingLogin {
    None,
    Preparing {
        account_id: daruda_store::accounts::AccountId,
        recipe: daruda_store::accounts::AccountRecipeId,
        mode: account_login_ops::LoginMode,
    },
    InProgress {
        account_id: daruda_store::accounts::AccountId,
        recipe: daruda_store::accounts::AccountRecipeId,
        // Read by `Workspace::cancel_pending_login` (`handle.cancel()`),
        // wired to the status-bar dropdown's Cancel row.
        handle: daruda_agent::accounts::LoginProcessHandle,
        mode: account_login_ops::LoginMode,
    },
}

pub struct Workspace {
    /// Stable cross-session identifier — matches the UUID stored on disk
    /// at `workspaces/<uuid>.json`. Minted at construction, never changes
    /// for the entity's lifetime. Read by the v3 persistence path;
    /// otherwise unread outside tests.
    #[allow(dead_code)]
    pub(in crate::workspace) uuid: daruda_store::project::WorkspaceUuid,
    /// TabBar + PaneTree runtime state. Holds every lane's runtime
    /// (tabs / panes / focus, keyed by `LaneRef`) plus the
    /// drag/context-menu overlays.
    pub(in crate::workspace) main_area: main_area::MainAreaContext,
    next_id: u64,
    focus_handle: FocusHandle,
    /// Dock resize drag — active while the user is pulling on the
    /// right edge of the left dock, the left edge of the right dock,
    /// or the top edge of the bottom dock.
    pub(in crate::workspace) dock_drag: Option<layout::ops::DockDrag>,
    /// When true, new tabs/panes spawn with the focused pane's cwd
    /// (iTerm2 "Reuse previous session's directory").
    pub(in crate::workspace) inherit_cwd: bool,
    /// Terminal config applied to every new pane (font size + iTerm2-style
    /// spacing multipliers). Single source of truth; `resize_all_tabs`
    /// reads the same settings to measure cells consistently with
    /// TerminalView.
    pub(in crate::workspace) terminal_config: TerminalConfig,
    /// Primary font family from config. Applied to each new pane's
    /// TerminalView so user-specified fonts take effect.
    pub(in crate::workspace) font_family: String,
    /// Left dock (lane list, git changes, files — the
    /// active view is picked by `left_dock_view`).
    pub(in crate::workspace) left_dock: gpui::Entity<layout::Dock>,
    /// Active view inside `left_dock`. Persisted via ProjectState.
    pub(in crate::workspace) left_dock_view: daruda_store::project::LeftDockView,
    /// Active tab inside the right dock (Usage / Skills / Tools /
    /// Tasks). Persisted via ProjectState.
    pub(in crate::workspace) right_dock_view: daruda_store::project::RightDockView,
    /// Claude Code integration state — usage / plan-limits / service-
    /// status / session-status / PTY tracker / JSONL fallback +
    /// associated background tasks. Grouped into one struct so the
    /// 18 sub-fields don't clutter `Workspace`'s top level. See
    /// [`claude_session_ops::ClaudeContext`].
    pub(in crate::workspace) claude: claude_session_ops::ClaudeContext,
    /// Runtime projects in this workspace. Zero entries = empty (Welcome
    /// screen). Each project owns its own lanes, reached via
    /// `projects[i].lanes`. `tabs`/`panes` live on the active lane's
    /// `MainAreaContext` slot.
    pub(in crate::workspace) projects: Vec<crate::project::Project>,
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
    /// Bottom dock (terminal panel, problems, output).
    pub(in crate::workspace) bottom_dock: gpui::Entity<layout::Dock>,
    /// Right dock (file explorer, git changes).
    pub(in crate::workspace) right_dock: gpui::Entity<layout::Dock>,
    /// Command palette state (Cmd+Shift+P).
    pub(in crate::workspace) command_palette: command::palette::CommandPaletteState,
    /// Lane switcher state (Cmd+P) — fuzzy quick-switch across every
    /// project's lanes.
    pub(in crate::workspace) lane_switcher: command::lane_switcher::LaneSwitcherState,
    /// Cached git status per (project, lane). Refreshed when the
    /// Git Changes view is activated or after a commit. Only entries
    /// that have been fetched at least once are present; missing =
    /// "not yet loaded".
    pub(in crate::workspace) git_status_cache:
        HashMap<daruda_store::project::LaneRef, crate::lane::git::GitStatusData>,
    /// Left-dock Files view state — per-lane lazy tree, watcher,
    /// gitignore matcher, scroll handle, keyboard cursor. Grouped
    /// into one struct so the 12 sub-fields don't clutter
    /// `Workspace`'s top level. See [`left_dock::file_tree_context::FileTreeContext`].
    pub(in crate::workspace) file_tree: left_dock::file_tree_context::FileTreeContext,
    /// Single source of truth for live-reloaded config mirror state.
    /// `apply_config` is the only update site (other than per-field
    /// toggle methods like `toggle_files_show_hidden`).
    pub(in crate::workspace) mirrors: ConfigMirrors,
    /// Per-lane "git status currently running" guard. Watcher
    /// events can arrive faster than `git status` can complete on a
    /// large repo; this keeps at most one in-flight task per lane.
    pub(in crate::workspace) git_status_in_flight: HashSet<daruda_store::project::LaneRef>,
    /// Set of lanes that asked for a status refresh while one was
    /// already running. Drained by the in-flight task on completion,
    /// re-firing once to capture intervening changes.
    pub(in crate::workspace) git_status_pending_repeat: HashSet<daruda_store::project::LaneRef>,
    /// Scroll handle for the Git Changes file list — shared with the scrollbar overlay.
    pub(in crate::workspace) git_changes_scroll_handle: gpui::ScrollHandle,
    /// Scroll handle for the right-dock panel body — shared with the
    /// scrollbar overlay. Used by every right-panel tab (Usage / Skills
    /// / Tools / Tasks) that wraps its body in `overflow_y_scroll`.
    pub(in crate::workspace) right_panel_scroll_handle: gpui::ScrollHandle,
    /// Commit-message input panel rendered in the Git Changes footer.
    pub(in crate::workspace) git_commit_input: gpui::Entity<crate::ui::InputPanel>,
    /// Keeps the InputPanel subscription alive for the lifetime of Workspace.
    _git_commit_subscription: gpui::Subscription,
    /// Inline status-bar message surface for the **pane-spawn** failure
    /// path (`report_pane_error`). All git / hooks / mcp / skills / files
    /// ops route through the 3-layer error pipeline (toast → details
    /// modal → NDJSON log) via `report_error`; do not reach for
    /// `last_error` for those. New IO/exec failures belong in the
    /// pipeline.
    pub(in crate::workspace) last_error: Option<gpui::SharedString>,
    /// Recent surfaced [`ErrorReport`]s, newest-first. Capped at 50
    /// entries — older reports drop out as new ones land. Long-tail
    /// archive that survives toast dismissal so the "Show recent
    /// errors" command palette entry can read it.
    pub(in crate::workspace) error_history:
        Vec<daruda_store::observability::error_report::ErrorReport>,
    /// Toast notification layer — owns the live queue, the 1 Hz expiry
    /// sweep, and the overlay renderer. Isolated into its own entity so
    /// a toast change triggers only the toast layer's repaint, not a
    /// full Workspace repaint.
    pub(in crate::workspace) toast_layer: gpui::Entity<toast_layer::ToastLayer>,
    /// Most recently observed window bounds (position + size). Updated by
    /// the `observe_window_bounds` callback so `save_state()` can
    /// persist the live window geometry without taking `Window` as a
    /// parameter.
    pub(in crate::workspace) cached_window_bounds: Option<daruda_store::project::WindowState>,
    /// Memoized status-bar "project config layer exists" flag, keyed by the
    /// active project root. `render()` runs every frame, so statting per
    /// render burned syscalls; recompute only when the root changes.
    /// `reload_config` clears it so a freshly created layer surfaces
    /// immediately.
    pub(in crate::workspace) cached_project_config: Option<(std::path::PathBuf, bool)>,
    /// User-set window title (Window > Edit Window Title…). When
    /// `Some`, replaces the auto-derived `"<pane title> — <cwd>"`
    /// string passed to `window.set_window_title`. Persisted to
    /// `ProjectState.window_user_label`.
    pub(in crate::workspace) window_user_label: Option<gpui::SharedString>,
    /// Effective shell program for new panes — `Some` only when a
    /// project layer (or the user `[shell]` section) sets `program`.
    /// `None` falls back to `$SHELL` / `/bin/zsh` via `PtyConfig::default`.
    /// Picked up by `create_pane_with_cwd` at spawn time; existing
    /// panes keep the program they were spawned with.
    pub(in crate::workspace) shell_program: Option<String>,
    /// Syntect theme name for syntax highlighting in the file viewer.
    /// Updated on every config reload; threaded into background load tasks.
    pub(in crate::workspace) syntax_theme: String,
    /// When true, clicking a file in the left dock reuses the single
    /// existing file-viewer tab instead of opening one per file.
    /// Mirrors `daruda_config::FileViewerConfig::preview_tab`.
    pub(in crate::workspace) file_viewer_preview_tab: bool,
    /// Notification + user-attention gates. Drives whether OSC 9 / 777 /
    /// 1337 RequestAttention surface to the OS. Read by per-pane
    /// `TerminalViewEvent` subscriptions and by the long-running command
    /// timer.
    pub(in crate::workspace) notifications: daruda_config::NotificationsConfig,
    /// Telegram bot bridge settings — gates `relay_to_telegram` (both
    /// `enabled` and a completed pairing are required before a ping is
    /// queued). Mirrored from the live config the same way
    /// `notifications` is, so a Settings-window toggle takes effect
    /// without any extra plumbing.
    pub(in crate::workspace) telegram: daruda_config::TelegramConfig,
    /// Clipboard write limits — caps streaming OSC 1337 `Copy=` /
    /// `EndCopy` payloads so a runaway shell cannot exhaust memory.
    pub(in crate::workspace) clipboard: daruda_config::ClipboardConfig,
    /// Agent chat configuration — permission mode applied on connect.
    pub(in crate::workspace) agent: daruda_config::AgentConfig,
    /// The agent catalog mirrored from config `[[agents]]`, already resolved
    /// (`Config::resolved_agents`) — preset references expanded, entries that
    /// resolve to nothing dropped. A newly opened pane runs under `agents[0]`;
    /// each pane resolves its `agent_id` to a launch command here at connect
    /// time. Guaranteed non-empty by the config layer.
    pub(in crate::workspace) agents: Vec<daruda_config::AgentDefinition>,
    /// The registered SSH/Docker host catalog mirrored from config
    /// `[[session_hosts]]` — a lane's `session_host.registry_id` resolves
    /// against this via `lane::session_host::effective_session_host`.
    pub(in crate::workspace) session_hosts: Vec<daruda_config::SessionHostEntry>,
    /// Removed catalog rows mirrored from config `[[session_host_tombstones]]`
    /// — chased when a `registry_id` no longer resolves in `session_hosts`,
    /// so a merge (`redirected_to`) still re-resolves. See
    /// `lane::session_host::effective_session_host`.
    pub(in crate::workspace) session_host_tombstones: Vec<daruda_config::SessionHostTombstone>,
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
    /// Presence-gated Telegram pings held back while the user is at this
    /// window, keyed by the firing pane; drained by `flush_deferred_telegram`.
    pub(in crate::workspace) deferred_telegram: std::collections::HashMap<
        main_area::pane_tree::PaneId,
        Vec<main_area::agent_chat_pane::telegram_ops::DeferredRelay>,
    >,
    /// True while a git commit or push operation is running. Prevents
    /// duplicate submissions when the user double-clicks Commit/Push.
    pub(in crate::workspace) git_op_in_flight: bool,
    /// Commit split button mode (Normal vs Amend). In `Amend` the primary
    /// button reads "Amend" (drives `git commit --amend`) and the dropdown
    /// reads "Cancel Amend". Entered via the dropdown's "Amend Last Commit",
    /// left on success / cancel / lane switch. Tied to the active lane — the
    /// prefilled message belongs to that lane's HEAD.
    pub(in crate::workspace) commit_mode: CommitMode,
    /// True while a staging operation (git add / restore-staged / add-all) is
    /// running. Separate from `git_op_in_flight` so a stage click doesn't
    /// block the commit button and vice versa.
    pub(in crate::workspace) git_stage_in_flight: bool,
    /// Per-lane set of collapsed dir groups in the Git Changes view.
    /// Keyed by the lane-relative dir string emitted by `group_by_dir`
    /// (e.g. `"src/workspace/left_dock"`). In-memory only — collapse state
    /// resets on app restart by design.
    pub(in crate::workspace) git_collapsed_dirs:
        std::collections::HashMap<daruda_store::project::LaneRef, HashSet<String>>,
    /// Per-lane keyboard cursor in the Git Changes view, stored as
    /// the file's repo-root-relative path (the same shape git status
    /// porcelain emits, so it round-trips into stage/unstage/diff ops
    /// without conversion). Path-keyed (not index-keyed) so refreshes
    /// that re-sort the list keep the cursor on the same file.
    pub(in crate::workspace) git_changes_cursor:
        std::collections::HashMap<daruda_store::project::LaneRef, std::path::PathBuf>,
    /// Focus handle for the Git Changes panel body. Bound to
    /// `key_context("GitChanges")` so the four arrow / Space / Enter
    /// keybindings only fire when the panel holds focus — otherwise
    /// they fall through to terminal panes.
    pub(in crate::workspace) git_changes_panel_focus: gpui::FocusHandle,
    /// Directory where state files are written by `persist_state()`.
    /// Injected at construction time: production passes `default_data_dir()`,
    /// tests pass a per-test temp directory.
    pub(in crate::workspace) data_dir: std::path::PathBuf,
    /// Customizable bottom-dock panels — user-managed tabs of macro
    /// widgets. Loaded from `panels.json` on construction (or seeded
    /// with Claude/Codex/Gemini on first launch). Mutations route
    /// through `main_area::bottom_dock::macro_ops` and persist via
    /// `main_area::bottom_dock::macro_ops::save_panels`.
    pub(in crate::workspace) panels: daruda_store::panels::PanelsState,
    /// Managed accounts across every auth domain — the catalog a pane's
    /// `AccountSelection` resolves against, plus the per-domain default seeded
    /// onto a freshly-created pane (see
    /// `Workspace::default_account_selection_for_new_pane`); resolution itself
    /// never falls back to that default, so an explicit `SystemDefault` always
    /// means the ambient environment.
    ///
    /// Read-cache of the app-wide [`accounts_global::AccountsGlobal`] (the
    /// single source of truth): seeded at construction, refreshed *only* by
    /// `_accounts_global_subscription`. Cached as a field so the many cx-free
    /// read sites (`main_area::pane::resolve_pane_account` callers,
    /// `focused_account`, the status-bar slot) stay cx-free, exactly like the
    /// config fields cache `SettingsStore`.
    pub(in crate::workspace) accounts: daruda_store::accounts::AccountsState,
    /// In-flight headless add-account login (Plan B), if any — see
    /// [`PendingLogin`]. Drives the add-account spinner/cancel affordance;
    /// `None` outside of `Workspace::add_managed_account`'s call through
    /// `Workspace::finish_login` / `Workspace::cancel_pending_login`.
    pub(in crate::workspace) pending_login: PendingLogin,
    /// Subscription that refreshes the `accounts` read-cache and repaints
    /// whenever the app-wide [`accounts_global::AccountsGlobal`] changes —
    /// so an add/reauth/default/delete in *any* window (or the Settings
    /// window) is reflected here immediately, with no manual broadcast.
    _accounts_global_subscription: gpui::Subscription,
    /// Subscription that calls `cx.notify()` whenever the app-wide
    /// `GlobalTasks` changes — so the Tasks tab in this workspace
    /// re-renders after a CRUD or lifecycle mutation triggered by any
    /// other workspace, hook, or modal.
    _tasks_global_subscription: gpui::Subscription,
    /// Background tick that re-renders the right-panel Tasks tab
    /// every [`crate::ui::theme::RIGHT_PANEL_TASK_LIVE_TICK_MS`]
    /// while at least one task is `Running`, so the pulse dot animates
    /// and the inline duration text advances. `None` when no
    /// `Running` row is on screen — the loop self-terminates and
    /// gets re-spawned by `ensure_task_live_tick` on the next
    /// state-change event. See `task_ops::spawn_task_live_tick`.
    pub(in crate::workspace) _task_live_tick: Option<gpui::Task<()>>,
    /// Active filter shown in the Tasks tab header. Default = `All`.
    pub(in crate::workspace) task_filter: daruda_store::tasks::TaskFilter,
    /// Per-repo lock that prevents two concurrent `start_task`
    /// invocations from racing on `git worktree add` against the same
    /// repository. Cleared after `finalize_create_lane` returns.
    pub(in crate::workspace) pending_lane_creates: HashSet<std::path::PathBuf>,
    /// Re-entry guard for the platform `on_window_should_close`
    /// callback. Set to `true` while the batch close prompt is
    /// awaiting the user's answer; cleared once the answer lands or
    /// the workspace is dropped.
    pub(in crate::workspace) window_close_in_flight: bool,
    /// Multi-line text input bound to the built-in "Input" bottom panel.
    /// Sends typed text verbatim to the focused terminal pane on
    /// Cmd+Enter or the inline Submit button click. The send button and
    /// drop chrome live alongside the input in
    /// `workspace/bottom/terminal_input.rs` rather than wrapping the
    /// state in an `InputPanel` composite (commit input still uses
    /// `InputPanel` for its dropdown + floating-bar layout).
    pub(in crate::workspace) terminal_input: gpui::Entity<crate::ui::InputState>,
    /// Keeps the terminal_input subscription alive for the lifetime of
    /// the Workspace entity.
    _terminal_input_subscription: gpui::Subscription,
    /// Cached line count of `terminal_input`. Updated on every
    /// `InputEvent::Change` so `adapt_dock_to_input_lines` can guard
    /// against redundant resizes without re-reading the entity on every
    /// render cycle.
    pub(in crate::workspace) terminal_input_line_count: usize,
    /// When true, the bottom dock shows the built-in "Input" panel
    /// instead of the active macro tab.
    pub(in crate::workspace) terminal_input_visible: bool,
    /// Search query input rendered atop the right-bar Skills tab.
    /// Cleared on `Esc`; substring-filters Project / Personal / Plugin
    /// scopes simultaneously. The renderer reads the current text via
    /// `RightDockSnapshot::skill_search_query` (captured per frame) so the
    /// panel render closure never re-enters the workspace.
    pub(in crate::workspace) skill_search_input: gpui::Entity<crate::ui::InputState>,
    /// Search query input rendered atop the right-bar Tasks tab. Same
    /// pattern as `skill_search_input` — substring-filters task rows
    /// over `title / prompt / notes / branch_name`. The renderer reads
    /// the current text via `RightDockSnapshot::task_search_query`
    /// (captured per frame).
    pub(in crate::workspace) task_search_input: gpui::Entity<crate::ui::InputState>,
    /// Plugin ids (`<plugin>@<marketplace>`) whose accordion section
    /// in the right-bar Skills tab is currently expanded. Default
    /// (empty set) means every plugin group renders collapsed; the
    /// user toggles individual groups via the accordion chevron.
    pub(in crate::workspace) skill_plugin_expanded: std::collections::HashSet<String>,
    /// Skills filesystem watcher — drops on shutdown / re-spawn so
    /// the FSEvent subscription unregisters cleanly. Updates land in
    /// the app-wide `SkillsState` Global (registered by
    /// `agent::skills::global::init`).
    _skills_watcher: Option<crate::hooks::skills_watcher::SkillsWatcherHandle>,
    _skills_event_pump: Option<gpui::Task<()>>,
    /// Subscription that calls `cx.notify()` whenever the `SkillsState`
    /// Global changes — so panels in this workspace re-render after a
    /// mutation triggered by another workspace's watcher or by the
    /// Settings window's plugin install / uninstall flow.
    _skills_global_subscription: gpui::Subscription,
    /// MCP filesystem watcher — drops on shutdown / re-spawn so the
    /// FSEvent subscription unregisters cleanly. Updates land in the
    /// app-wide `McpState` Global (registered by
    /// `agent::mcp::global::init`).
    _mcp_watcher: Option<crate::hooks::mcp_watcher::McpWatcherHandle>,
    _mcp_event_pump: Option<gpui::Task<()>>,
    /// Cached Project-scope `.mcp.json` directories (lane root + the
    /// focused cwd, each walked up to its git repo root). Recomputed
    /// only inside `respawn_mcp_watcher` — the render snapshot reads
    /// this field instead of stat-walking the filesystem every frame.
    mcp_project_dirs: Vec<std::path::PathBuf>,
    /// Subscription that calls `cx.notify()` whenever the `McpState`
    /// Global changes — so panels in this workspace re-render after a
    /// mutation triggered by another workspace's watcher or by a
    /// Settings-window action.
    _mcp_global_subscription: gpui::Subscription,
    /// Subscription on the `SettingsStore` Global. Re-resolves the
    /// effective config (user layer + this workspace's project
    /// overlay) and calls `apply_config` on every change.
    _settings_global_subscription: gpui::Subscription,
    /// Subscription on the `Updater` entity. Fires on every status
    /// transition; the handler toasts only when it becomes `Available`.
    /// `None` when no `Updater` global is registered (e.g. tests).
    _updater_subscription: Option<gpui::Subscription>,
    /// Last version surfaced via the "update available" toast. Guards
    /// against re-toasting the same version on repeated `Available`
    /// notifies.
    last_update_toast_version: Option<semver::Version>,
    /// Handle to this workspace's GPUI window. Stored so that methods
    /// called from `observe_global` (which has no `&mut Window`) can
    /// re-enter the window via `cx.update_window` when they need to
    /// update widgets whose setters require `&mut Window`.
    pub(in crate::workspace) window_handle: gpui::AnyWindowHandle,
    /// Per-lane bottom-dock input history. Keyed by `LaneRef` (same as
    /// every other per-lane workspace cache) to prevent cross-project
    /// collisions: `LaneId` is a per-project monotonic counter, so two
    /// different projects can share the same raw id. Populated by
    /// `send_terminal_input`; navigated via the ↑/↓
    /// `on_history_navigate` hook installed on `terminal_input`.
    pub(in crate::workspace) input_history: std::collections::HashMap<
        daruda_store::project::LaneRef,
        crate::lane::history::HistoryBuffer,
    >,
    /// Per-pane unsent draft text for the shared bottom-dock input. Keyed
    /// by the workspace-global `PaneId` of the input-capable pane
    /// (Terminal / AgentChat) the text was typed for. Swapped in/out by
    /// `set_focused_pane` on every focus change: the outgoing pane's text
    /// is saved to its `input_owner` and the incoming pane's draft is
    /// restored, so each pane keeps its own in-progress text. Empty
    /// strings are never stored — `remove` is used instead to avoid
    /// leaking entries for empty drafts or closed panes.
    pub(in crate::workspace) input_drafts:
        std::collections::HashMap<main_area::pane_tree::PaneId, String>,
    /// The input-capable pane whose draft is currently visible in
    /// `terminal_input` — the last-focused Terminal / AgentChat pane.
    /// Draft saves key off this rather than the outgoing `focused_pane_id`,
    /// so text typed while a non-input pane (File / TaskEdit) held focus
    /// still saves to the pane it was meant for on the next input-pane
    /// focus. `None` until the first input-capable pane is focused.
    pub(in crate::workspace) input_owner: Option<main_area::pane_tree::PaneId>,
    /// Status of the latest listening-TCP-port scan. Starts as Pending
    /// before the first scan tick lands, then tracks whether the scanner
    /// produced rows or was unavailable on the current platform/runtime.
    pub(in crate::workspace) port_scan_status: sync::ports::PortScanStatus,
    /// Latest listening-TCP-port scan, attributed to a lane where
    /// possible. Refreshed by `sync::ports`'s background poll loop
    /// (`set_scanned_ports`); read by the status bar's Ports segment
    /// snapshot builder.
    pub(in crate::workspace) attributed_ports: Vec<sync::ports::PortEntry>,
    /// Background listening-port scan loop for the status bar's Ports
    /// segment. Dropping it (Workspace teardown) cancels the loop.
    #[allow(dead_code)]
    _ports_pump: gpui::Task<()>,
}

impl Workspace {
    #[allow(dead_code)]
    pub fn new(
        config: &daruda_config::Config,
        data_dir: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::new_with_project(config, None, data_dir, window, cx)
    }

    pub fn new_with_project(
        config: &daruda_config::Config,
        project: Option<daruda_store::project::Project>,
        data_dir: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::new_with_project_impl(config, project, data_dir, window, cx, false)
    }

    /// Test-only constructor that skips the heavy runtime systems:
    /// initial tab + PTY spawn, JSONL/skills/MCP filesystem watchers,
    /// `tasks_global::load_from_dir`, `persist_state`, macro-shortcut
    /// registration, and `WindowRegistry` register. Tests that exercise
    /// any of those must opt in by calling the matching `refresh_*` /
    /// `add_tab` / `persist_state` method after construction.
    ///
    /// Rationale: 8-thread parallelism × (3 notify FS watchers + PTY
    /// shell fork/exec + sync disk I/O) is what makes the suite take
    /// ~60 s instead of <30 s.
    #[cfg(test)]
    pub fn new_with_project_for_test(
        config: &daruda_config::Config,
        project: Option<daruda_store::project::Project>,
        data_dir: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::new_with_project_impl(config, project, data_dir, window, cx, true)
    }

    /// Variant of [`Self::new_with_project_for_test`] that additionally
    /// runs the two pieces of heavy init that persistence + lane
    /// tests assume: opening the initial tab and writing the first
    /// `persist_state` snapshot. Other heavy work (FS watchers, macro
    /// shortcut registration, `WindowRegistry`) is still skipped.
    #[cfg(test)]
    pub fn new_with_project_for_test_full(
        config: &daruda_config::Config,
        project: Option<daruda_store::project::Project>,
        data_dir: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut ws = Self::new_with_project_impl(config, project, data_dir, window, cx, true);
        ws.add_tab(window, cx);
        // `persist_state` snapshots `cached_window_bounds`; production
        // captures them between `install_window_close_hook` and the
        // first persist. Mirror that here so the saved state has real
        // geometry instead of `None`.
        ws.capture_window_bounds(window);
        ws.persist_state(cx); // lint-reentrant-reads: test-only constructor, no dock entity is in EntityState::Mut
        ws
    }

    fn new_with_project_impl(
        config: &daruda_config::Config,
        project: Option<daruda_store::project::Project>,
        data_dir: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
        for_test: bool,
    ) -> Self {
        // Layer the project-local override (currently `[shell]` only)
        // on top of the user-global config so the workspace boots with
        // the right shell program for its project.
        let effective = config_ops::effective_config_for(project.as_ref(), config, &data_dir);
        let config = &effective;

        // Ensure app-wide Globals exist before any constructor code
        // pokes them. Production paths register these in `main.rs`;
        // test fixtures that build a Workspace directly (without
        // `init_gpui_component`) still need them for
        // `refresh_skills_watcher` / `refresh_mcp_watcher`. The
        // `Theme` Global is also needed at first paint by the dock's
        // TabBar and by `theme::current(cx)` reads inside any
        // wrapped-widget render path.
        crate::ui::theme::init_if_missing(cx);
        crate::agent::skills::global::init(cx);
        crate::agent::mcp::global::init(cx);
        crate::ui::theme::DarudaTheme::init(cx);
        crate::settings_store::SettingsStore::init(cx);

        let focus_handle = cx.focus_handle();
        let ws_weak = cx.entity().downgrade();
        let toast_layer = cx.new(|_| toast_layer::ToastLayer::new(ws_weak.clone()));

        // Commit-message input panel + subscription must be created before the
        // struct literal so both can reference each other without a borrow
        // conflict.  The subscription is stored as a struct field so it
        // lives for the entire lifetime of the Workspace entity.
        let ws_commit = ws_weak.clone();
        let ws_amend = ws_weak.clone();
        let git_commit_input = cx.new(|cx| {
            crate::ui::InputPanel::new(crate::ui::InputPanelLayout::ActionsFloating, window, cx)
                .with_placeholder(
                    crate::surface::strings::git_commit_placeholder(),
                    window,
                    cx,
                )
                .with_borderless(cx)
                .with_focus_ring(false, cx)
                .with_action(
                    crate::ui::PanelAction::new(
                        "commit",
                        crate::surface::strings::git_commit_btn(),
                        crate::ui::PanelActionVariant::Primary,
                        move |_, window, cx| {
                            let _ = ws_commit.upgrade().map(|w| {
                                w.update(cx, |ws, cx| {
                                    ws.on_commit_changes(&CommitChanges, window, cx)
                                })
                            });
                        },
                    )
                    .with_dropdown_item(
                        crate::surface::strings::ctx_git_commit_amend(),
                        move |window, app_cx| {
                            if let Some(ws) = ws_amend.upgrade() {
                                ws.update(app_cx, |ws, cx| ws.on_commit_amend(window, cx));
                            }
                        },
                    ),
                )
        });
        let git_commit_sub = cx.subscribe_in(
            &git_commit_input,
            window,
            |this, _, ev: &crate::ui::InputPanelEvent, window, cx| match ev {
                crate::ui::InputPanelEvent::Submit => {
                    this.on_commit_changes(&CommitChanges, window, cx);
                }
                crate::ui::InputPanelEvent::Changed => {}
            },
        );

        let ws_for_input = ws_weak.clone();
        let ws_for_tab = ws_weak.clone();
        let ws_for_escape = ws_weak.clone();
        let ws_for_accept = ws_weak.clone();
        let ws_for_provider = ws_weak.clone();
        let ws_for_history = ws_weak.clone();
        let terminal_input = cx.new(|cx_state| {
            let max_rows = usize::from(config.agent.input_max_rows);
            let mut state = crate::ui::InputState::new(window, cx_state)
                .auto_grow(1, max_rows)
                // While an agent chat pane is focused and the user hasn't opted
                // into modifier-to-send, plain Enter submits and Shift+Enter
                // inserts a newline (Zed's agent-panel default). Evaluated live
                // on each Enter so it tracks focus + config without any sync.
                .submit_on_enter(move |app| {
                    ws_for_input.upgrade().is_some_and(|ws| {
                        let ws = ws.read(app);
                        ws.is_agent_chat_pane(ws.active_runtime().focused_pane_id)
                            && !ws.agent.use_modifier_to_send
                    })
                })
                // Shift+Tab cycles the focused agent pane's session mode (Claude
                // Code's permission-mode cycle). `cycle_agent_mode` returns true
                // only when it actually switched (agent pane focused with ≥2
                // modes); the input then skips outdent. Otherwise it falls
                // through to the default outdent. Evaluated live on each press.
                .on_secondary_tab(move |_window, app| {
                    ws_for_tab.upgrade().is_some_and(|ws| {
                        let focused = ws.read(app).active_runtime().focused_pane_id;
                        ws.update(app, |ws, cx| ws.cycle_agent_mode(focused, cx))
                    })
                })
                // Escape cancels the focused agent pane's activity — the
                // keyboard counterpart of the bottom-dock "Stop" button. Returns
                // true only when the pane was actually busy (so Escape keeps
                // propagating normally otherwise). Fires only as a fallback after
                // the input's own Escape handling (see `InputState::on_escape`),
                // so an open completion menu still closes on the first Escape.
                .on_escape(move |_window, app| {
                    ws_for_escape.upgrade().is_some_and(|ws| {
                        let focused = ws.read(app).active_runtime().focused_pane_id;
                        ws.update(app, |ws, cx| ws.cancel_agent_turn_if_active(focused, cx))
                    })
                })
                // ↑/↓ at the boundary of the multi-line input navigates the
                // per-lane history buffer (shell-style). The hook checks whether
                // navigation is possible (entries exist / cursor is active), then
                // defers the actual `set_value` call. Deferring is the same re-
                // entry guard as `on_completion_accept`: the hook fires inside
                // `InputState`'s update, so we cannot call `terminal_input.update`
                // or `terminal_input.read` synchronously (CLAUDE.md pitfall #5).
                // Reading `ws.read(app).input_history` is fine — Workspace is a
                // different entity from terminal_input.
                .on_history_navigate(move |dir, window, app| {
                    let Some(ws) = ws_for_history.upgrade() else {
                        return false;
                    };
                    // Decide whether to consume before mutating any state.
                    // Reading ws (Workspace) is safe here: we're inside
                    // terminal_input's update, but Workspace is a different entity.
                    if !ws.read(app).history_navigate_possible(dir, app) {
                        return false;
                    }
                    let ws_deferred = ws.downgrade();
                    window.defer(app, move |window, cx| {
                        // SILENT-OK: workspace dropped between key press and defer
                        if let Some(ws) = ws_deferred.upgrade() {
                            ws.update(cx, |ws, cx| ws.do_history_navigate(dir, window, cx));
                        }
                    });
                    true
                })
                // Accepting a slash-command completion routes through the
                // workspace. The accept hook fires inside this `InputState`'s
                // update; `cx.defer_in` would re-lease this same InputState, so
                // `complete_slash_command` -> `send_terminal_input` (which
                // reads/clears `terminal_input`) would re-enter and panic. Defer
                // at window/app level — no entity re-lease (CLAUDE.md pitfall #5).
                .on_completion_accept(move |item, window, cx| {
                    let name = item.label.clone();
                    let ws = ws_for_accept.clone();
                    window.defer(cx, move |window, cx| {
                        if let Some(ws) = ws.upgrade() {
                            ws.update(cx, |ws, cx| ws.complete_slash_command(name, window, cx));
                        }
                    });
                });
            state.set_placeholder(
                crate::surface::strings::bottom_input_placeholder(),
                window,
                cx_state,
            );
            // Feed the native completion menu with ACP slash commands when a
            // chat pane is focused and a `/`-token is being typed.
            state.lsp.completion_provider = Some(std::rc::Rc::new(
                crate::workspace::main_area::bottom_dock::slash_command::SlashCommandProvider {
                    workspace: ws_for_provider,
                },
            ));
            state
        });
        // The bottom-dock body wires the Submit-button click →
        // `send_terminal_input`; this subscription covers the keyboard path.
        // `PressEnter { secondary: true }` is emitted on every *submit* —
        // Cmd+Enter always, plus a plain Enter when the `submit_on_enter`
        // predicate above is active (agent pane focused, modifier-to-send off).
        // Shift+Tab mode cycling is handled directly by the `on_secondary_tab`
        // handler installed above (no event round-trip needed).
        let terminal_input_sub = cx.subscribe_in(
            &terminal_input,
            window,
            |this, _, ev: &crate::ui::InputEvent, window, cx| match ev {
                crate::ui::InputEvent::PressEnter { secondary: true } => {
                    this.send_terminal_input(window, cx);
                }
                crate::ui::InputEvent::Change => {
                    this.adapt_dock_to_input_lines(window, cx);
                }
                _ => {}
            },
        );

        // PTY tracker — single thread polls sysinfo every 3 s for
        // claude descendants of registered panes. The tracker handle
        // is shared with the Workspace (for register/unregister) and
        // the receiver feeds an event-pump task that updates the
        // bindings map. See `workspace/sync/pty.rs`.
        let (pty_tracker, pty_rx) = crate::hooks::pty_tracker::PtyTracker::spawn();
        let pty_event_pump = sync::pty::spawn(pty_rx, cx);

        // Install the app-wide accounts Global from disk if this is the
        // first window (idempotent), then take the mirror from the Global —
        // a later window reads whatever the first installed, since one
        // process = one profile = one `data_dir`.
        accounts_global::install_if_absent(
            cx,
            daruda_store::accounts::load_accounts_in(&data_dir).unwrap_or_default(),
        );
        let accounts = accounts_global::snapshot(cx);
        // Sweep any per-account config dir left behind by a login that
        // was cancelled or crashed before being promoted to a
        // `ManagedAccount` — a shallow, one-shot readdir under
        // `claude-accounts/`, cheap enough to run inline here.
        //
        // `grace = account_login_ops::LOGIN_TIMEOUT`: multi-window is first-class
        // (see `WindowRegistry::for_each_workspace`), so a *different*
        // window's login can still be in flight — its config dir exists but
        // hasn't been promoted to `known_account_ids` yet — while this
        // window's constructor runs this sweep. Sparing anything younger
        // than the login timeout avoids deleting that other window's
        // live login dir out from under it; a genuine orphan (crash, or a
        // timed-out login whose cleanup never ran) is always older than
        // this, since a login can't still be running past its own timeout.
        let known_account_ids: Vec<daruda_store::accounts::AccountId> =
            accounts.accounts.iter().map(|account| account.id).collect();
        daruda_agent::accounts::sweep_orphan_dirs(
            &data_dir,
            &known_account_ids,
            account_login_ops::LOGIN_TIMEOUT,
        );

        let mut ws = Self {
            uuid: daruda_store::project::WorkspaceUuid::new(),
            main_area: main_area::MainAreaContext::default(),
            next_id: 0,
            focus_handle,
            dock_drag: None,
            inherit_cwd: true,
            terminal_config: config_ops::terminal_config_from(config),
            font_family: config.font.family.clone(),
            left_dock: {
                let ws = ws_weak.clone();
                cx.new(|_| {
                    let mut d = layout::Dock::new(layout::DockPosition::Left, ws);
                    d.resize(config.left_dock.left_default_width);
                    d.is_open = !config.left_dock.left_collapsed_by_default;
                    // Register the three left-dock view panels in the same order
                    // as `view_tabs::entries()` so `active_panel` and the
                    // tab strip always agree on which view is shown.
                    d.add_panel(layout::LanesPanel);
                    d.add_panel(layout::GitChangesPanel);
                    d.add_panel(layout::FilesPanel);
                    d
                })
            },
            left_dock_view: daruda_store::project::LeftDockView::default(),
            right_dock_view: daruda_store::project::RightDockView::default(),
            claude: claude_session_ops::ClaudeContext {
                usage_poll: config.usage.poll.clone(),
                usage_by_account: claude_session_ops::PerAccountUsage::default(),
                service_status: std::collections::HashMap::new(),
                sticky_focus_by_recipe: std::collections::HashMap::new(),
                usage_domain_override: None,
                usage_refresh_in_flight: false,
                claude_status: {
                    // Cold restore: load any status files that survived a
                    // previous run. TTL cleanup runs at the same time so
                    // orphans from past crashes don't accumulate.
                    let mut store = daruda_agent::ClaudeStatusStore::new();
                    if config.claude_status.enable
                        && let Ok(dir) = daruda_agent::hooks::status_file::default_dir()
                    {
                        let policy =
                            daruda_agent::hooks::cold_restore::ColdRestorePolicy::from_config_secs(
                                config.claude_status.stale_threshold_secs,
                                config.claude_status.file_ttl_days,
                            );
                        if let Ok(initial) = daruda_agent::hooks::cold_restore::run(&dir, &policy) {
                            store.load_initial(initial);
                        }
                    }
                    store
                },
                claude_status_enabled: config.claude_status.enable,
                stale_threshold_secs: config.claude_status.stale_threshold_secs,
                claude_hooks_installed: crate::hooks::installer::InstallerPaths::from_env()
                    .map(|p| crate::hooks::installer::is_installed(&p))
                    .unwrap_or(false),
                pty_tracker,
                pty_claude_bindings: HashMap::new(),
                _pty_event_pump: pty_event_pump,
                _jsonl_watcher: None,
                _jsonl_event_pump: None,
                tool_use_failure_counts: HashMap::new(),
                last_pushed_notification: HashMap::new(),
                _limits_pumps: sync::limits::spawn(cx),
            },
            // Bootstrap projects for this workspace. When the caller
            // supplies a project root, wrap it in a single runtime
            // `Project` (id `0`) carrying a pure placeholder lane —
            // git discovery is deferred to
            // `reconcile_bootstrapped_lanes` below so window creation
            // never blocks on git CLI. Otherwise the workspace starts
            // with no projects (Welcome path).
            projects: project
                .as_ref()
                .map(|p| {
                    vec![crate::project::Project::bootstrap_placeholder(
                        0,
                        p.root.clone(),
                    )]
                })
                .unwrap_or_default(),
            active: daruda_store::project::LaneRef::default(),
            groups: Vec::new(),
            next_project_id: if project.is_some() { 1 } else { 0 },
            next_group_id: 0,
            window_open_policy: daruda_store::project::WindowOpenPolicy::default(),
            bottom_dock: {
                let ws = ws_weak.clone();
                cx.new(|_| {
                    let mut d = layout::Dock::new(layout::DockPosition::Bottom, ws);
                    d.add_panel(layout::MacrosPanel);
                    d
                })
            },
            command_palette: command::palette::CommandPaletteState::default(),
            lane_switcher: command::lane_switcher::LaneSwitcherState::default(),
            git_status_cache: HashMap::new(),
            file_tree: left_dock::file_tree_context::FileTreeContext {
                file_trees: HashMap::new(),
                files_visible_cache: HashMap::new(),
                file_watchers: HashMap::new(),
                files_reload_queues: HashMap::new(),
                files_watcher_poll: None,
                files_panel_focus: cx.focus_handle(),
                files_selection: None,
                files_gitignore_index: HashMap::new(),
                files_scroll_handle: gpui::UniformListScrollHandle::new(),
            },
            mirrors: ConfigMirrors::from_config(config),
            git_status_in_flight: HashSet::new(),
            git_status_pending_repeat: HashSet::new(),
            git_changes_scroll_handle: gpui::ScrollHandle::new(),
            right_panel_scroll_handle: gpui::ScrollHandle::new(),
            git_commit_input,
            _git_commit_subscription: git_commit_sub,
            last_error: None,
            error_history: Vec::new(),
            toast_layer,
            cached_window_bounds: None,
            cached_project_config: None,
            window_user_label: None,
            shell_program: config.shell.program.clone(),
            syntax_theme: config.file_viewer.syntax_theme.clone(),
            file_viewer_preview_tab: config.file_viewer.preview_tab,
            notifications: config.notifications.clone(),
            telegram: config.telegram.clone(),
            clipboard: config.clipboard.clone(),
            agent: config.agent.clone(),
            agents: config.resolved_agents(),
            session_hosts: config.session_hosts.clone(),
            session_host_tombstones: config.session_host_tombstones.clone(),
            last_agent_id: None,
            agent_pulse_prev: Vec::new(),
            deferred_telegram: std::collections::HashMap::new(),
            git_op_in_flight: false,
            commit_mode: CommitMode::Normal,
            git_stage_in_flight: false,
            git_collapsed_dirs: std::collections::HashMap::new(),
            git_changes_cursor: std::collections::HashMap::new(),
            git_changes_panel_focus: cx.focus_handle(),
            panels: main_area::bottom_dock::macro_ops::load_or_seed_panels(&data_dir),
            accounts,
            pending_login: PendingLogin::None,
            // Managed accounts live in the app-wide `AccountsGlobal`; this
            // subscription refreshes the `accounts` read-cache from it and
            // repaints whenever any window mutates it (single, symmetric
            // cross-window propagation path — see `accounts_global`).
            _accounts_global_subscription: cx.observe_global::<accounts_global::AccountsGlobal>(
                |ws, cx| {
                    ws.accounts = accounts_global::snapshot(cx);
                    cx.notify();
                },
            ),
            // Task data lives in the app-wide `GlobalTasks`; this
            // subscription rebroadcasts mutations into this
            // workspace's render path and re-evaluates whether the
            // live tick (pulse + duration) needs to be running.
            _tasks_global_subscription: cx
                .observe_global::<crate::agent::tasks_global::GlobalTasks>(|ws, cx| {
                    ws.ensure_task_live_tick(cx);
                    cx.notify();
                }),
            _task_live_tick: None,
            task_filter: daruda_store::tasks::TaskFilter::default(),
            pending_lane_creates: HashSet::new(),
            window_close_in_flight: false,
            terminal_input,
            _terminal_input_subscription: terminal_input_sub,
            terminal_input_line_count: 1,
            terminal_input_visible: false,
            skill_search_input: cx.new(|cx_state| {
                crate::ui::InputState::new(window, cx_state)
                    .placeholder(crate::surface::strings::skills_search_placeholder())
            }),
            task_search_input: cx.new(|cx_state| {
                crate::ui::InputState::new(window, cx_state)
                    .placeholder(crate::surface::strings::task_search_placeholder())
            }),
            skill_plugin_expanded: std::collections::HashSet::new(),
            data_dir,
            right_dock: {
                let ws = ws_weak.clone();
                cx.new(|_| {
                    let mut d = layout::Dock::new(layout::DockPosition::Right, ws);
                    d.add_panel(layout::AgentChatPanel);
                    d
                })
            },
            _skills_watcher: None,
            _skills_event_pump: None,
            _skills_global_subscription: cx.observe_global::<crate::agent::skills::SkillsState>(
                |_ws, cx| {
                    // Right dock re-stages + diffs on this notify.
                    cx.notify();
                },
            ),
            _mcp_watcher: None,
            _mcp_event_pump: None,
            mcp_project_dirs: Vec::new(),
            _mcp_global_subscription: cx.observe_global::<crate::agent::mcp::McpState>(
                |_ws, cx| {
                    // Right dock re-stages + diffs on this notify.
                    cx.notify();
                },
            ),
            // Re-resolve the user layer with this workspace's project
            // overlay and reapply whenever `SettingsStore` changes —
            // both the FS watch tick and the Settings-window save
            // fold into the same Global mutation and fanout here.
            _settings_global_subscription: cx
                .observe_global::<crate::settings_store::SettingsStore>(|ws, cx| {
                    let store = crate::settings_store::SettingsStore::global(cx);
                    let lane = ws.active_project().map(|p| p.root.as_path());
                    let effective = store.effective_for(lane);
                    ws.apply_config(&effective, cx);
                }),
            // Observe the Updater entity: it self-notifies on every status
            // transition, so the handler filters for `Available` and toasts
            // once per new version.
            _updater_subscription: crate::update::Updater::get(cx).map(|e| {
                cx.observe(&e, |this: &mut Workspace, updater, cx| {
                    this.on_updater_status_changed(&updater, cx);
                })
            }),
            last_update_toast_version: None,
            window_handle: window.window_handle(),
            input_history: std::collections::HashMap::new(),
            input_drafts: std::collections::HashMap::new(),
            input_owner: None,
            port_scan_status: sync::ports::PortScanStatus::Pending,
            attributed_ports: Vec::new(),
            _ports_pump: sync::ports::spawn(cx),
        };
        // Invariant seed: the active lane's runtime must always exist in
        // `runtimes` so `active_runtime()` (read unconditionally by
        // `render`) never panics — including the Welcome state, where
        // `active` is `LaneRef::default()` and no project is open. Seed
        // it for whatever `active` is, unconditionally: production then
        // populates the first project's lane via `add_tab` below (the only
        // auto-seed left), and Welcome leaves it empty (render shows the
        // welcome screen). Lanes activated later are never auto-seeded —
        // they render the empty-state until the user opens content.
        ws.main_area.runtimes.entry(ws.active).or_default();
        // Test-only short-circuit: every line below this point spawns a
        // background thread (PTY, FS watchers) or performs sync disk I/O
        // (persist_state, tasks_global::load_from_dir). Tests that need
        // any of these must invoke them explicitly after construction;
        // skipping them by default is what brings the suite under 30 s.
        if for_test {
            // Render snapshots read `GlobalTasks`; registering an empty
            // one here costs nothing and prevents
            // "no state of type GlobalTasks exists" panics on the first
            // `cx.notify()` triggered by any test mutation.
            crate::agent::tasks_global::init(cx);
            return ws;
        }
        ws.add_tab(window, cx);
        // After tabs/lanes are seeded, decide whether the JSONL
        // fallback watcher should run for this Workspace and start it.
        ws.refresh_jsonl_watcher(cx);
        // Skills watcher: subscribes to project + personal skill dirs
        // and re-scans on every filesystem change.
        ws.refresh_skills_watcher(cx);
        // MCP watcher: subscribes to ~/.claude/settings.json and the
        // active lane's .mcp.json, re-loading the matching scope on
        // every external edit.
        ws.refresh_mcp_watcher(window, cx);
        // Load the right-panel Tasks tab from this workspace's
        // `data_dir`. Production paths all share the default
        // `~/.config/daruda/`; tests inject a fresh per-test dir.
        crate::agent::tasks_global::load_from_dir(cx, &ws.data_dir);
        // Kick off the pulse / duration tick if `load_from_dir`
        // restored any `Running` task — the `GlobalTasks` observe
        // subscription wouldn't fire here because the set_global call
        // pre-dates the subscription registration above.
        ws.ensure_task_live_tick(cx);
        // Background-detect `default_branch` for the bootstrapped project
        // (if any). `Project::bootstrap` sets it to `None` to avoid
        // blocking the UI thread during window creation; this fires
        // asynchronously and persists the result when it returns.
        ws.reconcile_project_default_branches(cx);
        // Upgrade the construction placeholder lane to the
        // git-discovered list off the UI thread (see
        // `Project::bootstrap_placeholder`).
        ws.reconcile_bootstrapped_lanes(cx);

        cx.observe_window_bounds(window, |this: &mut Workspace, window, cx| {
            this.capture_window_bounds(window);
            this.resize_all_tabs(window, cx);
        })
        .detach();

        // Intercept `Cmd+Q` and red-cross close attempts so dirty
        // TaskEdit panes don't silently disappear. The callback returns
        // `false` to veto the close, spawns the async batch prompt, and
        // then re-issues `window.remove_window()` once the user picks
        // Save all / Discard all.
        Self::install_window_close_hook(ws_weak.clone(), window, cx);
        // Capture initial bounds so first save carries real geometry.
        ws.capture_window_bounds(window);

        // Persist initial state so the project is restorable on next launch.
        ws.persist_state(cx); // lint-reentrant-reads: no dock entity is in EntityState::Mut during construction.

        // Register every macro shortcut from the loaded panels state.
        // Uses the App context (cx derefs to App) — global bindings,
        // last-wins, so two windows registering the same shortcut is
        // harmless.
        main_area::bottom_dock::macro_ops::register_macro_shortcuts(&ws.panels, cx);

        // Register this Workspace in the WindowRegistry so broadcasts
        // and window-lifecycle helpers can find it without iterating
        // all windows. The weak is captured at construction time rather
        // than inside the on_release closure: by the time on_release
        // fires, cx.entity() refers to an entity in the process of being
        // dropped, so pre-building the weak here is the safe pattern
        // (mirrors Zed's WorkspaceStore deregistration).
        let weak = cx.entity().downgrade();
        let window_handle = window.window_handle();
        crate::window_registry::WindowRegistry::register(window_handle, weak.clone(), cx);
        cx.on_release(move |_: &mut Workspace, cx: &mut gpui::App| {
            crate::window_registry::WindowRegistry::deregister(&weak, cx);
        })
        .detach();

        ws
    }

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

    pub(in crate::workspace) fn set_left_dock_view(
        &mut self,
        view: daruda_store::project::LeftDockView,
        cx: &mut Context<Self>,
    ) {
        if self.left_dock_view == view {
            return;
        }
        self.mutate_durable(cx, |ws, _| {
            ws.left_dock_view = view;
        });
        if view == daruda_store::project::LeftDockView::GitChanges {
            let target = self.active;
            self.refresh_git_status(target, cx);
        }
        cx.notify();
    }

    /// Human-readable name for this workspace in the recent list.
    /// Uses the active project's name when available, otherwise the
    /// first project's directory name, appending " +N more" when
    /// the workspace holds multiple projects.
    pub(in crate::workspace) fn recent_display_name(&self) -> String {
        let primary = self
            .projects
            .iter()
            .find(|p| p.id == self.active.project)
            .or_else(|| self.projects.first());

        let label = match primary {
            Some(p) if !p.name.is_empty() => p.name.clone(),
            Some(p) => p
                .root
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "?".into()),
            None => "empty".into(),
        };

        let rest = self.projects.len().saturating_sub(1);
        if rest == 0 {
            label
        } else {
            format!("{label} +{rest} more")
        }
    }

    /// Borrow the currently active project. `None` when the workspace
    /// has no projects (Welcome state).
    pub(in crate::workspace) fn active_project(&self) -> Option<&crate::project::Project> {
        let active = self.active;
        self.projects.iter().find(|p| p.id == active.project)
    }

    /// Mutably borrow the currently active project.
    pub(in crate::workspace) fn active_project_mut(
        &mut self,
    ) -> Option<&mut crate::project::Project> {
        let active = self.active;
        self.projects.iter_mut().find(|p| p.id == active.project)
    }

    /// Borrow the currently active lane (active project's active
    /// lane). `None` when no project is loaded.
    pub(in crate::workspace) fn active_lane(&self) -> Option<&crate::lane::Lane> {
        let id = self.active.lane;
        self.active_project()?.lane(id)
    }

    /// Mutably borrow the active lane.
    #[allow(dead_code)]
    pub(in crate::workspace) fn active_lane_mut(&mut self) -> Option<&mut crate::lane::Lane> {
        let id = self.active.lane;
        self.active_project_mut()?.lane_mut(id)
    }

    /// `true` when the active lane exists *and* its `LaneAvailability` is
    /// not `Present` (i.e. `Missing` or `AccessDenied`). Such a lane renders
    /// the empty-state and must reject pane-spawning actions (new tab /
    /// split) that would root a PTY at the dead path. `false` when there is
    /// no active lane at all (no project / welcome window) — that
    /// legitimately allows tabs.
    pub(in crate::workspace) fn active_lane_is_inaccessible(&self) -> bool {
        self.active_lane()
            .is_some_and(|l| l.availability != crate::lane::availability::LaneAvailability::Present)
    }

    /// `true` when `focused_pane_id` points at a real pane in the active
    /// lane's runtime. Tests pane membership (not equality to a sentinel):
    /// the first real pane gets id 0, the same value as the default, so a
    /// paneless lane (the inaccessible empty-state) is detected only because
    /// `panes` is empty — not by comparing against a magic id.
    pub(in crate::workspace) fn has_focused_pane(&self) -> bool {
        let focused = self.active_runtime().focused_pane_id;
        self.active_runtime().panes.iter().any(|p| p.id == focused)
    }

    /// Resolve a `ProjectId` to its runtime project.
    pub(in crate::workspace) fn project_for(
        &self,
        id: daruda_store::project::ProjectId,
    ) -> Option<&crate::project::Project> {
        self.projects.iter().find(|p| p.id == id)
    }

    pub(in crate::workspace) fn project_for_mut(
        &mut self,
        id: daruda_store::project::ProjectId,
    ) -> Option<&mut crate::project::Project> {
        self.projects.iter_mut().find(|p| p.id == id)
    }

    /// Resolve a `LaneRef` to its runtime lane.
    pub(in crate::workspace) fn lane_for(
        &self,
        target: daruda_store::project::LaneRef,
    ) -> Option<&crate::lane::Lane> {
        self.project_for(target.project)?.lane(target.lane)
    }

    /// Resolve a `LaneRef` to its runtime lane, mutably. Mirror of
    /// [`Self::lane_for`] for the write paths (branch reconcile, kind
    /// updates) that mutate the lane in place.
    pub(in crate::workspace) fn lane_for_mut(
        &mut self,
        target: daruda_store::project::LaneRef,
    ) -> Option<&mut crate::lane::Lane> {
        self.projects
            .iter_mut()
            .find(|p| p.id == target.project)?
            .lane_mut(target.lane)
    }

    /// Borrow the active project's lane list. Empty when the
    /// workspace has no projects (Welcome state).
    pub(in crate::workspace) fn active_lanes(&self) -> &[crate::lane::Lane] {
        self.active_project()
            .map(|p| p.lanes.as_slice())
            .unwrap_or(&[])
    }

    /// `LaneRef` pointing at the workspace's currently active
    /// lane. Convenience for call sites that already know the
    /// active id but want the `LaneRef` form for HashMap lookups.
    pub(in crate::workspace) fn active_ref(&self) -> daruda_store::project::LaneRef {
        self.active
    }

    /// `true` when at least one lane has a Claude session (PTY-bound or
    /// ACP agent chat) in an animating status (anything but `Idle`). Gates
    /// the status-pulse pump so the shared `StatusPulseClock` only
    /// repaints windows that actually show motion — idle windows stay at
    /// zero redraws.
    pub(crate) fn has_animating_claude_status(&self, cx: &gpui::App) -> bool {
        if !self.claude.claude_status_enabled {
            return false;
        }
        // ACP pane statuses across every lane (see `agent_chat_statuses`),
        // so a parked lane with a live agent turn still keeps the pulse
        // running.
        let acp_statuses = self.agent_chat_statuses(cx);
        if self.claude.pty_claude_bindings.is_empty() && acp_statuses.is_empty() {
            return false;
        }
        // Per-session, not per-lane-aggregate: the aggregate's
        // max-priority collapse would hide a `Connecting` session
        // (priority 0) behind an `Idle` sibling (priority 1) and stop
        // the pulse while that Connecting badge still animates in the
        // sub-row. Only a pane set where every bound session is `Idle`
        // is fully at rest.
        let index = self.pane_lane_index();
        crate::workspace::claude_status_aggregate::any_pane_session_animating(
            &index,
            &self.claude.pty_claude_bindings,
            &self.claude.claude_status,
            &acp_statuses,
        )
    }

    /// Display name of the currently active project. `None` when the
    /// workspace has no projects.
    pub(crate) fn active_project_name(&self) -> Option<String> {
        self.active_project().map(|p| p.name.clone())
    }

    /// Active lane's branch status, split into the three cases
    /// the status bar / window title care about: attached git branch,
    /// detached HEAD, or non-git default kind. Returned as a small
    /// enum so call-site renderers can decide whether to draw an
    /// inline "detached" chip in addition to the textual label.
    pub(in crate::workspace) fn active_branch_status(&self) -> BranchStatus {
        let Some(wt) = self.active_lane() else {
            return BranchStatus::NotGit;
        };
        match &wt.kind {
            daruda_store::project::LaneKind::Git {
                branch: Some(b), ..
            } => BranchStatus::Branch(b.clone()),
            daruda_store::project::LaneKind::Git { branch: None, .. } => BranchStatus::Detached,
            _ => BranchStatus::NotGit,
        }
    }

    /// Text portion of the status-bar project-branch slot. Returns
    /// `<project>/<branch>` for attached branches and `<project>`
    /// otherwise; the detached state is signalled separately by an
    /// inline chip ([`StatusBarData::is_detached`]), not folded into
    /// this string.
    pub(in crate::workspace) fn active_project_branch_label(&self) -> Option<String> {
        self.project_branch_label_with("/", false)
    }

    /// Window title fallback (`window_user_label` overrides this).
    /// Folds the detached marker into the text — macOS window titles
    /// are plain strings, so the chip cannot ride along the way it
    /// does in the status bar.
    pub(in crate::workspace) fn window_title_label(&self) -> Option<String> {
        self.project_branch_label_with(" · ", true)
    }

    fn project_branch_label_with(&self, sep: &str, mark_detached_inline: bool) -> Option<String> {
        let project = self.active_project()?;
        Some(match self.active_branch_status() {
            BranchStatus::Branch(b) => format!("{}{sep}{}", project.name, b),
            BranchStatus::Detached if mark_detached_inline => {
                format!("{} (detached)", project.name)
            }
            BranchStatus::Detached | BranchStatus::NotGit => project.name.clone(),
        })
    }

    /// True when any project in this workspace already has `root` as
    /// its checkout. Used by the `AddHere` policy path so opening the
    /// same folder twice in the same window focuses the existing
    /// window instead of registering a duplicate runtime project.
    /// Scoped to this window only — Policy B explicitly permits the
    /// same root across separate windows. Compares both as-given and
    /// canonicalized so the symlinked `/tmp` vs `/private/tmp`
    /// flavours on macOS still match.
    pub(crate) fn has_project_root(&self, root: &std::path::Path) -> bool {
        let canonical = std::fs::canonicalize(root).ok();
        self.projects.iter().any(|p| {
            if p.root == root {
                return true;
            }
            let p_canon = std::fs::canonicalize(&p.root).ok();
            match (canonical.as_ref(), p_canon.as_ref()) {
                (Some(a), Some(b)) => a == b,
                _ => false,
            }
        })
    }

    /// Read-only slice of every runtime project currently in this
    /// workspace. Used by menu / palette code that needs to list project
    /// metadata without going through `WorkspaceState`.
    #[allow(dead_code)]
    pub(crate) fn projects(&self) -> &[crate::project::Project] {
        &self.projects
    }

    /// Invalidate the right dock's `.cached()` element so it re-renders
    /// from the freshly staged snapshot. The right dock renders only from
    /// `Workspace::render`'s staged snapshot and never self-notifies for
    /// Workspace-owned state (usage, claude status, the MCP/skills/tasks
    /// globals, tab/filter setters, the status + task-live pulses), so
    /// every mutation of a right-dock source must call this — otherwise
    /// the cache shows stale data (root CLAUDE.md Pitfall #10). Embedded
    /// input entities (the search boxes, the usage dropdown) self-notify
    /// and dirty this dock as an ancestor, so they need no explicit call.
    /// Lease-free (`App::notify`) — dock event listeners run inside a
    /// `Context<Dock>` lease, so leasing the dock here would double-lease.
    pub(crate) fn notify_right_dock(&self, cx: &mut Context<Self>) {
        let dock_id = self.right_dock.entity_id();
        gpui::App::notify(cx, dock_id);
    }

    /// Invalidate the left dock's `.cached()` element directly. The sole
    /// caller is the status pulse, which advances badge animation frames
    /// that are *not* part of `LeftDockSnapshot` (so the staging diff reports
    /// no change and would not refresh the dock).
    /// Lease-free (`App::notify`) — dock event listeners run inside a
    /// `Context<Dock>` lease, so leasing the dock here would double-lease.
    pub(crate) fn notify_left_dock(&self, cx: &mut Context<Self>) {
        let dock_id = self.left_dock.entity_id();
        gpui::App::notify(cx, dock_id);
    }

    /// Refresh both dock badges after an agent session-status change.
    /// One notify re-stages both snapshots; each dock's staging diff
    /// repaints it only on a real change.
    pub(crate) fn notify_status_docks(&self, cx: &mut Context<Self>) {
        cx.notify();
    }

    /// Drive the AgentChat status pulse and paint one final settled frame
    /// for a pane that just left the Working state. Called every
    /// status-pulse tick.
    ///
    /// Scans every lane, not just the active one — a subagent can keep
    /// ticking in a parked lane, and its pane still needs a trailing
    /// settle frame when the run quiesces.
    ///
    /// A busy [`AgentChatView`] is dirtied each tick to animate its pulse
    /// (root CLAUDE.md Pitfall #10). A view that just went idle gets one
    /// more dirty here: its own settling self-notify can miss a cached
    /// view that fell out of the window's tracked set (gpui
    /// `detect_accessed_entities` lost-wakeup, see
    /// `lane_switch_scroll_dead_rootcause`), which would otherwise freeze
    /// its last "running" frame.
    pub(crate) fn pulse_agent_chats(&mut self, cx: &mut Context<Self>) {
        let tick_now = std::time::Instant::now();
        // Collect the candidate panes (clone the entity + its pane id) first so
        // the `self` borrow is dropped before any `view.update` / `self.fire_*`
        // below. A candidate is any pane that was busy at the last tick (so its
        // settle edge is still owed a fire) or could be busy now (`maybe_active`
        // — the cheap O(1) pre-check); idle-and-quiet panes are skipped so the
        // per-tick cost stays bounded.
        type PulseCandidate = (
            main_area::pane_tree::PaneId,
            gpui::Entity<main_area::agent_chat_pane::view::AgentChatView>,
        );
        let candidates: Vec<PulseCandidate> = self
            .main_area
            .runtimes
            .values()
            .flat_map(|rt| rt.panes.iter())
            .filter_map(|p| p.agent_chat_view().map(|v| (p.id, v.clone())))
            .filter(|(_, v)| {
                let vr = v.read(cx);
                vr.activity.was_busy || vr.maybe_active()
            })
            .collect();

        // Reconcile each candidate: an edge fires completion; the post-reconcile
        // busy level drives the repaint set. The pulse tick is the time-driven
        // settle driver — it catches a background subagent's quiescence that no
        // ACP event announces.
        let mut busy_ids: Vec<gpui::EntityId> = Vec::new();
        let mut completions = Vec::new();
        let mut post_turn_relays: Vec<(main_area::pane_tree::PaneId, String)> = Vec::new();
        for (pane_id, view) in &candidates {
            let edge = view.update(cx, |v, _| v.reconcile_activity(tick_now));
            // `reconcile_activity` just recomputed the busy level with `tick_now`
            // and stored it in `was_busy`; read that instead of calling
            // `is_busy()` again (a second O(items) `subagent_activity` scan with a
            // fresh `Instant::now()`) so the whole tick uses one consistent `now`.
            if view.read(cx).activity.was_busy {
                busy_ids.push(view.entity_id());
            }
            if let Some(outcome) = edge {
                completions.push((*pane_id, outcome));
            }
            if let Some(delta) = view.update(cx, |v, _| {
                v.reconcile_post_turn(
                    tick_now,
                    crate::workspace::main_area::agent_chat_pane::view::POST_TURN_QUIESCENCE,
                )
            }) {
                post_turn_relays.push((*pane_id, delta));
            }
        }
        for (pane_id, outcome) in completions {
            self.fire_activity_completion(pane_id, outcome, cx);
        }
        for (pane_id, delta) in post_turn_relays {
            self.relay_post_turn_to_telegram(pane_id, delta, cx);
        }

        // Nothing animating and nothing just settled — fully at rest, no work.
        if busy_ids.is_empty() && self.agent_pulse_prev.is_empty() {
            return;
        }
        // Re-render the workspace so the snapshot re-reads (re-tracks) the
        // cached AgentChat views before we dirty them by id — mirrors the
        // animating path's workspace notify, and is what lets the trailing
        // settled frame reach a view that had fallen out of the tracked set.
        cx.notify();
        for id in busy_ids.iter().chain(self.agent_pulse_prev.iter()) {
            gpui::App::notify(cx, *id);
        }
        self.agent_pulse_prev = busy_ids;
    }

    /// Re-evaluate every pane's deferred Telegram queue against the current
    /// presence signal. Runs on the periodic flush pump; each entry flushes
    /// individually once its own quiet window clears (see `ready_to_deliver`)
    /// rather than the whole map draining at once.
    pub(crate) fn flush_deferred_telegram(&mut self, cx: &mut Context<Self>) {
        if self.deferred_telegram.is_empty() {
            return;
        }
        let quiet_secs = self.telegram.active_idle_secs;
        if !self.telegram.defer_while_active || quiet_secs == 0 {
            self.deliver_deferred_telegram(false, 0.0, quiet_secs, cx);
            return;
        }
        let app_active = crate::platform::attention::is_app_active();
        let idle_secs = crate::platform::attention::system_idle_seconds();
        self.deliver_deferred_telegram(app_active, idle_secs, quiet_secs, cx);
    }

    /// Split out so tests can drive delivery without controlling live OS
    /// presence signals. For each pane's queue: drops permission pings whose
    /// request is no longer the pane's live pending permission, delivers
    /// entries `ready_to_deliver` clears, and re-queues the rest for a later
    /// tick. Skips (and drops) panes that have since closed.
    pub(in crate::workspace) fn deliver_deferred_telegram(
        &mut self,
        app_active: bool,
        idle_secs: f64,
        quiet_secs: u64,
        cx: &mut Context<Self>,
    ) {
        let now = std::time::Instant::now();
        let pending = std::mem::take(&mut self.deferred_telegram);
        for (pane_id, queue) in pending {
            let Some(view) = self.agent_chat_view(pane_id).cloned() else {
                continue;
            };
            let live_perms = view.read(cx).pending_permissions.clone();
            let (ready, still_holding) =
                partition_deferred(queue, &live_perms, now, app_active, idle_secs, quiet_secs);
            for entry in ready {
                self.relay_to_telegram(pane_id, entry.header, entry.tail, entry.permission, cx);
            }
            if !still_holding.is_empty() {
                self.deferred_telegram.insert(pane_id, still_holding);
            }
        }
    }

    pub(in crate::workspace) fn set_right_dock_view(
        &mut self,
        view: daruda_store::project::RightDockView,
        cx: &mut Context<Self>,
    ) {
        if self.right_dock_view == view {
            return;
        }
        self.mutate_durable(cx, |ws, _| {
            ws.right_dock_view = view;
        });
        cx.notify();
    }

    /// Execute the currently focused palette action and close.
    pub(super) fn execute_palette_action(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let action_id = self.command_palette.focused_action_id();
        self.command_palette.close();
        cx.notify();

        if let Some(id) = action_id {
            match id {
                "new_tab" => self.on_new_tab(&NewTab, window, cx),
                "new_task" => self.on_new_task(&NewTask, window, cx),
                "new_agent_chat" => self.on_open_agent_chat(&OpenAgentChat, window, cx),
                "edit_task" => self.on_edit_task(&EditTask, window, cx),
                "start_task" => self.open_task_picker_modal(
                    crate::workspace::right_dock::task_picker_modal::TaskPickAction::Start,
                    window,
                    cx,
                ),
                "cancel_task" => self.open_task_picker_modal(
                    crate::workspace::right_dock::task_picker_modal::TaskPickAction::Cancel,
                    window,
                    cx,
                ),
                "reopen_task" => self.open_task_picker_modal(
                    crate::workspace::right_dock::task_picker_modal::TaskPickAction::Reopen,
                    window,
                    cx,
                ),
                "retry_task" => self.open_task_picker_modal(
                    crate::workspace::right_dock::task_picker_modal::TaskPickAction::Retry,
                    window,
                    cx,
                ),
                "delete_task" => self.open_task_picker_modal(
                    crate::workspace::right_dock::task_picker_modal::TaskPickAction::Delete,
                    window,
                    cx,
                ),
                "close_pane" => self.on_close_pane(&ClosePane, window, cx),
                "close_tab" => self.on_close_tab(&CloseTab, window, cx),
                "split_right" => self.on_split_right(&SplitRight, window, cx),
                "split_down" => self.on_split_down(&SplitDown, window, cx),
                "next_tab" => self.on_next_tab(&NextTab, window, cx),
                "prev_tab" => self.on_prev_tab(&PrevTab, window, cx),
                "toggle_left_dock" => self.on_toggle_left_dock(&ToggleLeftDock, window, cx),
                "toggle_bottom_dock" => self.on_toggle_bottom_dock(&ToggleBottomDock, window, cx),
                "toggle_right_dock" => self.on_toggle_right_dock(&ToggleRightDock, window, cx),
                "toggle_command_palette" => {
                    self.on_toggle_command_palette(&ToggleCommandPalette, window, cx);
                }
                "toggle_lane_switcher" => {
                    self.on_toggle_lane_switcher(&ToggleLaneSwitcher, window, cx);
                }
                "focus_next_pane" => self.on_focus_next_pane(&FocusNextPane, window, cx),
                "focus_prev_pane" => self.on_focus_prev_pane(&FocusPrevPane, window, cx),
                "focus_pane_left" => self.on_focus_pane_left(&FocusPaneLeft, window, cx),
                "focus_pane_right" => self.on_focus_pane_right(&FocusPaneRight, window, cx),
                "focus_pane_up" => self.on_focus_pane_up(&FocusPaneUp, window, cx),
                "focus_pane_down" => self.on_focus_pane_down(&FocusPaneDown, window, cx),
                "move_tab_left" => self.on_move_tab_left(&MoveTabLeft, window, cx),
                "move_tab_right" => self.on_move_tab_right(&MoveTabRight, window, cx),
                "show_left_dock_lanes" => {
                    self.on_show_left_dock_worktrees(&ShowLeftDockLanes, window, cx);
                }
                "show_left_dock_git" => {
                    self.on_show_left_dock_git(&ShowLeftDockGit, window, cx);
                }
                "show_left_dock_files" => {
                    self.on_show_left_dock_files(&ShowLeftDockFiles, window, cx);
                }
                "switch_right_panel_usage" => {
                    self.on_switch_right_panel_usage(&SwitchRightPanelUsage, window, cx);
                }
                "switch_right_panel_skills" => {
                    self.on_switch_right_panel_skills(&SwitchRightPanelSkills, window, cx);
                }
                "switch_right_panel_tools" => {
                    self.on_switch_right_panel_tools(&SwitchRightPanelTools, window, cx);
                }
                "switch_right_panel_tasks" => {
                    self.on_switch_right_panel_tasks(&SwitchRightPanelTasks, window, cx);
                }
                "new_skill" => {
                    self.on_new_skill(&NewSkill, window, cx);
                }
                "copy" | "paste" | "select_all" => {
                    // These are terminal-level actions dispatched via the
                    // focused pane's TerminalView, not workspace actions.
                    // The GPUI action dispatch already handles them when the
                    // terminal has focus, so no-op here is correct.
                }
                "install_claude_hooks" => {
                    self.on_install_agent_hooks(&InstallAgentHooks, window, cx);
                }
                "uninstall_claude_hooks" => {
                    self.on_uninstall_agent_hooks(&UninstallAgentHooks, window, cx);
                }
                "open_command_history" => {
                    self.on_open_command_history(&OpenCommandHistory, window, cx);
                }
                "open_folder" => {
                    window.dispatch_action(Box::new(crate::OpenFolder), cx);
                }
                "close_project" => {
                    window.dispatch_action(Box::new(crate::CloseProject), cx);
                }
                "new_group" => self.on_new_group(&NewGroup, window, cx),
                "rename_project" => {
                    self.on_rename_active_project(&RenameActiveProject, window, cx);
                }
                "move_project_to_group" => {
                    self.on_move_active_project_to_group(&MoveActiveProjectToGroup, window, cx);
                }
                "quit" => cx.quit(),
                _ => {
                    // Per-section settings entries follow the
                    // `open_settings.<slug>` pattern used by the
                    // keybinding-override path. Bare "open_settings"
                    // resolves to the default page.
                    if id == "open_settings" {
                        self.on_open_settings(
                            &OpenSettings(daruda_config::BuiltinSection::default()),
                            window,
                            cx,
                        );
                    } else if let Some(slug) = id.strip_prefix("open_settings.")
                        && let Some(section) = daruda_config::BuiltinSection::from_slug(slug)
                    {
                        self.on_open_settings(&OpenSettings(section), window, cx);
                    }
                }
            }
        }
    }
}
