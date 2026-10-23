//! Snapshot types for dock rendering.
//!
//! Each `*DockSnapshot` struct carries a point-in-time copy of the
//! `Workspace` fields that the corresponding dock's `impl Render`
//! needs to build its element tree.  Snapshots are staged by
//! `render/docks.rs` at the start of every `Workspace::render` cycle
//! via `Entity<Dock>::update(cx, |d, _| d.snap = ...)`.
//!
//! **Why snapshots?**
//! `impl Render for Dock` runs inside a `Context<Dock>`.  Reading
//! `Workspace` state through `WeakEntity::upgrade()` + `.read(cx)`
//! from inside that context would re-enter the entity graph.
//! Staging a plain-data snapshot avoids all re-entry and keeps the
//! render closure free of entity borrows.

use crate::workspace::main_area::file_view_pane::DiffSource;
use gpui::{FocusHandle, UniformListScrollHandle, WeakEntity};

use crate::files::tree::EntryKind;
use crate::workspace::left_dock::file_tree_ops::VisibleEntry;

use super::diff_policy::{ByPointer, Handle, PerFrame};
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
    pub display_alias: Option<gpui::SharedString>,
    pub rail_grouped_lanes: std::collections::BTreeSet<daruda_store::project::LaneId>,
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
    /// clicks the project header.
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
///
/// Every field takes part in the notify-on-change diff unless it is wrapped in
/// a [`diff_policy`](super::diff_policy) marker saying why it must not.
#[derive(PartialEq)]
pub(in crate::workspace) struct LeftDockSnapshot {
    pub metrics: crate::ui::theme::UiMetrics,
    pub expanded: bool,
    pub individual_pane_lanes: std::collections::HashSet<daruda_store::project::LaneRef>,
    pub open_tabs: std::collections::HashMap<daruda_store::project::LaneRef, usize>,
    pub named_sessions: std::collections::HashMap<
        daruda_store::project::LaneRef,
        Vec<crate::workspace::left_dock::sessions::SessionRow>,
    >,
    pub rail_panes: std::collections::HashMap<
        daruda_store::project::LaneRef,
        Vec<crate::workspace::left_dock::rail::PaneSnapshot>,
    >,
    pub focused_pane: u64,
    pub left_dock_view: daruda_store::project::LeftDockView,
    pub workspace_page: Option<crate::workspace::pages::Page>,
    pub lanes: Vec<crate::lane::Lane>,
    /// Every project in the workspace, in `tab_order` order. Drives
    /// the multi-project tree render — each entry's `lanes` slice
    /// is shown under its project header. Empty when the workspace is
    /// in the Landing state.
    pub projects: Vec<ProjectSnapshot>,
    /// Every group in the workspace, in `tab_order` order. Group
    /// headers interleave with ungrouped projects in the top-level
    /// tree via the shared `tab_order` pool.
    pub groups: Vec<GroupSnapshot>,
    pub active: daruda_store::project::LaneRef,
    /// Per-lane working-tree status. Separate from `git_tracking_cache`
    /// because the two axes are refreshed by different events.
    pub git_worktree_cache: std::collections::HashMap<
        daruda_store::project::LaneRef,
        crate::lane::git::GitWorktreeStatus,
    >,
    /// Per-lane branch / upstream / ahead / behind.
    pub git_tracking_cache:
        std::collections::HashMap<daruda_store::project::LaneRef, crate::lane::git::GitTracking>,
    pub git_stage_in_flight: bool,
    /// Mirrors `Workspace::git_op_in_flight` — true while a Fetch / Push /
    /// Commit / Amend is running. Drives `loading + disabled` on the Fetch
    /// and Push buttons in the Git Changes header.
    pub git_op_in_flight: bool,
    /// Active lane's collapsed dir set (Git Changes view). Keyed by
    /// lane-relative dir string.
    pub git_collapsed_dirs: std::collections::HashSet<String>,
    /// Active lane's changes since its base; `None` until first read.
    pub git_against_base: Option<
        std::sync::Arc<
            Result<crate::lane::git::base::AgainstBase, crate::lane::git::base::BaseProblem>,
        >,
    >,
    /// Active lane's against-base section is folded.
    pub git_against_base_collapsed: bool,
    /// Active lane's keyboard cursor in the Git Changes view —
    /// repo-root-relative path of the focused row (or None if no row is
    /// focused). Drives the visual cursor highlight.
    pub git_changes_cursor: Option<std::path::PathBuf>,
    /// Focus handle bound to `key_context("GitChanges")`. Track this on
    /// the Git Changes body so its keyboard shortcuts fire only when
    /// the panel holds focus.
    pub git_changes_panel_focus: Handle<FocusHandle>,
    /// `(lane, path, source)` of the focused file viewer's pane,
    /// or `None` when no file pane is focused. Dock rows render a
    /// "selected" background when this triple matches.
    pub focused_file_selection: Option<(
        daruda_store::project::LaneRef,
        std::path::PathBuf,
        DiffSource,
    )>,
    pub git_changes_scroll_handle: Handle<gpui::UniformListScrollHandle>,
    /// Scroll handle for the Lanes view card list — the body below the
    /// (fixed) view header. Shared with its scrollbar thumb overlay.
    pub lanes_scroll_handle: Handle<gpui::ScrollHandle>,
    pub git_commit_input: Handle<gpui::Entity<crate::ui::InputPanel>>,
    pub files_panel_focus: Handle<FocusHandle>,
    pub files_scroll_handle: Handle<UniformListScrollHandle>,
    pub files_icon_color_mode: daruda_config::IconColorMode,
    pub cached_visible: ByPointer<Vec<VisibleEntry>>,
    pub root_kind: Option<EntryKind>,
    /// Aggregate agent status per lane. Keyed by `LaneRef`
    /// so lane ids in distinct projects (each project numbers its
    /// lanes from 0) don't collide. Empty when the
    /// `claude_status.enable` config flag is off, no agent session is
    /// running, or the lane has no matching cwd.
    pub agent_status_per_lane:
        std::collections::HashMap<daruda_store::project::LaneRef, daruda_agent::SessionStatus>,
    pub agent_outcome_per_lane:
        std::collections::HashMap<daruda_store::project::LaneRef, daruda_agent::AgentOutcome>,
    /// Per-session statuses for lanes that have ≥ 2 active agent
    /// sessions. Sub-row badges read this. Lanes with 0 or 1 sessions are
    /// absent (the leading indicator covers them).
    /// Keyed by `LaneRef` for the same cross-project reason.
    pub agent_per_session_per_lane: std::collections::HashMap<
        daruda_store::project::LaneRef,
        Vec<(String, daruda_agent::SessionStatus)>,
    >,
    /// `session_id` of the agent process living inside the focused pane.
    /// Used by sub-row badge render to highlight the session attached to
    /// the active terminal. `None` when the
    /// focused pane has no agent descendant or the tracker hasn't
    /// resolved it yet.
    pub agent_active_session_id: Option<String>,
    pub workspace: Handle<WeakEntity<Workspace>>,
}

impl LeftDockSnapshot {
    /// True when the left-dock-relevant *content* differs from `prev`.
    ///
    /// Derived rather than hand-written: a field added to this struct joins the
    /// diff automatically, and one that must stay out says so at its own
    /// declaration through a [`diff_policy`](super::diff_policy) wrapper. The
    /// cost of forgetting is then an unnecessary repaint rather than a dock
    /// that never repaints at all.
    ///
    /// Wired into left-dock render staging (`render/docks.rs`) as the
    /// notify-on-change comparator: the cached dock is marked dirty only when
    /// this reports a difference. This is the sole left-dock invalidation
    /// mechanism — a plain workspace `cx.notify()` re-stages the snapshot and
    /// lets this diff decide. (The status pulse is the one exception: it
    /// dirties the dock directly to animate badges, whose frames are not part
    /// of this snapshot.)
    pub(super) fn content_differs(&self, prev: &Self) -> bool {
        self != prev
    }
}

// ----------------------------------------------------------------
// Bottom dock
// ----------------------------------------------------------------

/// Read-only projection of one queued prompt for the bottom-dock
/// queued-prompt strip. The [`daruda_chat::view::PromptId`]
/// is the single source of truth on the `AgentChatView`; this mirrors it plus
/// the prompt text so the strip renders without re-entering the view entity,
/// and routes a per-item removal back to that id via `remove_queued_prompt`.
#[derive(Clone, PartialEq)]
pub(in crate::workspace) struct QueuedPromptView {
    pub id: daruda_chat::view::PromptId,
    pub text: String,
    /// True when this prompt is the one currently pulled into the composer for
    /// editing (mirrors `AgentChatView::editing_prompt == Some(id)`). The strip
    /// renders this row in an "editing…" state with a cancel affordance instead
    /// of the ✎ / × buttons. `PartialEq` on this field drives the bottom dock's
    /// `.cached()` re-stage when the edit target changes.
    pub editing: bool,
    /// True when this prompt was parked by a Stop (`AgentChatView::paused_prompts`)
    /// rather than sitting in the live queue. The strip renders parked rows in a
    /// dimmed "Paused" state and surfaces a Resume affordance; parked rows sort
    /// ahead of live ones. `PartialEq` re-stages the dock when parking changes.
    pub paused: bool,
}

/// The focused Agent chat pane's queued-prompt strip, as the bottom dock sees
/// it: which pane to route the row buttons back to, the rows themselves, and
/// whether the resume gesture is armed. One struct rather than a tuple plus a
/// sibling flag, so "armed" cannot exist without the rows it describes.
#[derive(Clone, PartialEq)]
pub(in crate::workspace) struct QueuedPromptsSnapshot {
    pub pane_id: crate::workspace::main_area::pane_tree::PaneId,
    pub prompts: Vec<QueuedPromptView>,
    /// Mirrors `AgentChatView::resume_armed()` — an empty-composer Enter has
    /// armed the parked queue and the header asks for a second one.
    pub resume_armed: bool,
}

/// The CLI status row a mirrored pane shows in place of the composer.
#[derive(PartialEq)]
pub(in crate::workspace) struct CliSnapshot {
    pub pane_id: super::super::main_area::pane_tree::PaneId,
    /// `None` when the run the pane mirrors no longer exists — the task was
    /// deleted or re-run — which offers nothing to continue.
    pub process: Option<daruda_store::tasks::CliProcessState>,
    pub loading: bool,
}

/// Point-in-time copy of `Workspace` fields consumed by the bottom
/// dock's `impl Render`.
///
/// `PartialEq` drives the bottom dock's `.cached()` coherence: the
/// render staging compares each frame's snapshot against the last and
/// only fires `cx.notify(bottom_dock)` when content actually changed,
/// so the 250 ms status pulse (which leaves this snapshot identical)
/// doesn't repaint the dock. Fields that must stay out of that diff carry a
/// [`diff_policy`](super::diff_policy) marker, same as the other two docks.
#[derive(PartialEq)]
pub(in crate::workspace) struct BottomDockSnapshot {
    pub agent_cli_snapshot: Option<CliSnapshot>,
    pub terminal_input_visible: bool,
    pub active_tab_id: Option<daruda_store::panels::TabId>,
    pub tab_summaries: Vec<(daruda_store::panels::TabId, String, usize)>,
    pub active_tab_widgets: Vec<daruda_store::panels::MacroKey>,
    /// Mirror of `Workspace::panels_grid_columns` — column count for
    /// the bottom-dock macro tile grid. Already clamped (>= 1).
    pub grid_columns: u8,
    /// Current displayed bottom dock height, including transient attachment space.
    /// Used by the tab-strip
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
    /// Names only: image payloads never enter render snapshots.
    pub attachment_draft: Option<(crate::workspace::main_area::pane_tree::PaneId, Vec<String>)>,
    /// Set when the focused pane is an Agent chat pane that advertises session
    /// modes. Carries that pane's id + its mode state so the bottom-input
    /// renders the mode chip to the left of the Submit button. `None` for a
    /// terminal-pane focus (or an agent pane the adapter gave no modes), where
    /// only the Submit button shows.
    pub agent_mode: Option<(
        crate::workspace::main_area::pane_tree::PaneId,
        daruda_acp::ModeStateView,
    )>,
    /// Set when the focused pane is an Agent chat pane advertising select config
    /// options of the Model / ThoughtLevel category. Carries the pane id + those
    /// options so the bottom input renders a chip per option (model, effort)
    /// beside the Submit button. `None` for a terminal-pane focus or an agent
    /// pane with no such options. Mode is excluded — it renders via `agent_mode`.
    pub agent_config_options: Option<(
        crate::workspace::main_area::pane_tree::PaneId,
        Vec<daruda_acp::ConfigOptionView>,
    )>,
    /// Set when the focused pane is an Agent chat pane with a non-empty prompt
    /// queue. Carries the pane id + the queued prompts (in FIFO order) so the
    /// bottom dock renders the queued-prompt strip above the input and routes
    /// each removal / clear-all back to that pane. `None` for a terminal-pane
    /// focus, an agent pane with an empty queue, or the Landing state — the
    /// strip is then not rendered.
    pub queued_prompts: Option<QueuedPromptsSnapshot>,
    /// Shell flavour of the focused pane's PTY. Drives drag-and-drop path
    /// quoting in the terminal input — Posix backslash/single-quote rules,
    /// fish, PowerShell, and cmd.exe all differ.
    pub shell: daruda_core::shell::quote::Shell,
    pub workspace: Handle<WeakEntity<Workspace>>,
}

// ----------------------------------------------------------------
// Right dock
// ----------------------------------------------------------------

/// Point-in-time copy of `Workspace` fields consumed by the right
/// dock's `impl Render`.
#[derive(PartialEq)]
pub(in crate::workspace) struct RightDockSnapshot {
    /// Active right-panel tab. The tab strip in the dock header reads
    /// this to highlight the current tab; the body match-arm reads it
    /// to pick the renderer.
    pub right_dock_view: daruda_store::project::RightDockView,
    /// Back-reference to the owning `Workspace`, mirrored from the
    /// dock entity. Tab-strip click handlers upgrade this to dispatch
    /// `set_right_dock_view` without re-entering the dock context.
    pub workspace: Handle<WeakEntity<Workspace>>,
    /// One section per auth domain worth showing, in `AccountRecipeId::all()`
    /// order. Empty when no provider is signed in, which the renderer replaces
    /// with a single notice.
    pub usage: Vec<UsageSectionSnapshot>,
    /// The focused pane's agent domain. `Exactly(recipe)` scopes the tab to
    /// one section automatically; `Any`/`Unsupported` (terminal, unresolved
    /// agent) falls back to `usage_domain_override` + a switcher. Computed
    /// once per snapshot build by `AccountDomain::for_pane` and reused here
    /// rather than recomputed in the renderer.
    pub focused_agent_domain: crate::workspace::main_area::pane::AccountDomain,
    /// Mirrors `AgentUsageState::usage_domain_override` — the switcher's last
    /// manual pick, read-only from this side.
    pub usage_domain_override: Option<daruda_store::accounts::AccountRecipeId>,
    /// `[usage] stop_on_overage`, shown as the Claude section's switch.
    pub stop_on_overage: bool,
    /// Locally aggregated activity (7-day turns + tokens charts), one entry
    /// per domain in `usage` that has any — a domain with no activity log
    /// (or none aggregated yet) is simply absent, not zeroed.
    pub activity: Vec<(
        daruda_store::accounts::AccountRecipeId,
        daruda_agent::ActivityStats,
    )>,
    /// Up to 10 recent past sessions per domain, restricted to the active
    /// Lane — one entry per domain in `usage` that has any match, most-recent
    /// first. See [`RestorableSession`].
    pub recent_sessions: Vec<(
        daruda_store::accounts::AccountRecipeId,
        Vec<RestorableSession>,
    )>,
    /// Whether a manual usage refresh is in flight, so the ⟳ button can
    /// render a disabled / spinning state.
    pub usage_refresh_in_flight: bool,
    /// Per-lane projection of the app-wide `GlobalSkills`
    /// for the Skills tab renderer. Carried by-value so the panel
    /// renderer never re-enters the workspace entity and never reads
    /// the Global from inside `Render::render`.
    pub skills: crate::agent::skills::SkillsSnapshot,
    /// Search query input rendered atop the Skills tab. Entity is
    /// shared with the Workspace; the renderer just embeds it inline.
    pub skill_search_input: Handle<gpui::Entity<crate::ui::InputState>>,
    /// Captured search text at snap build time. Lowercase substring
    /// match against skill `name` and `frontmatter.description` filters
    /// every scope (Project / Personal / Plugin) simultaneously.
    pub skill_search_query: String,
    /// Plugin ids whose accordion section in the Skills tab is open.
    /// Renderer treats default (empty) as "all collapsed". Cloned per
    /// frame — small set, cheap.
    pub skill_plugin_expanded: std::collections::HashSet<String>,
    pub sections: crate::workspace::right_dock::section::DockSections,
    /// Snapshot of the Tasks tab's task list. Plain-data clone — the
    /// renderer never touches `Workspace::tasks` directly.
    pub tasks: daruda_store::tasks::TasksBook,
    pub task_browser: crate::workspace::right_dock::tasks::TaskBrowserSnapshot,
    pub task_projects: TaskProjects,
    pub task_agents: std::sync::Arc<[daruda_config::AgentDefinition]>,
    /// Per-session Claude status, keyed by `session_id`. Mirrors the
    /// `ClaudeStatusStore` slice that the Tasks tab needs to render
    /// the `⟳ / ● / ⚠` glyph trailing each row's session-id badge.
    /// Empty when `claude_status.enable` is off or no session has
    /// reported a status yet.
    pub claude_status_per_session: std::collections::HashMap<String, daruda_agent::SessionStatus>,
    /// Per-session tool-use failure counts mirrored from
    /// `Workspace::claude.tool_use_failure_counts`. Only sessions
    /// with a positive count are present. The Tasks-tab renderer
    /// surfaces a `failures N/M` hint once a count reaches
    /// [`daruda_terminal::ux::strings::RIGHT_PANEL_TASK_FAILURE_DISPLAY_THRESHOLD`].
    pub tool_use_failure_counts: std::collections::HashMap<String, u32>,
    /// Reference instant captured once per frame so every row in the
    /// Tasks tab computes its live duration against the same `now`.
    /// `Utc::now()` per row would drift between calls within a single
    /// frame and break the `2m 14s → 2m 15s` invariant for sibling
    /// rows that update on the same tick.
    pub now: PerFrame<chrono::DateTime<chrono::Utc>>,
    /// Scroll handle shared between the right-panel body's
    /// `overflow_y_scroll` and the scrollbar thumb overlay. Cloned from
    /// `RightDockViews::scroll_handle` each frame.
    pub right_panel_scroll_handle: Handle<gpui::ScrollHandle>,
    /// Snapshot of `Workspace::mcp` for the Tools tab renderer.
    /// Carried by-value so the panel renderer never re-enters the
    /// workspace entity.
    pub mcp: crate::agent::mcp::McpSnapshot,
    pub library: crate::workspace::right_dock::library::LibraryView,
}

/// One auth domain's block in the Usage tab: whose account it is, what the last
/// poll found, and that provider's service health.
#[derive(Clone, Debug, PartialEq)]
pub struct UsageSectionSnapshot {
    pub recipe: daruda_store::accounts::AccountRecipeId,
    /// Display-only identity — the managed account's email, or a "System"
    /// fallback naming the domain's ambient home. Purely informational: no
    /// selector, no `SwitchPaneAccount` dispatch.
    pub account_label: gpui::SharedString,
    /// `Pending` draws a placeholder; a failed refresh keeps the previous
    /// numbers, marked stale. `SignedOut` never reaches here — such a domain
    /// has no section at all.
    pub outcome: daruda_agent::UsageOutcome,
    /// `None` before this domain's first status fetch lands, which the renderer
    /// maps to a dimmed pill.
    pub service_status: Option<daruda_agent::ServiceStatus>,
}

/// A past session resolved against a Lane already open in this workspace
/// window, ready to restore into a new pane. `agent_id` is the domain's
/// configured default agent — the session's original exact agent variant
/// isn't tracked.
#[derive(Clone, Debug, PartialEq)]
pub struct RestorableSession {
    pub session_id: String,
    pub agent_id: String,
    /// Account the activity cache entry belongs to. Restore must preserve this
    /// so a managed-account session reloads under the same config dir it came
    /// from, not the domain default.
    pub account: daruda_store::accounts::AccountSelection,
    pub lane_ref: daruda_store::project::LaneRef,
    /// Best available human title, or `None` when the renderer should fall
    /// back to the prompt preview and then the cwd's last path component.
    pub title: Option<gpui::SharedString>,
    /// One-line user prompt preview for sessions without a generated title.
    pub prompt_preview: Option<gpui::SharedString>,
    /// Git branch captured from the session log, shown as row metadata.
    pub git_branch: Option<gpui::SharedString>,
    pub cwd: std::path::PathBuf,
    pub last_active: std::time::SystemTime,
}

impl RestorableSession {
    /// Best provider-neutral row label before the UI falls back to lane/cwd
    /// context: captured title first, then latest prompt preview.
    pub(in crate::workspace) fn display_title(&self) -> Option<&str> {
        daruda_agent::activity::session_display_title(
            self.title.as_ref().map(|s| s.as_ref()),
            self.prompt_preview.as_ref().map(|s| s.as_ref()),
        )
    }
}

impl RightDockSnapshot {
    /// Notify-on-change diff, derived for the reason spelled out on
    /// [`LeftDockSnapshot::content_differs`]: a field joins the diff by
    /// default, and one that must not says so at its own declaration.
    ///
    /// `now` is marked [`PerFrame`]: the staging step rebuilds it every frame,
    /// so comparing it would repaint forever. It stays fresh because staging
    /// assigns the snapshot unconditionally (`render/docks.rs`) — the diff gates
    /// only the repaint. `flow_lane` is deliberately *not* marked even though
    /// nothing draws it: the lane is what a resume click sends, so two lanes
    /// whose runs happen to look alike must still re-stage the panel's
    /// handlers.
    pub(in crate::workspace) fn content_differs(&self, prev: &Self) -> bool {
        self != prev
    }
}

/// The projects the Tasks tab names: the active one, and what each project
/// open in this window is called. Taken at snapshot time so a row labels
/// its project without the renderer reading `Workspace`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TaskProjects {
    pub active: Option<daruda_store::project::ProjectUuid>,
    pub names: Vec<(daruda_store::project::ProjectUuid, String)>,
}

impl TaskProjects {
    /// `None` for a project this window does not have open.
    pub fn name(&self, uuid: daruda_store::project::ProjectUuid) -> Option<&str> {
        self.names
            .iter()
            .find(|(id, _)| *id == uuid)
            .map(|(_, name)| name.as_str())
    }
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

impl DockSnapshot {
    /// Whether `other` is the same variant carrying the same *content* — the
    /// question [`Dock::stage`] asks to decide whether to repaint.
    pub(super) fn same_content_as(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Left(a), Self::Left(b)) => !b.content_differs(a),
            (Self::Bottom(a), Self::Bottom(b)) => a == b,
            (Self::Right(a), Self::Right(b)) => !b.content_differs(a),
            // A different variant, or no prior snapshot at all, is always a
            // change — there is nothing to compare against.
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests;
