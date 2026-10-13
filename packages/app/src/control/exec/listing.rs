//! `/list` and `/brief`: the flat walk over every window, grouped into the
//! listing or counted into the brief.

use gpui::App;

use crate::control::result::{
    Activity, BriefSummary, ChatSummary, Health, LaneGroup, Listing, ProjectGroup, WindowGroup,
};
use crate::window_registry::WindowRegistry;
use daruda_store::project::ProjectId;

/// One row of the flat walk, before grouping.
struct Row {
    window: u32,
    /// Identity, not the name: two projects opened from different directories
    /// can share a basename, and grouping by name would merge them into one
    /// listing section that misreports the structure.
    project_id: ProjectId,
    project: String,
    lane: String,
    lane_order: u32,
    summary: ChatSummary,
}

pub(crate) fn listing(cx: &mut App) -> Listing {
    Listing {
        windows: group(collect_rows(cx)),
        omitted: 0,
    }
}

pub(crate) fn brief(cx: &mut App) -> BriefSummary {
    let rows = collect_rows(cx);
    BriefSummary {
        working: count(&rows, |s| s.activity == Activity::Working),
        awaiting_permission: count(&rows, |s| s.activity == Activity::AwaitingPermission),
        error: count(&rows, |s| s.health == Health::Error),
        total: rows.len() as u32,
    }
}

fn count(rows: &[Row], pred: impl Fn(&ChatSummary) -> bool) -> u32 {
    rows.iter().filter(|r| pred(&r.summary)).count() as u32
}

fn collect_rows(cx: &mut App) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut window = 0u32;
    // Every workspace: a listing answers "what is the user working on?", and
    // it is `control_snapshot` — through `lane_agent_chats` — that keeps the
    // orchestrator's own chat out of it, not the choice of walk here.
    WindowRegistry::for_each_workspace(cx, |ws, _win, cx| {
        for (lane_ref, summary) in ws.control_snapshot(cx) {
            rows.push(Row {
                window,
                project_id: lane_ref.project,
                project: ws.control_project_name(lane_ref.project),
                lane: ws.control_lane_name(lane_ref),
                lane_order: ws.control_lane_tab_order(lane_ref),
                summary,
            });
        }
        window += 1;
    });
    // `runtimes` is a HashMap, and window registration order is not a
    // guarantee we want ordinals to depend on. Sort once, here.
    rows.sort_by(|a, b| {
        a.window
            .cmp(&b.window)
            // Name first so the listing reads alphabetically, then id so two
            // same-named projects stay apart instead of interleaving.
            .then_with(|| a.project.cmp(&b.project))
            .then_with(|| a.project_id.cmp(&b.project_id))
            .then_with(|| a.lane_order.cmp(&b.lane_order))
            .then_with(|| a.summary.target.pane.cmp(&b.summary.target.pane))
    });
    rows
}

fn group(rows: Vec<Row>) -> Vec<WindowGroup> {
    let mut windows: Vec<WindowGroup> = Vec::new();
    // Tracks which project the open group belongs to, since `ProjectGroup`
    // carries only the name and two projects can share one.
    let mut open_project: Option<ProjectId> = None;
    for row in rows {
        let win = match windows.last_mut() {
            Some(w) if w.index == row.window => w,
            _ => {
                windows.push(WindowGroup {
                    index: row.window,
                    projects: Vec::new(),
                });
                windows.last_mut().expect("just pushed")
            }
        };
        let proj = match win.projects.last_mut() {
            Some(p) if open_project == Some(row.project_id) => p,
            _ => {
                open_project = Some(row.project_id);
                win.projects.push(ProjectGroup {
                    name: row.project,
                    lanes: Vec::new(),
                });
                win.projects.last_mut().expect("just pushed")
            }
        };
        match proj.lanes.last_mut() {
            Some(l) if l.name == row.lane => l.chats.push(row.summary),
            _ => proj.lanes.push(LaneGroup {
                name: row.lane,
                chats: vec![row.summary],
            }),
        }
    }
    windows
}
