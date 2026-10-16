//! Run state seeded straight into the workspace for `--screenshot` captures
//! and tests, without starting a real flow.

#[cfg(any(test, feature = "screenshot"))]
use std::path::PathBuf;

#[cfg(any(test, feature = "screenshot"))]
use gpui::Context;
#[cfg(feature = "screenshot")]
use gpui::Window;

#[cfg(any(test, feature = "screenshot"))]
use crate::runs::{RunView, RunsHost};
use crate::workspace::Workspace;
#[cfg(feature = "screenshot")]
use crate::workspace::command::flow_picker::FlowPurpose;
#[cfg(feature = "screenshot")]
use crate::workspace::flow_request::FlowSelection;

#[cfg(feature = "screenshot")]
use super::flow_profiles;

impl Workspace {
    /// Show the active lane's Flows page without changing the utility dock.
    /// Screenshot seeds use this windowless path; interactive entry points
    /// use `open_page` to transfer keyboard focus as well.
    #[cfg(feature = "screenshot")]
    pub(in crate::workspace) fn reveal_flows_panel(&mut self, cx: &mut Context<Self>) {
        self.set_flow_tab(crate::workspace::flow_browser::FlowTab::Runs, cx);
        self.show_page(crate::workspace::pages::Page::Flows, cx);
    }

    /// Put a killed run in the panel's history, for `--screenshot`. The
    /// one row with a way back into it — and the only state here that no
    /// scenario can reach without writing into a real repository.
    #[cfg(feature = "screenshot")]
    pub(in crate::workspace) fn seed_crashed_run_for_shot(&mut self, cx: &mut Context<Self>) {
        // The panel too, not only the history: the dock restores closed as
        // often as open, and a capture of a row behind a collapsed dock
        // shows nothing at all.
        self.reveal_flows_panel(cx);
        let dir =
            crate::workspace::flow_paths::runs_dir(&self.active_lane_root().unwrap_or_default());
        let shot = crate::workspace::flow_history::FlowHistory::for_shot(dir);
        self.flows.history.put(self.active, shot);
        cx.notify();
    }

    /// Put the picker on its second question, for `--screenshot`.
    ///
    /// Reaches the state directly instead of picking a flow: picking one
    /// under `FlowPurpose::Run` *submits* it, and a capture that starts a
    /// real run leaves a run directory behind every time it is taken.
    #[cfg(feature = "screenshot")]
    pub(in crate::workspace) fn ask_flow_profile_for_shot(&mut self, cx: &mut Context<Self>) {
        let Some(sources) = self.flow_sources() else {
            return;
        };
        let Some((path, profiles)) = sources.list_flows().into_iter().find_map(|found| {
            let names = flow_profiles(&found.path);
            (!names.is_empty()).then_some((found.path, names))
        }) else {
            return;
        };
        self.overlays.flow_picker.ask_profile(
            self.active,
            FlowPurpose::Run,
            path,
            FlowSelection::default(),
            profiles,
        );
        cx.notify();
    }

    /// Put a run on screen without one running, for `--screenshot`. A run
    /// is only visible mid-flight, which no capture can wait for.
    ///
    /// `asking` seeds the parked-permission state as well — the buttons a
    /// person has to be able to read and hit, and the one part of this
    /// feature no state test can look at.
    #[cfg(feature = "screenshot")]
    pub(in crate::workspace) fn seed_flow_run_for_shot(
        &mut self,
        asking: bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ensure_project_for_shot();
        let lane = self.active;
        let Some(worktree) = self.lane_for(lane).map(|lane| lane.path.clone()) else {
            return;
        };
        let run_dir = self.active_lane_root().unwrap_or_default();
        // This capture is of the panel and the chip, which read what the run
        // is doing; no graph is open for it to colour.
        self.seed_flow_run_for_test(lane, run_dir.clone(), cx);
        RunsHost::apply_flow_event(
            &worktree,
            &run_dir,
            &daruda_flow::event::FlowEvent::NodeStarted {
                node: "verdict".into(),
                attempt: 2,
            },
            None,
            cx,
        );
        if asking {
            // Through the real parking path: the modal is raised there, and
            // a capture that skipped it would show only the panel.
            // The receiver goes at once: nothing in a capture answers, and
            // no event follows to find the question abandoned.
            let (reply, _) = smol::channel::bounded(1);
            RunsHost::park_flow_ask(
                &worktree,
                &run_dir,
                shot_question(1, "implement", "rm -rf target/debug/incremental", reply),
                cx,
            );
        }
    }

    #[cfg(any(test, feature = "screenshot"))]
    pub(in crate::workspace) fn seed_flow_run_for_test(
        &mut self,
        lane: daruda_store::project::LaneRef,
        run_dir: PathBuf,
        cx: &mut Context<Self>,
    ) {
        let source = crate::workspace::flow_request::FlowSource::Resumed {
            run_dir: run_dir.clone(),
        };
        self.seed_flow_run_of_for_test(lane, run_dir, source, cx);
    }

    /// Same, for a run whose origin matters — a graph is coloured only
    /// when the run can name the file it is of. Also the seed the
    /// `flow-graph-running` capture uses, which is the same need.
    #[cfg(any(test, feature = "screenshot"))]
    pub(in crate::workspace) fn seed_flow_run_of_for_test(
        &mut self,
        lane: daruda_store::project::LaneRef,
        run_dir: PathBuf,
        source: crate::workspace::flow_request::FlowSource,
        cx: &mut Context<Self>,
    ) {
        let Some(worktree) = self.lane_for(lane).map(|lane| lane.path.clone()) else {
            return;
        };
        RunsHost::seed_flow_run(worktree, seeded_view(run_dir, source), cx);
    }
}

/// A run with no engine behind it. The node ids come off the file the seed
/// names, the way a real submission takes them off what it loaded — a graph
/// is only coloured when the run and the file still agree about which nodes
/// exist.
#[cfg(any(test, feature = "screenshot"))]
fn seeded_view(run_dir: PathBuf, source: crate::workspace::flow_request::FlowSource) -> RunView {
    let nodes_at_start = match &source {
        crate::workspace::flow_request::FlowSource::File(path) => std::fs::read_to_string(path)
            .ok()
            .and_then(|text| daruda_flow::parse::parse_flow_file(&text).ok())
            .map(|file| file.nodes.iter().map(|n| n.id.clone()).collect())
            .unwrap_or_default(),
        crate::workspace::flow_request::FlowSource::Resumed { .. } => Vec::new(),
    };
    RunView::new(run_dir, source, nodes_at_start)
}

/// A permission question for a capture, as an agent asks one.
#[cfg(feature = "screenshot")]
pub(in crate::workspace) fn shot_question(
    ask_id: u64,
    node: &str,
    detail: &str,
    reply: smol::channel::Sender<daruda_acp::PermissionDecision>,
) -> daruda_flow::runner::PendingAsk {
    use daruda_acp::{PermissionChoice, PermissionKindView};
    daruda_flow::runner::PendingAsk {
        owner: daruda_flow::runner::AskOwner::Node {
            node: node.into(),
            generation: 0,
            attempt: 1,
        },
        ask_id,
        request: daruda_flow::runner::AskRequest {
            tool: "Bash".to_string(),
            detail: Some(detail.to_string()),
            options: vec![
                PermissionChoice {
                    option_id: "once".to_string(),
                    name: "Allow once".to_string(),
                    kind: PermissionKindView::AllowOnce,
                },
                PermissionChoice {
                    option_id: "always".to_string(),
                    name: "Allow always".to_string(),
                    kind: PermissionKindView::AllowAlways,
                },
                PermissionChoice {
                    option_id: "no".to_string(),
                    name: "Reject".to_string(),
                    kind: PermissionKindView::RejectOnce,
                },
            ],
        },
        reply,
    }
}
