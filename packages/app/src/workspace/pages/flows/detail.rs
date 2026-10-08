//! The Flows page's detail: the one flow graph or past run the window holds.
//!
//! It belongs to no lane's tabs, but it runs in one — the worktree the page
//! was browsing when it opened. Running, validating and a run's colours all
//! use that lane, so switching worktrees under it changes none of them.

use daruda_store::project::{FlowDetailTarget, LaneRef};
use std::path::PathBuf;

use gpui::{App, Context, Entity, SharedString, Window};

use crate::workspace::Workspace;
use crate::workspace::pages::flows::graph::FlowGraphView;

/// Names the Flows page's detail. Never reused, so a callback holding an
/// older one finds nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(in crate::workspace) struct FlowDetailId(pub u64);

pub(in crate::workspace) enum FlowDetailBody {
    /// The view holds the file's path; nothing here mirrors it.
    Graph(Entity<FlowGraphView>),
    /// A past run, read-only.
    Run(RunDetail),
}

pub(in crate::workspace) struct RunDetail {
    pub dir: PathBuf,
    /// When it started, as the history worded it. Empty when the history no
    /// longer lists it.
    pub started: SharedString,
    pub report: RunReport,
}

/// The run's `run.md`, read off the main thread.
pub(in crate::workspace) enum RunReport {
    Loading,
    Loaded(SharedString),
    /// Not there, or not readable.
    Missing,
}

pub(in crate::workspace) struct FlowDetail {
    pub id: FlowDetailId,
    pub lane: LaneRef,
    pub body: FlowDetailBody,
}

/// At most one detail: opening another goes through the leave guard first.
#[derive(Default)]
pub(in crate::workspace) struct FlowsPage {
    pub detail: Option<FlowDetail>,
}

impl Workspace {
    /// The detail the Flows page holds, if any.
    pub(in crate::workspace) fn flow_detail_id(&self) -> Option<FlowDetailId> {
        self.pages.flows.detail.as_ref().map(|detail| detail.id)
    }

    /// The graph detail `id` names, with the lane it runs in. `None` once
    /// `id` is no longer the page's detail.
    pub(in crate::workspace) fn flow_graph(
        &self,
        id: FlowDetailId,
    ) -> Option<(LaneRef, Entity<FlowGraphView>)> {
        let detail = self.pages.flows.detail.as_ref().filter(|d| d.id == id)?;
        match &detail.body {
            FlowDetailBody::Graph(view) => Some((detail.lane, view.clone())),
            FlowDetailBody::Run(_) => None,
        }
    }

    /// The open graph, whatever its id — for what reaches it by file or by
    /// run rather than by the id a gesture carries.
    pub(in crate::workspace) fn open_graph(
        &self,
    ) -> Option<(FlowDetailId, LaneRef, Entity<FlowGraphView>)> {
        let detail = self.pages.flows.detail.as_ref()?;
        match &detail.body {
            FlowDetailBody::Graph(view) => Some((detail.id, detail.lane, view.clone())),
            FlowDetailBody::Run(_) => None,
        }
    }

    /// The open graph when it draws `path`.
    pub(in crate::workspace) fn open_graph_of(
        &self,
        path: &std::path::Path,
        cx: &App,
    ) -> Option<(FlowDetailId, LaneRef, Entity<FlowGraphView>)> {
        self.open_graph()
            .filter(|(_, _, view)| view.read(cx).path() == path)
    }

    /// What the detail shows, as the workspace file records it.
    pub(in crate::workspace) fn flow_detail_target(&self, cx: &App) -> Option<FlowDetailTarget> {
        let detail = self.pages.flows.detail.as_ref()?;
        let project = self.projects.get(detail.lane.project)?.uuid;
        let lane = detail.lane.lane;
        Some(match &detail.body {
            FlowDetailBody::Graph(view) => FlowDetailTarget::Graph {
                project,
                lane,
                path: view.read(cx).path().to_path_buf(),
            },
            FlowDetailBody::Run(run) => FlowDetailTarget::Run {
                project,
                lane,
                dir: run.dir.clone(),
            },
        })
    }

    /// Reopen the detail a saved workspace showed. A worktree that is gone
    /// leaves the page on its list; a file that is gone still opens, and says
    /// so.
    pub(in crate::workspace) fn restore_flow_detail(
        &mut self,
        target: &FlowDetailTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (FlowDetailTarget::Graph { project, lane, .. }
        | FlowDetailTarget::Run { project, lane, .. }) = target;
        let Some(project) = self.project_by_uuid(*project).map(|p| p.id) else {
            return;
        };
        let lane = LaneRef {
            project,
            lane: *lane,
        };
        if self.lane_for(lane).is_none() {
            return;
        }
        match target {
            FlowDetailTarget::Graph { path, .. } => {
                self.install_flow_graph(lane, path, window, cx);
            }
            FlowDetailTarget::Run { dir, .. } => self.install_run_detail(lane, dir.clone(), cx),
        }
    }
}
