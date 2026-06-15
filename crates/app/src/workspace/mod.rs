//! Terminology — keep these distinct in code and prose:
//!
//! - `Tab bar`: horizontal strip at the top of the window. One per workspace.
//!   Identifiers: `tab_bar`, `TAB_BAR_HEIGHT`.
//! - `Tab cell`: one entry inside the tab bar. Identifiers: `tab_cell`,
//!   `tab_titles`, `tab-close`.
//! - `Pane`: a leaf in the split tree (PTY + TerminalView). Identifiers:
//!   `pane`, `Pane`, `pane_id`, `PaneId`.
//! - `Pane header`: per-pane title bar shown above a pane in split mode only.
//!   Identifiers: `pane_header`, `PANE_HEADER_HEIGHT`, `pane-hdr`, `pane-close`.
//!
//! Never use bare `header` or `title_bar` — always one of `tab_*` or `pane_*`.
//! `terminal_title()` (daruda_terminal) returns the OSC-2 string used by both
//! tab cells and pane headers; do not rename it back to `tab_title`.

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
pub(in crate::workspace) mod dialog_helpers;
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
mod spawn_helpers;
pub(crate) mod status_bar;
pub(in crate::workspace) mod sync;
#[cfg(test)]
mod tests;
mod toast_layer;
mod window_close_ops;

pub(in crate::workspace) use config_sync::ConfigMirrors;
pub(in crate::workspace) use modal_view::ModalView;
pub(in crate::workspace) use persistence::LaneRuntime;

use std::collections::{HashMap, HashSet};

use gpui::{AppContext, Context, FocusHandle, Window, actions};

use daruda_terminal::TerminalConfig;

// ----------------------------------------------------------------
// Actions
// ----------------------------------------------------------------

/// Parameterized action — fires when a user-defined macro shortcut
/// is pressed. Carries the shortcut string so the handler can resolve
/// it back to the macro at dispatch time (the binding is set up by
/// `main_area::bottom_dock::macro_ops::register_macro_shortcuts`,
/// which can register the same shortcut without recompiling).
///
/// `no_json` skips the keymap-deserialize derive — this action is
/// only constructed at runtime by the registrar, never loaded from a
/// keymap.json file.
#[derive(Clone, PartialEq, Debug, gpui::Action)]
#[action(namespace = workspace, no_json)]
pub struct RunMacroByShortcut(pub gpui::SharedString);

/// Open the Settings window on the requested page.
///
/// Carries [`daruda_config::BuiltinSection`] so each menu / palette
/// entry can land directly on the right page (Font, Keymap, etc.)
/// instead of dropping the user on the General page and forcing a
/// click. `Cmd+,` and the legacy `"open_settings"` config name both
/// resolve to `OpenSettings(BuiltinSection::default())` (= General).
///
/// `no_json` is used because daruda's keybinding override path lives
/// in `surface::action_map`, not GPUI's keymap.json — see the
/// `open_settings.<slug>` matching in `bind!` for the per-section
/// override syntax.
#[derive(Clone, PartialEq, Debug, gpui::Action)]
#[action(namespace = workspace, no_json)]
pub struct OpenSettings(pub daruda_config::BuiltinSection);

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
        InstallClaudeHooks,
        UninstallClaudeHooks,
        MinimizeWindow,
        ZoomWindow,
        ToggleFullScreen,
        EditWindowTitle,
        OpenCommandHistory,
        CloseOtherTabs,
        CloseTabsToRight,
        ToggleZoomPane,
        /// Open a fresh TaskEdit pane in draft mode (no task_id).
        /// Wired from the Command Palette `new_task` entry and
        /// configurable as a keybinding via
        /// `[keybindings] new_task = "cmd-shift-n"` in user config
        /// (m-1 review note).
        NewTask,
        /// Open the task picker filtered to all tasks; the picked
        /// task gets a TaskEdit pane via `open_task_edit_pane`.
        /// Configurable as a keybinding via `[keybindings] edit_task`.
        EditTask,
        /// Prompt for a group name and create an empty group in the
        /// shared (group + ungrouped-project) tab-order pool. Wired
        /// from the Command Palette `new_group` entry.
        NewGroup,
        /// Rename the active workspace project via a single-field
        /// modal. Wired from the Command Palette `rename_project`
        /// entry.
        RenameActiveProject,
        /// Move the active project into a group selected by name
        /// (blank input = ungrouped, unknown name creates a fresh
        /// group). Wired from the Command Palette
        /// `move_project_to_group` entry.
        MoveActiveProjectToGroup,
        /// Save the currently focused file-view pane to disk.
        /// Wired to `cmd-s` in the `FileViewer` focus scope.
        SaveFilePane,
    ]
);

// ----------------------------------------------------------------
// Workspace
// ----------------------------------------------------------------

const TITLE_BAR_HEIGHT: f32 = crate::ui::theme::TITLE_BAR_HEIGHT;
const TAB_BAR_HEIGHT: f32 = crate::ui::theme::TAB_BAR_HEIGHT;

pub struct Workspace {
    /// Stable cross-session identifier — matches the UUID stored on
    /// disk at `workspaces/<uuid>.json`. Minted at construction (or
    /// adopted from a restored [`daruda_store::project::WorkspaceState`]
    /// in a later task) and never changes for the lifetime of the
    /// `Workspace` entity. Read by the v3 persistence path; the legacy
    /// save path still keys by `path_hash`, so the field is currently
    /// unread by anything other than tests.
    #[allow(dead_code)]
    pub(in crate::workspace) uuid: daruda_store::project::WorkspaceUuid,
    /// TabBar + PaneTree runtime state. Holds tabs, panes, focus,
    /// drag/context-menu overlays, and inactive lane runtimes.
    pub(in crate::workspace) main_area: main_area::MainAreaContext,
    next_id: u64,
    focus_handle: FocusHandle,
    /// Dock resize drag — active while the user is pulling on the
    /// right edge of the left dock, the left edge of the right dock,
    /// or the top edge of the bottom dock.
    pub(in crate::workspace) dock_drag: Option<layout::ops::DockDrag>,
    /// Overlay alpha for inactive panes when split (0.0 = no dim).
    /// Tunable knob — first step toward a `Theme` struct (config Phase).
    pub(in crate::workspace) dim_alpha: f32,
    /// When true, new tabs/panes spawn with the focused pane's cwd
    /// (iTerm2 "Reuse previous session's directory"). Future config knob.
    pub(in crate::workspace) inherit_cwd: bool,
    /// Terminal config applied to every new pane. Holds the font
    /// size + iTerm2-style vertical/horizontal spacing multipliers so
    /// all panes share one source of truth; `resize_all_tabs` reads
    /// the same settings to measure cells consistently with the
    /// TerminalView render path.
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
    /// Runtime projects in this workspace. Zero entries = empty
    /// (Welcome screen). One entry = single-project window (current
    /// behaviour); commit c onward allows multiple. Each project owns
    /// its own lanes, so the per-project lane list is reached
    /// via `projects[i].lanes`. `tabs`/`panes` still live on the
    /// active lane's `MainAreaContext` slot until W-3 migrates
    /// them into the runtime struct.
    pub(in crate::workspace) projects: Vec<crate::project::Project>,
    /// Active (project, lane) pair. When `projects` is non-empty,
    /// always points at a live entry — kept normalized by
    /// `activate_lane` and `finalize_remove_*` paths.
    pub(in crate::workspace) active: daruda_store::project::LaneRef,
    /// User-defined groups in the left dock. Each project's
    /// `group_id` links into this list. Empty during the early
    /// multi-project rollout — Group CRUD lands in a later commit.
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
    /// archive that survives toast dismissal so the (Step 6) "Show
    /// recent errors" command palette entry can read it.
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
    /// active project root. `project_config_path` + `Path::exists` are
    /// filesystem stats; `render()` runs on every animation frame (status
    /// badges request frames without `cx.notify`), so statting per render
    /// burned syscalls in a tight loop. Recompute only when the active root
    /// changes; `reload_config` clears it so a freshly created project layer
    /// surfaces immediately.
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
    /// Clipboard write limits — caps streaming OSC 1337 `Copy=` /
    /// `EndCopy` payloads so a runaway shell cannot exhaust memory.
    pub(in crate::workspace) clipboard: daruda_config::ClipboardConfig,
    /// True while a git commit or push operation is running. Prevents
    /// duplicate submissions when the user double-clicks Commit/Push.
    pub(in crate::workspace) git_op_in_flight: bool,
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
    /// callback. Set to `true` while the R-25 batch prompt is
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
    /// Handle to this workspace's GPUI window. Stored so that methods
    /// called from `observe_global` (which has no `&mut Window`) can
    /// re-enter the window via `cx.update_window` when they need to
    /// update widgets whose setters require `&mut Window`.
    pub(in crate::workspace) window_handle: gpui::AnyWindowHandle,
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
        // Layer the project-local override (Phase 1: `[shell]` only)
        // on top of the user-global config so the workspace boots with
        // the right shell program for its project.
        let effective = config_ops::effective_config_for(project.as_ref(), config);
        let config = &effective;

        // Ensure app-wide Globals exist before any constructor code
        // pokes them. Production paths register these in `main.rs`;
        // test fixtures that build a Workspace directly (without
        // `init_gpui_component`) still need them for
        // `refresh_skills_watcher` / `refresh_mcp_watcher`. The
        // `Theme` Global is also needed at first paint by the dock's
        // TabBar (5950ff1) and by `theme::current(cx)` reads inside
        // any wrapped-widget render path.
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

        let terminal_input = cx.new(|cx_state| {
            let mut state = crate::ui::InputState::new(window, cx_state).multi_line(true);
            state.set_placeholder(
                crate::surface::strings::bottom_input_placeholder(),
                window,
                cx_state,
            );
            state
        });
        // Cmd+Enter (`secondary: true`) submits; plain Enter inserts
        // a newline (multi-line mode). The bottom-dock body itself
        // wires the Submit-button click → `send_terminal_input` —
        // this subscription only covers the keyboard path.
        let terminal_input_sub = cx.subscribe_in(
            &terminal_input,
            window,
            |this, _, ev: &crate::ui::InputEvent, window, cx| {
                if let crate::ui::InputEvent::PressEnter { secondary: true } = ev {
                    this.send_terminal_input(window, cx);
                }
            },
        );

        // PTY tracker — single thread polls sysinfo every 3 s for
        // claude descendants of registered panes. The tracker handle
        // is shared with the Workspace (for register/unregister) and
        // the receiver feeds an event-pump task that updates the
        // bindings map. See `workspace/sync/pty.rs`.
        let (pty_tracker, pty_rx) = crate::hooks::pty_tracker::PtyTracker::spawn();
        let pty_event_pump = sync::pty::spawn(pty_rx, cx);

        let mut ws = Self {
            uuid: daruda_store::project::WorkspaceUuid::new(),
            main_area: main_area::MainAreaContext::default(),
            next_id: 0,
            focus_handle,
            dock_drag: None,
            dim_alpha: render::DEFAULT_INACTIVE_PANE_DIM_ALPHA,
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
                plan_limits: daruda_claude::PlanLimits::default(),
                service_status: daruda_claude::ServiceStatus::default(),
                activity: daruda_claude::ActivityStats::default(),
                usage_refresh_in_flight: false,
                claude_status: {
                    // Cold restore: load any status files that survived a
                    // previous run. TTL cleanup runs at the same time so
                    // orphans from past crashes don't accumulate.
                    let mut store = daruda_claude::ClaudeStatusStore::new();
                    if config.claude_status.enable
                        && let Ok(dir) = daruda_claude::hooks::status_file::default_dir()
                    {
                        let policy =
                            daruda_claude::hooks::cold_restore::ColdRestorePolicy::from_config_secs(
                                config.claude_status.stale_threshold_secs,
                                config.claude_status.file_ttl_days,
                            );
                        if let Ok(initial) = daruda_claude::hooks::cold_restore::run(&dir, &policy)
                        {
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
            clipboard: config.clipboard.clone(),
            git_op_in_flight: false,
            git_stage_in_flight: false,
            git_collapsed_dirs: std::collections::HashMap::new(),
            git_changes_cursor: std::collections::HashMap::new(),
            git_changes_panel_focus: cx.focus_handle(),
            panels: main_area::bottom_dock::macro_ops::load_or_seed_panels(&data_dir),
            // Task data lives in the app-wide `GlobalTasks`; this
            // subscription rebroadcasts mutations into this
            // workspace's render path and re-evaluates whether the
            // R-23 live tick (pulse + duration) needs to be running.
            _tasks_global_subscription: cx
                .observe_global::<crate::agent::tasks_global::GlobalTasks>(|ws, cx| {
                    ws.ensure_task_live_tick(cx);
                    ws.notify_right_dock(cx);
                    cx.notify();
                }),
            _task_live_tick: None,
            task_filter: daruda_store::tasks::TaskFilter::default(),
            pending_lane_creates: HashSet::new(),
            window_close_in_flight: false,
            terminal_input,
            _terminal_input_subscription: terminal_input_sub,
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
                |ws, cx| {
                    ws.notify_right_dock(cx);
                    cx.notify();
                },
            ),
            _mcp_watcher: None,
            _mcp_event_pump: None,
            mcp_project_dirs: Vec::new(),
            _mcp_global_subscription: cx.observe_global::<crate::agent::mcp::McpState>(|ws, cx| {
                ws.notify_right_dock(cx);
                cx.notify();
            }),
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
            window_handle: window.window_handle(),
        };
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
        // Kick off the R-23 pulse / duration tick if `load_from_dir`
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

        // R-25 / I-8: intercept `Cmd+Q` and red-cross close attempts so
        // dirty TaskEdit panes don't silently disappear. The callback
        // returns `false` to veto the close, spawns the async batch
        // prompt, and then re-issues `window.remove_window()` once the
        // user picks Save all / Discard all.
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
        // renders too — left-dock tree, status bar, window title, etc.
        // Without an explicit `cx.notify()` the dock keeps the stale
        // snapshot until an unrelated event fires (the May-2026
        // add-project / add-group regressions both had this shape).
        // Keep notify here so every call site (group/project CRUD,
        // lane DnD, policy updates) gets a render for free.
        cx.notify();
        // Left dock is `.cached()` and renders the project/group/lane
        // tree from Workspace fields. Durable mutations always change
        // that tree, so dirty the dock entity here to invalidate the
        // cache for every call site (Pitfall #10).
        self.notify_left_dock(cx);
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
        self.notify_left_dock(cx);
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
        let focused = self.main_area.focused_pane_id;
        self.main_area.panes.iter().any(|p| p.id == focused)
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

    /// `true` when at least one lane has a PID-confirmed Claude session in
    /// an animating status (anything but `Idle`). Gates the status-pulse
    /// pump so the shared `StatusPulseClock` only repaints windows that
    /// actually show motion — idle windows stay at zero redraws.
    pub(crate) fn has_animating_claude_status(&self) -> bool {
        if !self.claude.claude_status_enabled {
            return false;
        }
        if self.claude.pty_claude_bindings.is_empty() {
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
    /// (Policy B explicitly permits the same root across separate
    /// windows — the cross-window guard no longer applies.) Compares
    /// both as-given and canonicalized so the symlinked `/tmp` vs
    /// `/private/tmp` flavours on macOS still match.
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

    /// Invalidate the left dock's `.cached()` element so it re-renders
    /// from the freshly staged snapshot. The left dock renders the
    /// Worktrees/Git-changes/Files views from `Workspace` fields
    /// (projects, groups, lanes, git status, file tree, claude status)
    /// and never self-notifies for Workspace-owned state, so every
    /// mutation site of a left-dock source must call this — otherwise
    /// the cache shows stale data (root CLAUDE.md Pitfall #10).
    /// Embedded input entities (`git_commit_input`) self-notify and
    /// dirty this dock as an ancestor, so they need no explicit call.
    /// Lease-free (`App::notify`) — dock event listeners run inside a
    /// `Context<Dock>` lease, so leasing the dock here would double-lease.
    pub(crate) fn notify_left_dock(&self, cx: &mut Context<Self>) {
        let dock_id = self.left_dock.entity_id();
        gpui::App::notify(cx, dock_id);
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
        self.notify_right_dock(cx);
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
                    self.on_install_claude_hooks(&InstallClaudeHooks, window, cx);
                }
                "uninstall_claude_hooks" => {
                    self.on_uninstall_claude_hooks(&UninstallClaudeHooks, window, cx);
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
