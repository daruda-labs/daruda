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

/// Plain-data snapshot of one project for the left-dock lanes
/// tree. Mirrors the runtime [`crate::project::Project`] fields the
/// render needs, plus the lanes list copied by value so dock
/// render can iterate without re-entering the workspace entity.
#[derive(Clone, PartialEq)]
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
    /// Mirror of the runtime project's detected default branch (e.g.
    /// "main"). `None` for non-git projects or when detection has not
    /// resolved yet. Drives the branch chip rendered on the project header row.
    pub default_branch: Option<gpui::SharedString>,
    pub lanes: Vec<crate::lane::Lane>,
    /// Last-active lane id, mirrored from the runtime project so
    /// the dock can snap the active focus back to it when the user
    /// clicks the project header (§5.5).
    pub last_active_lane_id: daruda_store::project::LaneId,
    /// Whether the project's lane list is hidden under its header.
    /// Toggled by the project header chevron click.
    pub is_collapsed: bool,
    /// Runtime read-availability of the project root directory.
    /// Drives the muted-header + state-icon treatment when the root
    /// is missing or access-denied. Mirrored from
    /// [`crate::project::Project::availability`]; never serialized.
    pub availability: crate::lane::availability::LaneAvailability,
}

/// Plain-data snapshot of one group for the left-dock tree.
///
/// Drives the accordion header rendered in the Lanes view: caret
/// flips on `is_collapsed`, optional color dot keyed off `color`, and
/// member projects fold/unfold based on the same flag.
#[derive(Clone, PartialEq)]
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
    /// lanes view itself.
    #[allow(dead_code)]
    pub active_project_name: Option<gpui::SharedString>,
    pub lanes: Vec<crate::lane::Lane>,
    /// Every project in the workspace, in `tab_order` order. Drives
    /// the multi-project tree render — each entry's `lanes` slice
    /// is shown under its project header. Empty when the workspace is
    /// in the Welcome state.
    pub projects: Vec<ProjectSnapshot>,
    /// Every group in the workspace, in `tab_order` order. Group
    /// headers interleave with ungrouped projects in the top-level
    /// tree via the shared `tab_order` pool.
    pub groups: Vec<GroupSnapshot>,
    pub active: daruda_store::project::LaneRef,
    pub git_status_cache:
        std::collections::HashMap<daruda_store::project::LaneRef, crate::lane::git::GitStatusData>,
    pub git_stage_in_flight: bool,
    /// Mirrors `Workspace::git_op_in_flight` — true while a Fetch / Push /
    /// Commit / Amend is running. Drives `loading + disabled` on the Fetch
    /// and Push buttons in the Git Changes header.
    pub git_op_in_flight: bool,
    /// Active lane's collapsed dir set (Git Changes view). Keyed by
    /// lane-relative dir string.
    pub git_collapsed_dirs: std::collections::HashSet<String>,
    /// Active lane's keyboard cursor in the Git Changes view —
    /// repo-root-relative path of the focused row (or None if no row is
    /// focused). Drives the visual cursor highlight.
    pub git_changes_cursor: Option<std::path::PathBuf>,
    /// Focus handle bound to `key_context("GitChanges")`. Track this on
    /// the Git Changes body so its keyboard shortcuts fire only when
    /// the panel holds focus.
    pub git_changes_panel_focus: FocusHandle,
    /// `(lane, path, staged)` of the focused file viewer's pane,
    /// or `None` when no file pane is focused. Dock rows render a
    /// "selected" background when this triple matches.
    pub focused_file_selection: Option<(daruda_store::project::LaneId, std::path::PathBuf, bool)>,
    pub git_changes_scroll_handle: gpui::ScrollHandle,
    pub git_commit_input: gpui::Entity<crate::ui::InputPanel>,
    pub files_panel_focus: FocusHandle,
    pub files_scroll_handle: UniformListScrollHandle,
    pub files_icon_color_mode: daruda_config::IconColorMode,
    pub cached_visible: Arc<Vec<VisibleEntry>>,
    pub root_kind: Option<EntryKind>,
    /// Aggregate Claude status per lane. Keyed by `LaneRef`
    /// so lane ids in distinct projects (each project numbers its
    /// lanes from 0) don't collide. Empty when the
    /// `claude_status.enable` config flag is off, no Claude session is
    /// running, or the lane has no matching cwd.
    pub claude_status_per_lane:
        std::collections::HashMap<daruda_store::project::LaneRef, daruda_claude::SessionStatus>,
    /// Per-session statuses for lanes that have ≥ 2 active Claude
    /// sessions. Phase D sub-row badges read this. Lanes with 0 or
    /// 1 sessions are absent (the leading indicator covers them).
    /// Keyed by `LaneRef` for the same cross-project reason.
    pub claude_per_session_per_lane: std::collections::HashMap<
        daruda_store::project::LaneRef,
        Vec<(String, daruda_claude::SessionStatus)>,
    >,
    /// `session_id` of the claude process living inside the focused
    /// pane (Phase E). Used by sub-row badge render to highlight the
    /// session attached to the active terminal. `None` when the
    /// focused pane has no claude descendant or the tracker hasn't
    /// resolved it yet.
    pub claude_active_session_id: Option<String>,
    /// Show the "Claude integration disabled" banner above the
    /// lanes list. True when status is enabled in config but
    /// hooks aren't yet installed in `~/.claude/settings.json`.
    pub claude_install_banner_visible: bool,
    pub workspace: WeakEntity<Workspace>,
}

impl LeftDockSnapshot {
    /// True when the left-dock-relevant *content* differs from `prev`.
    ///
    /// Hand-written (rather than a derived `PartialEq` like
    /// `BottomDockSnapshot`) for two reasons:
    /// - `cached_visible` is a stable `Arc` (see `cached_or_rebuild_visible`,
    ///   which clones the cached `Arc` until invalidated) so it is compared
    ///   by `Arc::ptr_eq` — O(1), with no need for `VisibleEntry: PartialEq`.
    /// - GPUI handles (`FocusHandle` / `ScrollHandle` /
    ///   `UniformListScrollHandle` / `Entity` / `WeakEntity`) are the same
    ///   instance for the workspace's lifetime, so they are
    ///   content-irrelevant and intentionally EXCLUDED: `git_changes_panel_focus`,
    ///   `git_changes_scroll_handle`, `git_commit_input`, `files_panel_focus`,
    ///   `files_scroll_handle`, `workspace`. `active_project_name` is also
    ///   excluded — it is `#[allow(dead_code)]`, unused by the render.
    ///
    /// Wired into left-dock render staging (`render/mod.rs`) as the
    /// notify-on-change comparator: the cached dock is marked dirty only
    /// when this reports a content difference. This is now the sole
    /// left-dock invalidation mechanism — the manual `notify_left_dock()`
    /// calls are gone; a plain workspace `cx.notify()` re-stages the
    /// snapshot and lets this diff decide. (The status pulse is the one
    /// exception: it dirties the dock directly to animate badges, whose
    /// frames are not part of this snapshot.)
    pub(in crate::workspace) fn content_differs(&self, prev: &Self) -> bool {
        !std::sync::Arc::ptr_eq(&self.cached_visible, &prev.cached_visible)
            || self.left_dock_view != prev.left_dock_view
            || self.active != prev.active
            || self.lanes != prev.lanes
            || self.projects != prev.projects
            || self.groups != prev.groups
            || self.git_status_cache != prev.git_status_cache
            || self.git_stage_in_flight != prev.git_stage_in_flight
            || self.git_op_in_flight != prev.git_op_in_flight
            || self.git_collapsed_dirs != prev.git_collapsed_dirs
            || self.git_changes_cursor != prev.git_changes_cursor
            || self.focused_file_selection != prev.focused_file_selection
            || self.files_icon_color_mode != prev.files_icon_color_mode
            || self.root_kind != prev.root_kind
            || self.claude_status_per_lane != prev.claude_status_per_lane
            || self.claude_per_session_per_lane != prev.claude_per_session_per_lane
            || self.claude_active_session_id != prev.claude_active_session_id
            || self.claude_install_banner_visible != prev.claude_install_banner_visible
    }
}

// ----------------------------------------------------------------
// Bottom dock
// ----------------------------------------------------------------

/// Point-in-time copy of `Workspace` fields consumed by the bottom
/// dock's `impl Render`.
///
/// `PartialEq` drives the bottom dock's `.cached()` coherence: the
/// render staging compares each frame's snapshot against the last and
/// only fires `cx.notify(bottom_dock)` when content actually changed,
/// so the 250 ms status pulse (which leaves this snapshot identical)
/// doesn't repaint the dock. `Entity`/`WeakEntity` handles compare by
/// stable id, so they never spuriously trip the diff.
#[derive(PartialEq)]
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
    /// Set when the focused pane is an Agent chat pane with a turn in
    /// flight. Carries that pane's id so the bottom-input submit button
    /// renders "Stop" and routes its click to `cancel_agent_turn(pane)`
    /// instead of `send_terminal_input`. `None` for every other focus
    /// state (terminal focus, idle agent pane), where the button is "Send".
    pub agent_stop_pane: Option<crate::workspace::main_area::pane_tree::PaneId>,
    /// Set when the focused pane is an Agent chat pane that advertises session
    /// modes. Carries that pane's id + its mode state so the bottom-input
    /// renders the mode chip to the left of the Submit button. `None` for a
    /// terminal-pane focus (or an agent pane the adapter gave no modes), where
    /// only the Submit button shows.
    pub agent_mode: Option<(
        crate::workspace::main_area::pane_tree::PaneId,
        daruda_acp::ModeStateView,
    )>,
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
    /// Latest 5h / 7d plan-rate snapshot. Default-constructed (both
    /// windows `None`) before the first successful `/api/oauth/usage`
    /// fetch, in which case the renderer draws placeholder gauges.
    pub plan_limits: daruda_claude::PlanLimits,
    /// Latest service-status snapshot. Default is `Unknown`, which
    /// the renderer maps to a dimmed pill.
    pub service_status: daruda_claude::ServiceStatus,
    /// Locally aggregated activity (today + 7-day chart + totals).
    /// Default-constructed (empty) before the first aggregation, in
    /// which case the renderer draws zeroed activity cards.
    pub activity: daruda_claude::ActivityStats,
    /// Whether a manual usage refresh is in flight, so the ⟳ button can
    /// render a disabled / spinning state.
    pub usage_refresh_in_flight: bool,
    /// Per-lane projection of the app-wide `SkillsState` Global
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
    /// Aggregate Claude session status per lane, keyed by the
    /// lane's filesystem path so the Tasks tab can paint a
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
/// All payload variants are `Box`ed: `LeftDockSnapshot` (~416 B),
/// `RightDockSnapshot` (~776 B), and `BottomDockSnapshot` dwarf the
/// cleared `None` state — leaving them inline would inflate every
/// `DockSnapshot` slot well past clippy's variant-size threshold.
pub(in crate::workspace) enum DockSnapshot {
    Left(Box<LeftDockSnapshot>),
    Bottom(Box<BottomDockSnapshot>),
    Right(Box<RightDockSnapshot>),
    /// Initial / cleared state — `Dock::render` returns an empty
    /// element when the snap is absent (first frame safety net).
    None,
}

#[cfg(test)]
mod tests {
    use super::*;

    use gpui::{AppContext as _, ScrollHandle, TestAppContext, Window};

    /// Build a minimal but real `LeftDockSnapshot` inside a live window so
    /// the excluded GPUI handle fields (`FocusHandle` / `ScrollHandle` /
    /// `Entity<InputPanel>` / `WeakEntity<Workspace>`) are valid instances.
    /// The data fields default to empty/false so individual tests mutate
    /// only the field under examination.
    fn fixture(window: &mut Window, cx: &mut gpui::App) -> LeftDockSnapshot {
        let git_commit_input = cx.new(|cx| {
            crate::ui::InputPanel::new(crate::ui::InputPanelLayout::ActionsBelow, window, cx)
        });
        LeftDockSnapshot {
            left_dock_view: daruda_store::project::LeftDockView::default(),
            active_project_name: None,
            lanes: Vec::new(),
            projects: Vec::new(),
            groups: Vec::new(),
            active: daruda_store::project::LaneRef::default(),
            git_status_cache: std::collections::HashMap::new(),
            git_stage_in_flight: false,
            git_op_in_flight: false,
            git_collapsed_dirs: std::collections::HashSet::new(),
            git_changes_cursor: None,
            git_changes_panel_focus: cx.focus_handle(),
            focused_file_selection: None,
            git_changes_scroll_handle: ScrollHandle::new(),
            git_commit_input,
            files_panel_focus: cx.focus_handle(),
            files_scroll_handle: UniformListScrollHandle::new(),
            files_icon_color_mode: daruda_config::IconColorMode::default(),
            cached_visible: Arc::new(Vec::new()),
            root_kind: None,
            claude_status_per_lane: std::collections::HashMap::new(),
            claude_per_session_per_lane: std::collections::HashMap::new(),
            claude_active_session_id: None,
            claude_install_banner_visible: false,
            workspace: WeakEntity::new_invalid(),
        }
    }

    #[gpui::test]
    fn identical_content_does_not_differ(cx: &mut TestAppContext) {
        crate::test_support::init_gpui_component(cx);
        cx.add_window(|window, cx| {
            let a = fixture(window, cx);
            let b = fixture(window, cx);
            // Distinct handle instances (separate `cx.focus_handle()` /
            // `InputPanel` entities) must NOT trip the diff — only content
            // matters. `cached_visible` shares the same `Arc` only when one
            // is cloned from the other, so clone `b`'s into a copy to model
            // the "cache reused" path.
            let b = LeftDockSnapshot {
                cached_visible: a.cached_visible.clone(),
                ..b
            };
            assert!(
                !a.content_differs(&b),
                "identical content (modulo handles + shared cache Arc) must not differ"
            );
            gpui::Empty
        });
    }

    #[gpui::test]
    fn claude_status_change_differs(cx: &mut TestAppContext) {
        crate::test_support::init_gpui_component(cx);
        cx.add_window(|window, cx| {
            let a = fixture(window, cx);
            let mut b = fixture(window, cx);
            b.cached_visible = a.cached_visible.clone();
            b.claude_status_per_lane.insert(
                daruda_store::project::LaneRef::default(),
                daruda_claude::SessionStatus::Working,
            );
            assert!(
                a.content_differs(&b),
                "a changed claude_status_per_lane must trip content_differs"
            );
            gpui::Empty
        });
    }

    #[gpui::test]
    fn git_field_change_differs(cx: &mut TestAppContext) {
        crate::test_support::init_gpui_component(cx);
        cx.add_window(|window, cx| {
            let a = fixture(window, cx);
            let mut b = fixture(window, cx);
            b.cached_visible = a.cached_visible.clone();
            b.git_op_in_flight = true;
            assert!(
                a.content_differs(&b),
                "a changed git field must trip content_differs"
            );
            gpui::Empty
        });
    }

    #[gpui::test]
    fn lanes_change_differs(cx: &mut TestAppContext) {
        crate::test_support::init_gpui_component(cx);
        cx.add_window(|window, cx| {
            let a = fixture(window, cx);
            let mut b = fixture(window, cx);
            b.cached_visible = a.cached_visible.clone();
            b.lanes.push(crate::lane::Lane::default_for_project(
                0,
                std::path::PathBuf::from("/tmp/scratch"),
            ));
            assert!(
                a.content_differs(&b),
                "a changed lanes vec must trip content_differs"
            );
            gpui::Empty
        });
    }

    #[gpui::test]
    fn distinct_cache_arc_differs(cx: &mut TestAppContext) {
        crate::test_support::init_gpui_component(cx);
        cx.add_window(|window, cx| {
            // Two independently-allocated empty `Arc`s have equal content
            // but distinct pointers — `Arc::ptr_eq` (intentionally) reports
            // them as a difference, modeling a cache rebuild.
            let a = fixture(window, cx);
            let b = fixture(window, cx);
            assert!(
                a.content_differs(&b),
                "distinct cached_visible Arc must trip content_differs (ptr_eq)"
            );
            gpui::Empty
        });
    }
}
