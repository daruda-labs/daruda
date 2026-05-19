//! Snapshot types for dock rendering.
//!
//! Each `*DockSnapshot` struct carries a point-in-time copy of the
//! `Workspace` fields that the corresponding dock's `impl Render`
//! needs to build its element tree.  Snapshots are staged by
//! `render/mod.rs` at the start of every `Workspace::render` cycle
//! via `Entity<Dock>::update(cx, |d, _| d.snap = ...)`.
//!
//! **Why snapshots?**
//! `impl Render for Dock` runs inside a `Context<Dock>`.  Reading
//! `Workspace` state through `WeakEntity::upgrade()` + `.read(cx)`
//! from inside that context would re-enter the entity graph.
//! Staging a plain-data snapshot avoids all re-entry and keeps the
//! render closure free of entity borrows.

use std::sync::Arc;

use gpui::{FocusHandle, UniformListScrollHandle, WeakEntity};

use crate::files::tree::EntryKind;
use crate::workspace::left_dock::file_tree_ops::VisibleEntry;

use crate::workspace::Workspace;

// ----------------------------------------------------------------
// Left dock
// ----------------------------------------------------------------

/// Plain-data snapshot of one project for the left-dock worktrees
/// tree. Mirrors the runtime [`crate::project::Project`] fields the
/// render needs, plus the worktrees list copied by value so dock
/// render can iterate without re-entering the workspace entity.
#[derive(Clone)]
pub(in crate::workspace) struct ProjectSnapshot {
    pub id: daruda_store::project::ProjectId,
    pub name: gpui::SharedString,
    /// Group membership. When `Some`, the project renders nested under
    /// its group's accordion in the left dock; ungrouped projects sit
    /// at the top level.
    pub group_id: Option<daruda_store::project::GroupId>,
    /// Per-project tint surfaced by the upcoming group/project chrome.
    /// Render layer does not consume it yet.
    #[allow(dead_code)]
    pub color: Option<gpui::SharedString>,
    pub tab_order: u32,
    pub worktrees: Vec<crate::worktree::Worktree>,
    /// Last-active worktree id, mirrored from the runtime project so
    /// the dock can snap the active focus back to it when the user
    /// clicks the project header (§5.5).
    pub last_active_worktree_id: daruda_store::project::WorktreeId,
    /// Whether the project's worktree list is hidden under its header.
    /// Toggled by the project header chevron click.
    pub is_collapsed: bool,
}

/// Plain-data snapshot of one group for the left-dock tree.
///
/// Drives the accordion header rendered in the Worktrees view: caret
/// flips on `is_collapsed`, optional color dot keyed off `color`, and
/// member projects fold/unfold based on the same flag.
#[derive(Clone)]
pub(in crate::workspace) struct GroupSnapshot {
    pub id: daruda_store::project::GroupId,
    pub name: gpui::SharedString,
    pub color: Option<gpui::SharedString>,
    pub tab_order: u32,
    pub is_collapsed: bool,
}

/// Point-in-time copy of `Workspace` fields consumed by the left
/// dock's `impl Render`.
pub(in crate::workspace) struct LeftDockSnapshot {
    pub left_dock_view: daruda_store::project::LeftDockView,
    /// Display name of the active project. Superseded by per-project
    /// headers driven off `projects` once the multi-project tree
    /// landed; kept around so any pending consumer (status bar,
    /// header rebuilds) can still read it. Unused inside the
    /// worktrees view itself.
    #[allow(dead_code)]
    pub active_project_name: Option<gpui::SharedString>,
    pub worktrees: Vec<crate::worktree::Worktree>,
    /// Every project in the workspace, in `tab_order` order. Drives
    /// the multi-project tree render — each entry's `worktrees` slice
    /// is shown under its project header. Empty when the workspace is
    /// in the Welcome state.
    pub projects: Vec<ProjectSnapshot>,
    /// Every group in the workspace, in `tab_order` order. Group
    /// headers interleave with ungrouped projects in the top-level
    /// tree via the shared `tab_order` pool.
    pub groups: Vec<GroupSnapshot>,
    pub active: daruda_store::project::WorktreeRef,
    pub active_tab_count: usize,
    pub git_status_cache: std::collections::HashMap<
        daruda_store::project::WorktreeRef,
        crate::worktree::git::GitStatusData,
    >,
    pub git_stage_in_flight: bool,
    /// Mirrors `Workspace::git_op_in_flight` — true while a Fetch / Push /
    /// Commit / Amend is running. Drives `loading + disabled` on the Fetch
    /// and Push buttons in the Git Changes header.
    pub git_op_in_flight: bool,
    /// Active worktree's collapsed dir set (Git Changes view). Keyed by
    /// worktree-relative dir string.
    pub git_collapsed_dirs: std::collections::HashSet<String>,
    /// Active worktree's keyboard cursor in the Git Changes view —
    /// repo-root-relative path of the focused row (or None if no row is
    /// focused). Drives the visual cursor highlight.
    pub git_changes_cursor: Option<std::path::PathBuf>,
    /// Focus handle bound to `key_context("GitChanges")`. Track this on
    /// the Git Changes body so its keyboard shortcuts fire only when
    /// the panel holds focus.
    pub git_changes_panel_focus: FocusHandle,
    /// `(worktree, path, staged)` of the focused file viewer's pane,
    /// or `None` when no file pane is focused. Dock rows render a
    /// "selected" background when this triple matches.
    pub focused_file_selection:
        Option<(daruda_store::project::WorktreeId, std::path::PathBuf, bool)>,
    pub git_changes_scroll_handle: gpui::ScrollHandle,
    pub git_commit_input: gpui::Entity<crate::ui::InputPanel>,
    pub files_panel_focus: FocusHandle,
    pub files_scroll_handle: UniformListScrollHandle,
    pub files_icon_color_mode: daruda_config::IconColorMode,
    pub cached_visible: Arc<Vec<VisibleEntry>>,
    pub root_kind: Option<EntryKind>,
    /// Aggregate Claude status per worktree. Keyed by `WorktreeRef`
    /// so worktree ids in distinct projects (each project numbers its
    /// worktrees from 0) don't collide. Empty when the
    /// `claude_status.enable` config flag is off, no Claude session is
    /// running, or the worktree has no matching cwd.
    pub claude_status_per_worktree: std::collections::HashMap<
        daruda_store::project::WorktreeRef,
        daruda_claude::SessionStatus,
    >,
    /// Per-session statuses for worktrees that have ≥ 2 active Claude
    /// sessions. Phase D sub-row badges read this. Worktrees with 0 or
    /// 1 sessions are absent (the leading indicator covers them).
    /// Keyed by `WorktreeRef` for the same cross-project reason.
    pub claude_per_session_per_worktree: std::collections::HashMap<
        daruda_store::project::WorktreeRef,
        Vec<(String, daruda_claude::SessionStatus)>,
    >,
    /// `session_id` of the claude process living inside the focused
    /// pane (Phase E). Used by sub-row badge render to highlight the
    /// session attached to the active terminal. `None` when the
    /// focused pane has no claude descendant or the tracker hasn't
    /// resolved it yet.
    pub claude_active_session_id: Option<String>,
    /// Show the "Claude integration disabled" banner above the
    /// worktrees list. True when status is enabled in config but
    /// hooks aren't yet installed in `~/.claude/settings.json`.
    pub claude_install_banner_visible: bool,
    pub workspace: WeakEntity<Workspace>,
}

// ----------------------------------------------------------------
// Bottom dock
// ----------------------------------------------------------------

/// Point-in-time copy of `Workspace` fields consumed by the bottom
/// dock's `impl Render`.
pub(in crate::workspace) struct BottomDockSnapshot {
    pub terminal_input_visible: bool,
    pub active_tab_id: Option<daruda_store::panels::TabId>,
    pub tab_summaries: Vec<(daruda_store::panels::TabId, String, usize)>,
    pub active_tab_widgets: Vec<daruda_store::panels::MacroKey>,
    /// Mirror of `Workspace::panels_grid_columns` — column count for
    /// the bottom-dock macro tile grid. Already clamped (>= 1).
    pub grid_columns: u8,
    /// Current bottom dock height in px, mirrored from
    /// `Workspace::bottom_dock.read(cx).size`. Used by the tab-strip
    /// suffix to pick the active row-preset label and the menu
    /// checkmark without re-reading the dock entity during render.
    pub bottom_dock_size: f32,
    pub terminal_input: gpui::Entity<crate::ui::InputState>,
    /// Shell flavour of the focused pane's PTY. Drives drag-and-drop path
    /// quoting in the terminal input — Posix backslash/single-quote rules,
    /// fish, PowerShell, and cmd.exe all differ.
    pub shell: crate::shell_quote::Shell,
    pub workspace: WeakEntity<Workspace>,
}

// ----------------------------------------------------------------
// Right dock
// ----------------------------------------------------------------

/// Point-in-time copy of `Workspace` fields consumed by the right
/// dock's `impl Render`.
pub(in crate::workspace) struct RightDockSnapshot {
    /// Active right-panel tab. The tab strip in the dock header reads
    /// this to highlight the current tab; the body match-arm reads it
    /// to pick the renderer.
    pub right_dock_view: daruda_store::project::RightDockView,
    /// Back-reference to the owning `Workspace`, mirrored from the
    /// dock entity. Tab-strip click handlers upgrade this to dispatch
    /// `set_right_dock_view` without re-entering the dock context.
    pub workspace: WeakEntity<Workspace>,
    /// Snapshot of `Workspace::usage` for the Usage tab renderer.
    /// Carried in the snap so the per-tab body can read it without
    /// re-entering the workspace entity.
    pub usage: daruda_claude::usage::UsageState,
    /// Per-million-token pricing applied when rendering the Usage
    /// tab's cost columns. Sourced from `Workspace::usage_pricing` so
    /// the render pass never reaches into the workspace entity.
    pub usage_pricing: daruda_claude::usage::UsagePricing,
    /// Latest 5h / 7d plan-rate snapshot. Default-constructed (both
    /// windows `None`) before the first successful `/api/oauth/usage`
    /// fetch, in which case the renderer draws placeholder gauges.
    pub plan_limits: daruda_claude::PlanLimits,
    /// Latest service-status snapshot. Default is `Unknown`, which
    /// the renderer maps to a dimmed pill.
    pub service_status: daruda_claude::ServiceStatus,
    /// Active time window for the Usage tab. The renderer uses this
    /// to compute the cutoff and pick the dropdown's selected option
    /// without dipping back into the workspace.
    pub usage_window: daruda_store::project::UsageWindow,
    /// Entity handle for the Usage tab's time-window dropdown.
    /// Cloned cheaply each frame; the workspace is the source of
    /// truth for selection state.
    pub usage_select: gpui::Entity<crate::ui::select::SelectState>,
    /// Per-worktree projection of the app-wide `SkillsState` Global
    /// for the Skills tab renderer. Carried by-value so the panel
    /// renderer never re-enters the workspace entity (G2 / pitfall §4)
    /// and never reads the Global from inside `Render::render`.
    pub skills: crate::agent::skills::SkillsSnapshot,
    /// Search query input rendered atop the Skills tab. Entity is
    /// shared with the Workspace; the renderer just embeds it inline.
    pub skill_search_input: gpui::Entity<crate::ui::InputState>,
    /// Captured search text at snap build time. Lowercase substring
    /// match against skill `name` and `frontmatter.description` filters
    /// every scope (Project / Personal / Plugin) simultaneously.
    pub skill_search_query: String,
    /// Plugin ids whose accordion section in the Skills tab is open.
    /// Renderer treats default (empty) as "all collapsed". Cloned per
    /// frame — small set, cheap.
    pub skill_plugin_expanded: std::collections::HashSet<String>,
    /// Snapshot of the Tasks tab's task list. Plain-data clone — the
    /// renderer never touches `Workspace::tasks` directly.
    pub tasks: daruda_store::tasks::TasksState,
    /// Search query input rendered atop the Tasks tab. Entity is
    /// shared with the Workspace; the renderer just embeds it inline.
    pub task_search_input: gpui::Entity<crate::ui::InputState>,
    /// Captured search text at snap build time. Lowercase substring
    /// match against `title / prompt / notes / branch_name`.
    pub task_search_query: String,
    /// Active Tasks-tab filter (Backlog / Running / Done / All).
    pub task_filter: daruda_store::tasks::TaskFilter,
    /// Aggregate Claude session status per worktree, keyed by the
    /// worktree's filesystem path so the Tasks tab can paint a
    /// session badge next to each `Running` row without consulting
    /// the workspace entity. Empty when the `claude_status.enable`
    /// config flag is off.
    #[allow(dead_code)]
    pub claude_status_per_path:
        std::collections::HashMap<std::path::PathBuf, daruda_claude::SessionStatus>,
    /// Per-session Claude status, keyed by `session_id`. Mirrors the
    /// `ClaudeStatusStore` slice that the Tasks tab needs to render
    /// the `⟳ / ● / ⚠` glyph trailing each row's session-id badge
    /// (R-23). Empty when `claude_status.enable` is off or no
    /// session has reported a status yet.
    pub claude_status_per_session: std::collections::HashMap<String, daruda_claude::SessionStatus>,
    /// Per-session tool-use failure counts mirrored from
    /// `Workspace::claude.tool_use_failure_counts`. Only sessions
    /// with a positive count are present. The Tasks-tab renderer
    /// surfaces a `failures N/M` hint once a count reaches
    /// [`daruda_terminal::ux::strings::RIGHT_PANEL_TASK_FAILURE_DISPLAY_THRESHOLD`]
    /// (R-23 / R-11 Phase 2 carry-over).
    pub tool_use_failure_counts: std::collections::HashMap<String, u32>,
    /// Reference instant captured once per frame so every row in the
    /// Tasks tab computes its live duration against the same `now`.
    /// `Utc::now()` per row would drift between calls within a single
    /// frame and break the `2m 14s → 2m 15s` invariant for sibling
    /// rows that update on the same tick (R-23).
    pub now: chrono::DateTime<chrono::Utc>,
    /// Scroll handle shared between the right-panel body's
    /// `overflow_y_scroll` and the scrollbar thumb overlay. Cloned from
    /// `Workspace::right_panel_scroll_handle` each frame.
    pub right_panel_scroll_handle: gpui::ScrollHandle,
    /// Snapshot of `Workspace::mcp` for the Tools tab renderer.
    /// Carried by-value so the panel renderer never re-enters the
    /// workspace entity (G2 / pitfall §4).
    pub mcp: crate::agent::mcp::McpSnapshot,
}

// ----------------------------------------------------------------
// Discriminated union stored on Dock
// ----------------------------------------------------------------

/// The staging field on `Dock`. Each `Dock` holds exactly one variant
/// corresponding to its `DockPosition`. Initialized to `None` and
/// overwritten by `Workspace::render` before `Dock::render` is invoked.
///
/// `Left` and `Right` are `Box`ed because `LeftDockSnapshot` (~416 B)
/// and `RightDockSnapshot` (~776 B) dwarf the other variants — leaving
/// them inline would inflate every `DockSnapshot` slot, including the
/// cleared `None` state, well past clippy's variant-size threshold.
pub(in crate::workspace) enum DockSnapshot {
    Left(Box<LeftDockSnapshot>),
    Bottom(BottomDockSnapshot),
    Right(Box<RightDockSnapshot>),
    /// Initial / cleared state — `Dock::render` returns an empty
    /// element when the snap is absent (first frame safety net).
    None,
}
