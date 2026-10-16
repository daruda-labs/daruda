//! Run state seeded straight into the workspace for `--screenshot` captures
//! and tests, without starting a real flow.

#[cfg(any(test, feature = "screenshot"))]
use std::path::PathBuf;

#[cfg(feature = "screenshot")]
use gpui::{Context, Window};

use crate::workspace::Workspace;
#[cfg(feature = "screenshot")]
use crate::workspace::command::flow_picker::FlowPurpose;
#[cfg(feature = "screenshot")]
use crate::workspace::flow_request::FlowSelection;
#[cfg(any(test, feature = "screenshot"))]
use crate::workspace::flow_runs::RunHandle;

#[cfg(feature = "screenshot")]
use super::flow_profiles;

// Only the seeded runs name a stage outright; a real one gets there through
// the event pump.
#[cfg(any(test, feature = "screenshot"))]
use crate::workspace::flow_runs::RunStage;

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
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use daruda_acp::{PermissionChoice, PermissionKindView};
        let lane = self.active;
        let run_dir = self.active_lane_root().unwrap_or_default();
        self.flows.runs.insert(
            lane,
            // This capture is of the panel and the chip, which read `doing`;
            // no graph is open for it to colour.
            RunHandle::seeded(
                run_dir.clone(),
                crate::workspace::flow_request::FlowSource::Resumed {
                    run_dir: run_dir.clone(),
                },
                RunStage::Node {
                    id: "verdict".into(),
                    attempt: 2,
                },
            ),
        );
        if asking {
            // Through the real parking path, not by writing the stage: the
            // modal is raised there, and a capture that skipped it would
            // show only the panel — the half already covered.
            let (reply, _rx) = smol::channel::bounded(1);
            self.park_flow_ask(
                lane,
                &run_dir,
                daruda_flow::runner::PendingAsk {
                    owner: daruda_flow::runner::AskOwner::Node {
                        node: "implement".into(),
                        generation: 0,
                        attempt: 1,
                    },
                    ask_id: 1,
                    request: daruda_flow::runner::AskRequest {
                        tool: "Bash".to_string(),
                        detail: Some("rm -rf target/debug/incremental".to_string()),
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
                },
                window,
                cx,
            );
        }
        cx.notify();
    }

    #[cfg(test)]
    pub(in crate::workspace) fn seed_flow_run_for_test(
        &mut self,
        lane: daruda_store::project::LaneRef,
        run_dir: PathBuf,
    ) {
        let source = crate::workspace::flow_request::FlowSource::Resumed {
            run_dir: run_dir.clone(),
        };
        self.seed_flow_run_of_for_test(lane, run_dir, source);
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
    ) {
        self.flows
            .runs
            .insert(lane, RunHandle::seeded(run_dir, source, RunStage::Starting));
    }
}
