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
mod claude_session_ops;
pub(in crate::workspace) mod command;
mod config_ops;
pub(in crate::workspace) mod dialog_helpers;
pub(in crate::workspace) mod error;
pub(in crate::workspace) mod layout;
mod left_dock;
pub(in crate::workspace) mod main_area;
pub(in crate::workspace) mod modal_view;
mod path_drag;
mod persistence;
mod render;
mod right_dock;
mod spawn_helpers;
pub(crate) mod status_bar;
pub(in crate::workspace) mod sync;
#[cfg(test)]
mod tests;
mod toast_layer;
mod window_close_ops;
mod worktree_ops;

pub(in crate::workspace) use modal_view::ModalView;
pub(in crate::workspace) use persistence::WorktreeRuntime;

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
        ActivateWorktree1,
        ActivateWorktree2,
        ActivateWorktree3,
        ActivateWorktree4,
        ActivateWorktree5,
        ActivateWorktree6,
        ActivateWorktree7,
        ActivateWorktree8,
        ActivateWorktree9,
        ShowLeftDockWorktrees,
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
    ]
);

// ----------------------------------------------------------------
// Workspace
// ----------------------------------------------------------------

const TITLE_BAR_HEIGHT: f32 = crate::ui::theme::TITLE_BAR_HEIGHT;
const TAB_BAR_HEIGHT: f32 = crate::ui::theme::TAB_BAR_HEIGHT;

pub struct Workspace {
    /// TabBar + PaneTree runtime state. Holds tabs, panes, focus,
    /// drag/context-menu overlays, and inactive worktree runtimes.
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
    /// Left dock (worktree list, git changes, files — the
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
    /// Worktrees for this project. Always non-empty when `project` is
    /// `Some` (Workspace::new_with_project bootstraps one default entry).
    /// `tabs`/`panes` still live on `Workspace` until W-3 migrates
    /// them into `Worktree`.
    pub(in crate::workspace) worktrees: Vec<crate::worktree::Worktree>,
    /// ID of the visible worktree. Meaningful only when `worktrees`
    /// is non-empty.
    pub(in crate::workspace) active_worktree_id: daruda_store::project::WorktreeId,
    /// Bottom dock (terminal panel, problems, output).
    pub(in crate::workspace) bottom_dock: gpui::Entity<layout::Dock>,
    /// Right dock (file explorer, git changes).
    pub(in crate::workspace) right_dock: gpui::Entity<layout::Dock>,
    /// Command palette state (Cmd+Shift+P).
    pub(in crate::workspace) command_palette: command::palette::CommandPaletteState,
    /// Active project. None = empty workspace (no project root).
    pub(in crate::workspace) project: Option<daruda_store::project::Project>,
    /// Cached git status per worktree. Refreshed when the Git Changes
    /// view is activated or after a commit. Only entries that have been
    /// fetched at least once are present; missing = "not yet loaded".
    pub(in crate::workspace) git_status_cache:
        HashMap<daruda_store::project::WorktreeId, crate::worktree::git::GitStatusData>,
    /// Left-dock Files view state — per-worktree lazy tree, watcher,
    /// gitignore matcher, scroll handle, keyboard cursor. Grouped
    /// into one struct so the 12 sub-fields don't clutter
    /// `Workspace`'s top level. See [`left_dock::file_tree_context::FileTreeContext`].
    pub(in crate::workspace) file_tree: left_dock::file_tree_context::FileTreeContext,
    /// Per-worktree "git status currently running" guard. Watcher
    /// events can arrive faster than `git status` can complete on a
    /// large repo; this keeps at most one in-flight task per worktree.
    pub(in crate::workspace) git_status_in_flight: HashSet<daruda_store::project::WorktreeId>,
    /// Set of worktrees that asked for a status refresh while one was
    /// already running. Drained by the in-flight task on completion,
    /// re-firing once to capture intervening changes.
    pub(in crate::workspace) git_status_pending_repeat: HashSet<daruda_store::project::WorktreeId>,
    /// Mirror of `daruda_config::PanelsConfig::grid_columns`. Drives the
    /// bottom-dock macro tile grid column count.
    pub(in crate::workspace) panels_grid_columns: u8,
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
    /// User-set window title (Window > Edit Window Title…). When
    /// `Some`, replaces the auto-derived `"<pane title> — <cwd>"`
    /// string passed to `window.set_window_title`. Persisted to
    /// `ProjectState.window_user_label`.
    pub(in crate::workspace) window_user_label: Option<gpui::SharedString>,
    /// When true, the stdout poll task auto-closes a pane (and its
    /// containing tab if it was the only pane) as soon as the shell
    /// process terminates. Mirrors iTerm2's "When the Session Ends"
    /// preference. Read live by the poll task, so a config reload
    /// takes effect for already-running panes.
    pub(in crate::workspace) close_pane_on_exit: bool,
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
    /// Per-worktree set of collapsed dir groups in the Git Changes view.
    /// Keyed by the worktree-relative dir string emitted by `group_by_dir`
    /// (e.g. `"src/workspace/left_dock"`). In-memory only — collapse state
    /// resets on app restart by design.
    pub(in crate::workspace) git_collapsed_dirs:
        std::collections::HashMap<daruda_store::project::WorktreeId, HashSet<String>>,
    /// Per-worktree keyboard cursor in the Git Changes view, stored as
    /// the file's repo-root-relative path (the same shape git status
    /// porcelain emits, so it round-trips into stage/unstage/diff ops
    /// without conversion). Path-keyed (not index-keyed) so refreshes
    /// that re-sort the list keep the cursor on the same file.
    pub(in crate::workspace) git_changes_cursor:
        std::collections::HashMap<daruda_store::project::WorktreeId, std::path::PathBuf>,
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
    /// repository. Cleared after `finalize_create_worktree` returns.
    pub(in crate::workspace) pending_worktree_creates: HashSet<std::path::PathBuf>,
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
    /// Subscription that calls `cx.notify()` whenever the `McpState`
    /// Global changes — so panels in this workspace re-render after a
    /// mutation triggered by another workspace's watcher or by a
    /// Settings-window action.
    _mcp_global_subscription: gpui::Subscription,
    /// Subscription on the `SettingsStore` Global. Re-resolves the
    /// effective config (user layer + this workspace's project
    /// overlay) and calls `apply_config` on every change.
    _settings_global_subscription: gpui::Subscription,
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
                .with_placeholder(crate::surface::strings::GIT_COMMIT_PLACEHOLDER, window, cx)
                .with_borderless(cx)
                .with_focus_ring(false, cx)
                .with_action(
                    crate::ui::PanelAction::new(
                        "commit",
                        crate::surface::strings::GIT_COMMIT_BTN,
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
                        crate::surface::strings::CTX_GIT_COMMIT_AMEND,
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
                crate::surface::strings::BOTTOM_INPUT_PLACEHOLDER,
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

        // Right-panel Usage tab time-window dropdown. Owned by
        // Workspace (not the Dock entity) so the selection survives
        // dock teardown and the workspace's persistence layer can
        // sync the picker after `restore_state` reapplies the saved
        // `active_usage_window`.
        let usage_default_slug =
            gpui::SharedString::from(daruda_store::project::UsageWindow::default().slug());
        let usage_select = cx.new(|cx_inner| {
            let opts: Vec<crate::ui::select::SelectOption> =
                daruda_store::project::UsageWindow::ALL
                    .iter()
                    .map(|w| {
                        crate::ui::select::SelectOption::new(
                            w.slug(),
                            crate::surface::strings::usage_window_label(*w),
                        )
                    })
                    .collect();
            crate::ui::select::state_with_options(opts, Some(&usage_default_slug), window, cx_inner)
        });
        let usage_select_sub = cx.subscribe_in(
            &usage_select,
            window,
            |this, _entity, ev: &crate::ui::select::ConfirmEvent, window, cx| {
                let crate::ui::select::SelectEvent::Confirm(Some(value)) = ev else {
                    return;
                };
                if let Some(w) = daruda_store::project::UsageWindow::from_slug(value.as_ref()) {
                    this.set_usage_window(w, window, cx);
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
            main_area: main_area::MainAreaContext::default(),
            next_id: 0,
            focus_handle,
            dock_drag: None,
            dim_alpha: render::DEFAULT_INACTIVE_PANE_DIM_ALPHA,
            inherit_cwd: true,
            terminal_config: {
                // Build ANSI 16-color palette from the effective colors
                // (preset overrides the [colors] section when set).
                let colors = config.effective_colors();
                let pal = colors.to_ansi_palette();
                let mut cfg = TerminalConfig {
                    default_fg: ghostty_vt::Rgb {
                        r: colors.foreground.r,
                        g: colors.foreground.g,
                        b: colors.foreground.b,
                    },
                    default_bg: ghostty_vt::Rgb {
                        r: colors.background.r,
                        g: colors.background.g,
                        b: colors.background.b,
                    },
                    font_size: config.font.size,
                    vertical_spacing: config.font.vertical_spacing,
                    horizontal_spacing: config.font.horizontal_spacing,
                    palette: Some(pal),
                    background_alpha: config.window.opacity,
                    osc1337_max_bytes: config.clipboard.streaming_max_bytes,
                    ..TerminalConfig::default()
                };
                cfg.clamp_font_settings();
                cfg
            },
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
                    d.add_panel(layout::WorktreesPanel);
                    d.add_panel(layout::GitChangesPanel);
                    d.add_panel(layout::FilesPanel);
                    d
                })
            },
            left_dock_view: daruda_store::project::LeftDockView::default(),
            right_dock_view: daruda_store::project::RightDockView::default(),
            claude: claude_session_ops::ClaudeContext {
                usage: daruda_claude::usage::UsageState::default(),
                usage_pricing: config_ops::usage_pricing_from_config(&config.usage.pricing),
                usage_poll: config.usage.poll.clone(),
                plan_limits: daruda_claude::PlanLimits::default(),
                service_status: daruda_claude::ServiceStatus::default(),
                usage_window: daruda_store::project::UsageWindow::default(),
                usage_select,
                _usage_select_subscription: usage_select_sub,
                claude_status: {
                    // Cold restore: load any status files that survived a
                    // previous run. TTL cleanup runs at the same time so
                    // orphans from past crashes don't accumulate. The
                    // stale-NeedsAttention threshold lives on the store
                    // itself (used by aggregate queries to demote stuck
                    // states from a TUI-dismissed Notification).
                    let mut store = daruda_claude::ClaudeStatusStore::new()
                        .with_needs_attention_stale(std::time::Duration::from_secs(
                            config.claude_status.needs_attention_stale_secs,
                        ));
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
                claude_hooks_installed: crate::hooks::installer::InstallerPaths::from_env()
                    .map(|p| crate::hooks::installer::is_installed(&p))
                    .unwrap_or(false),
                pty_tracker,
                pty_claude_bindings: HashMap::new(),
                _pty_event_pump: pty_event_pump,
                _jsonl_watcher_shutdown: None,
                _jsonl_event_pump: None,
                tool_use_failure_counts: HashMap::new(),
                _limits_pumps: sync::limits::spawn(cx),
            },
            // Bootstrap worktrees for this project. If git is
            // installed and the path is a repo, every linked worktree
            // becomes an entry (the one at project_root sorts to
            // `id = 0` so it's the active one). Otherwise a single
            // `Default` worktree is used. Skipped entirely for
            // project-less windows.
            worktrees: project
                .as_ref()
                .map(|p| crate::worktree::Worktree::bootstrap_from_project(&p.root))
                .unwrap_or_default(),
            active_worktree_id: 0,
            bottom_dock: {
                let ws = ws_weak.clone();
                cx.new(|_| {
                    let mut d = layout::Dock::new(layout::DockPosition::Bottom, ws);
                    d.add_panel(layout::MacrosPanel);
                    d
                })
            },
            command_palette: command::palette::CommandPaletteState::default(),
            project: project.clone(),
            git_status_cache: HashMap::new(),
            file_tree: left_dock::file_tree_context::FileTreeContext {
                file_trees: HashMap::new(),
                files_visible_cache: HashMap::new(),
                file_watchers: HashMap::new(),
                files_reload_queues: HashMap::new(),
                files_watcher_poll: None,
                files_show_hidden: config.left_dock.files_show_hidden,
                files_panel_focus: cx.focus_handle(),
                files_selection: None,
                files_use_gitignore: config.left_dock.files_use_gitignore,
                files_icon_color_mode: config.left_dock.file_icon_color_mode.clone(),
                files_gitignore_index: HashMap::new(),
                files_scroll_handle: gpui::UniformListScrollHandle::new(),
            },
            git_status_in_flight: HashSet::new(),
            git_status_pending_repeat: HashSet::new(),
            panels_grid_columns: config.panels.grid_columns,
            git_changes_scroll_handle: gpui::ScrollHandle::new(),
            right_panel_scroll_handle: gpui::ScrollHandle::new(),
            git_commit_input,
            _git_commit_subscription: git_commit_sub,
            last_error: None,
            error_history: Vec::new(),
            toast_layer,
            cached_window_bounds: None,
            window_user_label: None,
            close_pane_on_exit: config.shell.close_pane_on_exit,
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
                    cx.notify();
                }),
            _task_live_tick: None,
            task_filter: daruda_store::tasks::TaskFilter::default(),
            pending_worktree_creates: HashSet::new(),
            window_close_in_flight: false,
            terminal_input,
            _terminal_input_subscription: terminal_input_sub,
            terminal_input_visible: false,
            skill_search_input: cx.new(|cx_state| {
                crate::ui::InputState::new(window, cx_state)
                    .placeholder(crate::surface::strings::SKILLS_SEARCH_PLACEHOLDER)
            }),
            task_search_input: cx.new(|cx_state| {
                crate::ui::InputState::new(window, cx_state)
                    .placeholder(crate::surface::strings::TASK_SEARCH_PLACEHOLDER)
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
            _skills_global_subscription: cx
                .observe_global::<crate::agent::skills::SkillsState>(|_, cx| cx.notify()),
            _mcp_watcher: None,
            _mcp_event_pump: None,
            _mcp_global_subscription: cx
                .observe_global::<crate::agent::mcp::McpState>(|_, cx| cx.notify()),
            // Re-resolve the user layer with this workspace's project
            // overlay and reapply whenever `SettingsStore` changes —
            // both the FS watch tick and the Settings-window save
            // fold into the same Global mutation and fanout here.
            _settings_global_subscription: cx
                .observe_global::<crate::settings_store::SettingsStore>(|ws, cx| {
                    let store = crate::settings_store::SettingsStore::global(cx);
                    let worktree = ws.project.as_ref().map(|p| p.root.as_path());
                    let effective = store.effective_for(worktree);
                    ws.apply_config(&effective, cx);
                }),
        };
        ws.add_tab(window, cx);
        // After tabs/worktrees are seeded, decide whether the JSONL
        // fallback watcher should run for this Workspace and start it.
        ws.refresh_jsonl_watcher(cx);
        // Skills watcher: subscribes to project + personal skill dirs
        // and re-scans on every filesystem change.
        ws.refresh_skills_watcher(cx);
        // MCP watcher: subscribes to ~/.claude/settings.json and the
        // active worktree's .mcp.json, re-loading the matching scope on
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
        self.left_dock_view = view;
        self.mark_dirty_and_save(cx);
        if view == daruda_store::project::LeftDockView::GitChanges {
            let id = self.active_worktree_id;
            self.refresh_git_status(id, cx);
        }
        cx.notify();
    }

    pub(in crate::workspace) fn set_right_dock_view(
        &mut self,
        view: daruda_store::project::RightDockView,
        cx: &mut Context<Self>,
    ) {
        if self.right_dock_view == view {
            return;
        }
        self.right_dock_view = view;
        self.mark_dirty_and_save(cx);
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
                "focus_next_pane" => self.on_focus_next_pane(&FocusNextPane, window, cx),
                "focus_prev_pane" => self.on_focus_prev_pane(&FocusPrevPane, window, cx),
                "focus_pane_left" => self.on_focus_pane_left(&FocusPaneLeft, window, cx),
                "focus_pane_right" => self.on_focus_pane_right(&FocusPaneRight, window, cx),
                "focus_pane_up" => self.on_focus_pane_up(&FocusPaneUp, window, cx),
                "focus_pane_down" => self.on_focus_pane_down(&FocusPaneDown, window, cx),
                "move_tab_left" => self.on_move_tab_left(&MoveTabLeft, window, cx),
                "move_tab_right" => self.on_move_tab_right(&MoveTabRight, window, cx),
                "show_left_dock_worktrees" => {
                    self.on_show_left_dock_worktrees(&ShowLeftDockWorktrees, window, cx);
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
