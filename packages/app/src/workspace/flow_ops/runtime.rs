//! Submit, resume, and stop process-owned flow runs.

use std::path::{Path, PathBuf};

use daruda_flow::runner::{AcpRunner, OverageStop, ProcessRunner, Runners};
use gpui::Context;

use super::FlowStartRefused;
use crate::workspace::Workspace;
use crate::workspace::flow_request::{FlowSelection, FlowSubmission, union_strip_env};

impl Workspace {
    /// Start `path` in `lane` for `started_by`, returning the run's id.
    /// Says nothing on a refusal: the caller answers it in its own terms.
    pub(in crate::workspace) fn submit_flow_run(
        &mut self,
        lane: daruda_store::project::LaneRef,
        path: &Path,
        profile: Option<&str>,
        selection: &FlowSelection,
        submitter: (daruda_flow::request::StartedBy, bool),
        cx: &mut Context<Self>,
    ) -> Result<String, FlowStartRefused> {
        let mut submission = self
            .build_flow_request(lane, path, profile, selection, cx)
            .map_err(FlowStartRefused::Submit)?;
        submission.request.started_by = Some(submitter.0);
        Self::start_flow_thread(submission, submitter, cx)
    }

    /// The desktop's answer to a start it asked for. Another window can have
    /// started a run since the guard passed; the guard's answer then is to
    /// offer stopping the run that is there.
    pub(super) fn answer_flow_start_refusal(
        &mut self,
        lane: daruda_store::project::LaneRef,
        refused: FlowStartRefused,
        cx: &mut Context<Self>,
    ) {
        match refused {
            FlowStartRefused::Submit(e) => self.report_flow_refusal(e, cx),
            FlowStartRefused::AlreadyRunning => self.offer_to_stop_flow_in(lane, cx),
        }
    }

    /// Continue the run in `run_dir` through the funnel a fresh submission
    /// takes, so a resumed run is watched, stopped and listed like any other.
    /// The desktop resumes it, so the desktop is owed its answer; the run's
    /// log keeps who first started it.
    pub(in crate::workspace) fn resume_flow_run(
        &mut self,
        lane_ref: daruda_store::project::LaneRef,
        run_dir: &Path,
        cx: &mut Context<Self>,
    ) {
        let started = self
            .build_resume_request(lane_ref, run_dir, cx)
            .map_err(FlowStartRefused::Submit)
            .and_then(|submission| {
                Self::start_flow_thread(
                    submission,
                    (daruda_flow::request::StartedBy::Desktop, false),
                    cx,
                )
            });
        if let Err(refused) = started {
            self.answer_flow_start_refusal(lane_ref, refused, cx);
        }
    }

    /// Hand the run to the process. Its thread, stop switch and pump live
    /// there, so this window closing does not take the run with it.
    /// `submitted_by` is who this episode answers to.
    fn start_flow_thread(
        submission: FlowSubmission,
        submitter: (daruda_flow::request::StartedBy, bool),
        cx: &mut Context<Self>,
    ) -> Result<String, FlowStartRefused> {
        let (submitted_by, solicited) = submitter;
        let FlowSubmission {
            tree,
            request,
            source,
            node_install_dir,
            events,
            asks,
        } = submission;
        let overage = crate::overage_policy::OveragePolicy::stop_for_run(cx);
        let runners = {
            let overage = overage.clone();
            Box::new(move |request: &daruda_flow::request::RunRequest| {
                runners_for(request, node_install_dir, overage)
            })
        };
        crate::runs::RunsHost::start_flow(
            crate::runs::FlowStart {
                tree,
                request,
                source,
                runners,
                overage,
                submitted_by,
                solicited,
                events,
                asks,
            },
            cx,
        )
        .map_err(|crate::runs::AlreadyRunning| FlowStartRefused::AlreadyRunning)
    }

    pub(in crate::workspace) fn stop_flow_run_at(
        &mut self,
        lane: daruda_store::project::LaneRef,
        expected_run_dir: &Path,
        cx: &mut Context<Self>,
    ) {
        if self
            .flow_run_in(lane, cx)
            .is_some_and(|run| run.view().run_dir() == expected_run_dir)
        {
            self.stop_flow_run_in(lane, cx);
        }
    }

    /// Stop whichever run the named lane currently holds.
    pub(in crate::workspace) fn stop_flow_run_in(
        &mut self,
        lane: daruda_store::project::LaneRef,
        cx: &mut Context<Self>,
    ) {
        // The engine releases its own side — the interrupt arm answers the
        // adapter `Cancelled`.
        if let Some(worktree) = self.lane_for(lane).map(|lane| lane.path.clone()) {
            crate::runs::RunsHost::stop_flow(&worktree, cx);
        }
    }

    pub(in crate::workspace) fn stop_flow_ask(
        &mut self,
        lane: daruda_store::project::LaneRef,
        run_dir: &std::path::Path,
        ask_id: u64,
        cx: &mut Context<Self>,
    ) {
        if self.flow_run_in(lane, cx).is_some_and(|run| {
            run.view().run_dir() == run_dir
                && run
                    .view()
                    .question(ask_id)
                    .is_some_and(|ask| !ask.is_closed())
        }) {
            self.stop_flow_run_in(lane, cx);
        }
    }
}

/// The pair of runners one run drives with.
///
/// A free function rather than an inline literal inside the thread so the
/// one security-relevant decision in it — what `ProcessRunner` is told to
/// unset — can be asserted. A command node inherits the environment but
/// not the ACP account credentials, and the runner only
/// unsets what the host hands it: `Vec::new()` here is a silent leak into
/// every shell line a committed flow file names, with no other symptom.
fn runners_for(
    request: &daruda_flow::request::RunRequest,
    node_install_dir: PathBuf,
    overage: OverageStop,
) -> Runners {
    Runners {
        command: ProcessRunner::new(union_strip_env(&request.agents)),
        agent: AcpRunner::new(request.agents.clone(), node_install_dir)
            .with_accounts(request.history.accounts.clone())
            .with_overage_stop(overage),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use daruda_acp::LaunchSpec;
    use daruda_flow::request::{Budget, RunRequest};

    use super::*;

    const MIXED_FLOW: &str = "\
version: 1
defaults:
  agent:
    id: claude
    mode: bypassPermissions
nodes:
  - id: verdict
    kind: agent
    output: verdict.md
    prompt: write a line
  - id: gate
    kind: command
    deps: [verdict]
    run: \"true\"
";

    fn request_with(strip: &[&str]) -> RunRequest {
        let loaded = daruda_flow::load(MIXED_FLOW, None).expect("the fixture flow loads");
        RunRequest {
            history: Default::default(),
            started_by: Some(daruda_flow::request::StartedBy::Desktop),
            until: None,
            pinned: Vec::new(),
            loaded,
            cwd: PathBuf::from("/lane"),
            run_dir: PathBuf::from("/lane/runs/1"),
            lock_dir: PathBuf::from("/data/flow-locks"),
            flow_dir: PathBuf::from("/lane/flows"),
            agents: HashMap::from([(
                "claude".to_string(),
                LaunchSpec {
                    command: "adapter".to_string(),
                    strip_env: strip.iter().map(|s| (*s).to_string()).collect(),
                },
            )]),
            node_install_dir: PathBuf::from("/data/node"),
            budget: Budget::unlimited(),
            is_alive: Box::new(|_| false),
            git_status: None,
            events: None,
            ask: None,
            resume: None,
        }
    }

    /// The account-credential stripping rule is applied here. Reverting
    /// `runners_for` to `ProcessRunner::new(Vec::new())` — which is what
    /// `examples/run_flow.rs` does, and the obvious thing to copy — leaks
    /// the account's credentials into every shell line a committed flow
    /// file names, and this is the only thing that would notice.
    #[test]
    fn the_command_runner_is_handed_the_credentials_to_strip() {
        let request = request_with(&["ANTHROPIC_API_KEY"]);
        let runners = runners_for(
            &request,
            PathBuf::from("/data/node"),
            OverageStop::default(),
        );
        assert_eq!(runners.command.strip_env(), ["ANTHROPIC_API_KEY"]);
    }
}
