//! A live run as this window reads it: which of its lanes holds one, and
//! what a row says about it.
//!
//! The run itself belongs to the process ([`crate::runs::RunsHost`]), keyed
//! by worktree, so a window that did not start it — or outlived the one that
//! did — sees, answers and stops it the same way.

use std::path::Path;

use daruda_store::project::LaneRef;
use gpui::App;

use super::Workspace;
use crate::runs::{FlowRun, Lull, NodeRunState, ParkedAsk, RunView, RunsHost};
use crate::surface::strings as s;

impl Workspace {
    /// This window's lane at `worktree`, if it holds one.
    pub(in crate::workspace) fn lane_at(&self, worktree: &Path) -> Option<LaneRef> {
        self.projects
            .lanes()
            .find(|(_, _, lane)| lane.path == worktree)
            .map(|(lane_ref, _, _)| lane_ref)
    }

    /// The run going in `lane`, whoever started it.
    pub(in crate::workspace) fn flow_run_in<'a>(
        &self,
        lane: LaneRef,
        cx: &'a App,
    ) -> Option<&'a FlowRun> {
        RunsHost::flow_run(&self.lane_for(lane)?.path, cx)
    }

    pub(in crate::workspace) fn is_flow_running_in(&self, lane: LaneRef, cx: &App) -> bool {
        self.flow_run_in(lane, cx).is_some()
    }
}

/// What the run is doing, in the only terms that stay true: which nodes are
/// on, and whether one is on a later try. Several nodes run together, so it
/// names each of them; between nodes it says what came last.
pub(in crate::workspace) fn describe(view: &RunView) -> String {
    let running = view.running();
    if running.is_empty() {
        return match view.lull() {
            Lull::Starting => s::flow::stage_starting(),
            Lull::After { node, attempt } => {
                describe_node(node, NodeRunState::Running { attempt: *attempt })
            }
            Lull::Rederiving { gate } => s::flow::stage_rederiving(gate.as_str()),
        };
    }
    running
        .iter()
        .map(|(node, state)| describe_node(node, *state))
        .collect::<Vec<_>>()
        .join(", ")
}

/// `attempt` is only worth saying past the first — every node has a first
/// try, and "try 1" on every row buries the one that says "try 3".
fn describe_node(node: &daruda_flow::NodeId, state: NodeRunState) -> String {
    match state {
        NodeRunState::Fixing => s::flow::stage_fixing(node.as_str()),
        NodeRunState::Running { attempt } if attempt > 1 => {
            s::flow::stage_node_retry(node.as_str(), attempt)
        }
        _ => s::flow::stage_node(node.as_str()),
    }
}

/// The line a question is shown under: who is asking, and for what.
pub(in crate::workspace) fn describe_question(question: &ParkedAsk) -> String {
    s::flow::stage_asking(question.node().as_str(), question.tool())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use daruda_flow::event::FlowEvent;

    use super::*;
    use crate::runs::FlowSource;

    fn view() -> RunView {
        let run_dir = PathBuf::from("/run");
        RunView::new(
            run_dir.clone(),
            FlowSource::Resumed { run_dir },
            vec!["design".into(), "build".into()],
        )
    }

    fn started(node: &str, attempt: u32) -> FlowEvent {
        FlowEvent::NodeStarted {
            node: node.into(),
            attempt,
        }
    }

    #[test]
    fn a_run_names_every_node_under_way_in_flow_order() {
        let mut view = view();
        assert_eq!(describe(&view), s::flow::stage_starting());
        view.advance(&started("build", 2));
        view.advance(&started("design", 1));
        assert_eq!(
            describe(&view),
            format!(
                "{}, {}",
                s::flow::stage_node("design"),
                s::flow::stage_node_retry("build", 2)
            )
        );
    }

    #[test]
    fn between_nodes_it_says_what_came_last() {
        let mut view = view();
        view.advance(&started("design", 3));
        view.advance(&FlowEvent::NodePassed {
            node: "design".into(),
            attempt: 3,
        });
        assert_eq!(describe(&view), s::flow::stage_node_retry("design", 3));
        view.advance(&FlowEvent::FixEnded {
            gate: "build".into(),
            failure: None,
        });
        assert_eq!(describe(&view), s::flow::stage_rederiving("build"));
    }
}
