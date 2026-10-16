//! In-memory flow browser fixtures; never write into the restored repository.

use super::{FlowGrouping, FlowScope, FlowTab, listing::FlowListing};
use crate::runs::{ParkedAsk, RunView, RunsHost};
use crate::workspace::{
    Workspace,
    flow_history::{FlowHistory, FlowRunEntry},
    flow_paths::{FlowOrigin, FoundFlow},
    flow_request::FlowSource,
    pages::Page,
};
use daruda_flow::event::FlowEvent;
use gpui::{Context, Window};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FlowBrowserShot {
    Definitions,
    Grouped,
    Empty,
    Runs,
    Asking,
}

impl Workspace {
    /// A placeholder project when none was restored: a run is held by a
    /// worktree, so a capture needs one to show a run in.
    pub(in crate::workspace) fn ensure_project_for_shot(&mut self) {
        if !self.projects.is_empty() {
            return;
        }
        let project = crate::project::Project::bootstrap_placeholder(
            self.next_project_id,
            self.data_dir.clone(),
        );
        self.next_project_id += 1;
        self.set_active(daruda_store::project::LaneRef {
            project: project.id,
            lane: project.lanes[0].id,
        });
        self.projects.push(project);
    }

    pub(in crate::workspace) fn seed_flow_browser_for_shot(
        &mut self,
        shot: FlowBrowserShot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use daruda_flow::marker::RunStatus;
        self.ensure_project_for_shot();
        self.flows.browser.state.scope = FlowScope::Current;
        let tab = match shot {
            FlowBrowserShot::Runs | FlowBrowserShot::Asking => FlowTab::Runs,
            _ => FlowTab::Definitions,
        };
        self.set_flow_tab(tab, cx);
        self.open_page(Page::Flows, window, cx);
        if let Some(project) = self.project_for_mut(self.active.project) {
            project.name = "daruda".into();
        }
        let root = self.data_dir.join("flow-preview");
        let files: Vec<_> = [
            ("release-validation.yaml", FlowOrigin::Repo),
            (
                "design-review-and-accessibility-check.yaml",
                FlowOrigin::Repo,
            ),
            ("daily-code-review.yaml", FlowOrigin::Project),
            ("documentation-refresh.yaml", FlowOrigin::Project),
            ("security-audit.yaml", FlowOrigin::Global),
            ("triage.yaml", FlowOrigin::Global),
        ]
        .into_iter()
        .map(|(name, origin)| FoundFlow {
            name: name.to_owned(),
            path: root.join(name),
            origin,
        })
        .collect();
        let now = std::time::SystemTime::now();
        let modified = files.iter().map(|file| (file.path.clone(), now)).collect();
        let source = FlowSource::File(files[0].path.clone());
        self.flows
            .list
            .put(self.active, FlowListing { files, modified });
        if shot == FlowBrowserShot::Grouped {
            self.flows.browser.state.grouping = FlowGrouping::Origin;
            self.flows.browser.state.toggle_group(FlowOrigin::Project);
        }
        if shot == FlowBrowserShot::Empty {
            self.flows.browser.searches[tab.index()].update(cx, |input, cx| {
                input.set_value("no matching flow", window, cx)
            });
        }
        let millis = chrono::Utc::now().timestamp_millis() as u128;
        let live_dir = root.join(crate::workspace::flow_request::run_id(millis, 42, 7));
        // Two nodes running together, as a parallel wave does; asking, each
        // holds a question of its own and both stay up at once. Folded into
        // the view before it is seeded, so no dialog covers the page.
        let nodes = [
            ("release-check", 2, "cargo test -p daruda"),
            ("docs-check", 1, "rm -rf target/doc"),
        ];
        let mut view = RunView::new(
            live_dir.clone(),
            source,
            nodes.iter().map(|(node, _, _)| (*node).into()).collect(),
        );
        for (ask_id, (node, attempt, detail)) in (1..).zip(nodes) {
            view.advance(&FlowEvent::NodeStarted {
                node: node.into(),
                attempt,
            });
            if shot == FlowBrowserShot::Asking {
                // The receiver stands in for the engine's waiting node; dropped,
                // the question would read as abandoned and the next one parked
                // would take it down. A capture process exits right after.
                let (reply, waiting) = smol::channel::bounded(1);
                std::mem::forget(waiting);
                view.park(ParkedAsk::new(
                    crate::workspace::flow_ops::seeds::shot_question(ask_id, node, detail, reply),
                ));
            }
        }
        if let Some(worktree) = self.lane_for(self.active).map(|lane| lane.path.clone()) {
            RunsHost::seed_flow_run(worktree, view, cx);
        }
        let history = [
            RunStatus::Running,
            RunStatus::Done,
            RunStatus::Failed,
            RunStatus::Crashed,
            RunStatus::Stalled,
            RunStatus::Canceled,
            RunStatus::Unknown,
        ]
        .into_iter()
        .enumerate()
        .map(|(index, status)| {
            let dir = if index == 0 {
                live_dir.clone()
            } else {
                root.join(crate::workspace::flow_request::run_id(
                    millis - index as u128 * 3600000,
                    42,
                    index as u32,
                ))
            };
            FlowRunEntry {
                dir,
                status,
                report: None,
                started: crate::surface::strings::flow::run_started_at(
                    chrono::Utc::now() - chrono::Duration::hours(index as i64),
                )
                .into(),
            }
        })
        .collect();
        self.flows
            .history
            .put(self.active, FlowHistory::seeded(history));
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn screenshot_states_cover_both_tabs_and_attention() {
        assert_ne!(FlowBrowserShot::Runs, FlowBrowserShot::Asking);
        assert_ne!(FlowBrowserShot::Definitions, FlowBrowserShot::Grouped);
    }
}
