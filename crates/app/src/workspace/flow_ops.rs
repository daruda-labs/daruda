//! Running a flow from the app, start to finish: the picker funnel (the
//! guard, the profile question, the dispatch), the static check that can
//! refuse before anything is spawned, submission and resume, the stop
//! switch, and the two ways to a run that is no longer in front of you —
//! the panel it belongs to and the report it wrote. Every refusal along
//! the way is worded in one place here, so a flow that cannot run says why
//! in the same voice however it was started.
//!
//! The engine's `execute` blocks, so a run owns a thread and the workspace
//! owns its [`CancelToken`]. Everything the run has to say comes back on
//! one `FlowEvent` stream — there is no second completion channel, because
//! two would be two things to keep in step. Draining that stream, and the
//! questions beside it, is [`super::flow_events`]'s; a live run's display
//! shape is [`super::flow_rows`]'s, and a finished one's is
//! [`super::flow_history`]'s.
//!
//! The `--screenshot` and test seeds reach run state directly rather than
//! by starting a flow: a capture or a test that submitted a real one would
//! leave a run directory behind in whichever repository was open.

use std::path::{Path, PathBuf};

use daruda_flow::event::{FlowEvent, RunEnd};
use daruda_flow::runner::{AcpRunner, CancelToken, ProcessRunner, Runners};
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use gpui::{Context, Window};

use super::Workspace;
use super::command::flow_picker::{FlowPick, FlowPicker, FlowPurpose};
use super::flow_request::{FlowSelection, FlowSubmission, FlowSubmitError, union_strip_env};
use super::flow_runs::RunHandle;
// Only the seeded runs name a stage outright; a real one gets there through
// the event pump.
#[cfg(any(test, feature = "screenshot"))]
use super::flow_runs::RunStage;
use crate::surface::strings as s;

impl Workspace {
    // ---- Picker ----

    pub(in crate::workspace) fn on_run_flow(
        &mut self,
        _: &super::RunFlow,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_flow_picker(FlowPurpose::Run, cx);
    }

    pub(in crate::workspace) fn on_validate_flow(
        &mut self,
        _: &super::ValidateFlow,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_flow_picker(FlowPurpose::Validate, cx);
    }

    pub(in crate::workspace) fn open_flow_picker(
        &mut self,
        purpose: FlowPurpose,
        cx: &mut Context<Self>,
    ) {
        if self.flow_picker.is_open() {
            self.flow_picker.close();
            cx.notify();
            return;
        }
        if !self.flow_run_guard(purpose, cx) {
            return;
        }
        let listed = self
            .flow_sources()
            .map(|sources| sources.list_flows())
            .unwrap_or_default();
        self.flow_picker.open(purpose, listed);
        cx.notify();
    }

    /// Whether `purpose` may go ahead in the active lane, having already said
    /// why not when it may not.
    ///
    /// Design §14's affordance: what says a run is going is the lock, not a
    /// field this app keeps — so a run started by a previous session, or by
    /// another window, is recognised the same way. Every way in asks this
    /// first, whether or not a list of flows is part of it.
    fn flow_run_guard(&mut self, purpose: FlowPurpose, cx: &mut Context<Self>) -> bool {
        let Some(cwd) = self.active_lane_root() else {
            self.report_flow_refusal(FlowSubmitError::NoLane, cx);
            return false;
        };
        // A purpose no running flow stands in the way of does not read the
        // lock at all: `lane_holder` goes to disk and then probes the pid it
        // names, for an answer that could not change the outcome.
        if !purpose.blocked_by_a_running_flow() {
            return true;
        }
        match self.lane_holder(&cwd) {
            // The stop switch is a `CancelToken`, so the only run we can
            // stop is one whose token we are holding — for *this* lane.
            // The map is the authority, not the lock's pid: a token for
            // another lane belongs to this process too and would not stop
            // this one.
            Some(_) if self.runs.is_running(self.active) => {
                self.flow_picker = FlowPicker::Stopping;
                cx.notify();
                false
            }
            // Offering "stop it" for a run we cannot reach would be a
            // button that does nothing.
            Some(holder) => {
                self.report_flow_refusal(FlowSubmitError::LockHeld { pid: holder.pid }, cx);
                false
            }
            None => true,
        }
    }

    /// The live process holding this lane's run lock, if any. A lock left by
    /// a crashed run names a pid that is gone, and does not stop a new run
    /// — the engine reclaims it.
    fn lane_holder(&self, cwd: &Path) -> Option<daruda_flow::lock::LockHolder> {
        daruda_flow::lock::read_holder(&super::flow_paths::runs_dir(cwd))
            .filter(|holder| super::flow_request::process_is_alive(holder.pid))
    }

    pub(in crate::workspace) fn close_flow_picker(&mut self, cx: &mut Context<Self>) {
        self.flow_picker.close();
        cx.notify();
    }

    /// Act on the focused row and close.
    pub(in crate::workspace) fn execute_flow_picker_selection(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let picked = self.flow_picker.focused_pick();
        let was_stopping = matches!(self.flow_picker, FlowPicker::Stopping);

        match picked {
            // Which flow, answered. Whatever is left of it belongs to
            // `start_flow`, which is also where the graph pane's ▶ comes in —
            // that button knows the flow already and skips only this question.
            Some(FlowPick::Flow(purpose, path)) => {
                self.start_flow(purpose, path, FlowSelection::default(), window, cx)
            }
            // The second question, so nothing is left to ask.
            Some(FlowPick::Profile(purpose, path, selection, profile)) => {
                self.flow_picker.close();
                cx.notify();
                self.dispatch_flow(purpose, &path, profile.as_deref(), &selection, window, cx);
            }
            None => {
                self.flow_picker.close();
                cx.notify();
                if was_stopping {
                    self.stop_flow_run(cx);
                }
            }
        }
    }

    /// Run or check `path` without asking which flow.
    ///
    /// The guard still runs: a surface that names the flow does not thereby
    /// know whether the lane is free, and skipping it here would be a second
    /// answer to "is a run already going" for the lock to disagree with.
    pub(in crate::workspace) fn run_flow_at(
        &mut self,
        path: &Path,
        purpose: FlowPurpose,
        selection: FlowSelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.flow_run_guard(purpose, cx) {
            return;
        }
        self.start_flow(purpose, path.to_path_buf(), selection, window, cx);
    }

    /// Everything a named flow still has to go through: the profile question
    /// when the file declares any, then the act itself.
    ///
    /// The one place that decides whether a profile is asked for. The guard is
    /// deliberately not here — the picker ran it before it listed anything, and
    /// asking twice would drop a `Stopping` picker over a list already shown.
    fn start_flow(
        &mut self,
        purpose: FlowPurpose,
        path: PathBuf,
        selection: FlowSelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A flow that declares profiles has one more question to answer, so the
        // picker comes up for it — even when nothing opened the picker to begin
        // with. Everything else is decided.
        if purpose.asks_about_profiles() {
            let profiles = flow_profiles(&path);
            if !profiles.is_empty() {
                // The selection rides into the picker rather than being asked
                // for again on the way out: the profile stage already carries
                // what has been decided so far, and which nodes to spend on is
                // one of those things.
                self.flow_picker
                    .ask_profile(purpose, path, selection, profiles);
                cx.notify();
                return;
            }
        }

        self.flow_picker.close();
        cx.notify();
        self.dispatch_flow(purpose, &path, None, &selection, window, cx);
    }

    /// Act on a flow that has every answer it needs.
    fn dispatch_flow(
        &mut self,
        purpose: FlowPurpose,
        path: &Path,
        profile: Option<&str>,
        selection: &FlowSelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match purpose {
            FlowPurpose::Validate => self.validate_flow(path, profile, window, cx),
            FlowPurpose::Run => self.submit_flow_run(path, profile, selection, cx),
            FlowPurpose::Graph => self.open_flow_graph(path, window, cx),
        }
    }

    // ---- Authoring: the file, not its contents (S4 owns the contents) ----

    // ---- Stage 1: static checks, which cost nothing ----

    /// Report what can be known without running the flow. Opens no session,
    /// takes no lock, and creates no run directory.
    fn validate_flow(
        &mut self,
        path: &Path,
        profile: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = super::flow_paths::flow_label(path);
        let (title, body) = match self.check_flow(path, profile, cx) {
            Ok(issues) if issues.is_empty() => (s::flow_valid_title(), s::flow_valid_body(&name)),
            Ok(issues) => (
                s::flow_invalid_title(&name, issues.len()),
                issue_report(&issues),
            ),
            Err(FlowSubmitError::Load(daruda_flow::FlowError::Validate(issues))) => (
                s::flow_invalid_title(&name, issues.len()),
                issue_report(&issues),
            ),
            Err(FlowSubmitError::Load(daruda_flow::FlowError::Parse(detail))) => {
                (s::flow_parse_failed_title(), detail)
            }
            // Not about the flow: no lane, a remote lane, an unreadable
            // file. The same refusal `Run Flow…` would give.
            Err(other) => {
                self.report_flow_refusal(other, cx);
                return;
            }
        };
        super::dialog_helpers::open_alert_dialog(title, body, s::flow_close(), window, cx);
    }

    // ---- Stage 2: the run ----

    fn submit_flow_run(
        &mut self,
        path: &Path,
        profile: Option<&str>,
        selection: &FlowSelection,
        cx: &mut Context<Self>,
    ) {
        match self.build_flow_request(path, profile, selection, cx) {
            Ok(submission) => self.start_flow_thread(submission, cx),
            Err(e) => self.report_flow_refusal(e, cx),
        }
    }

    /// Continue the run in `run_dir` instead of starting a new one.
    ///
    /// The same funnel a fresh submission goes through, on purpose: a
    /// resumed run is watched, stopped and listed exactly like any other,
    /// and a second launch path would be a second set of those to keep
    /// working.
    pub(in crate::workspace) fn resume_flow_run(
        &mut self,
        lane_ref: daruda_store::project::LaneRef,
        run_dir: &Path,
        cx: &mut Context<Self>,
    ) {
        match self.build_resume_request(lane_ref, run_dir, cx) {
            Ok(submission) => self.start_flow_thread(submission, cx),
            Err(e) => self.report_flow_refusal(e, cx),
        }
    }

    fn start_flow_thread(&mut self, submission: FlowSubmission, cx: &mut Context<Self>) {
        let FlowSubmission {
            lane,
            request,
            source,
            node_install_dir,
            events,
            asks,
        } = submission;
        let cancel = CancelToken::default();
        let run_dir = request.run_dir.clone();
        let run_dir_for_asks = run_dir.clone();
        // Before the thread takes the request.
        let nodes_at_start: Vec<daruda_flow::NodeId> = request
            .loaded
            .flow()
            .nodes
            .iter()
            .map(|node| node.id.clone())
            .collect();
        // Captured by the request builder: the run belongs to the lane it
        // was submitted from, not to whichever one is active when it ends.
        let lane_ref = lane;

        let thread = {
            let cancel = cancel.clone();
            std::thread::spawn(move || {
                let runners = runners_for(&request, node_install_dir);
                let report = daruda_flow::schedule::execute(&request, &runners, &cancel);
                // A run the engine refuses before it starts — a lock another
                // process holds, a runs directory it cannot create — takes
                // nothing, so it rightly emits neither `RunStarted` nor
                // `RunEnded`. It reports by *returning*. Without this the
                // user picks a flow and sees nothing happen at all.
                //
                // Safe to send unconditionally: the watcher stops at the
                // first `RunEnded`, so when the engine sent its own this one
                // is never read.
                if let Some(events) = request.events.as_ref() {
                    let end = RunEnd::from(&report.outcome);
                    let _ = events.try_send(FlowEvent::RunEnded { end });
                }
            })
        };
        self.runs.insert(
            lane_ref,
            RunHandle::started(cancel, run_dir, source, nodes_at_start, thread),
        );
        self.watch_flow_events(lane_ref, events, cx);
        self.watch_flow_asks(lane_ref, run_dir_for_asks, asks, cx);
        cx.notify();
    }

    /// Stop the run in the active lane. Runs in other lanes are untouched
    /// — each holds its own token.
    pub(in crate::workspace) fn stop_flow_run(&mut self, cx: &mut Context<Self>) {
        let lane = self.active;
        self.stop_flow_run_in(lane, cx);
    }

    /// Stop the run a named lane holds. The status bar lists every run this
    /// window started, including ones in lanes that are not active, so it
    /// needs to say which — the active lane is not the answer there.
    pub(in crate::workspace) fn stop_flow_run_in(
        &mut self,
        lane: daruda_store::project::LaneRef,
        cx: &mut Context<Self>,
    ) {
        // The engine releases its own side — the interrupt arm answers the
        // adapter `Cancelled`.
        self.runs.cancel(lane);
        cx.notify();
    }

    /// Bring the run in `lane` into view: switch to its lane if needed,
    /// open the right dock, and show the Flows tab.
    ///
    /// The panel is lane-scoped, so a question in another lane is answered
    /// *there* — this is the one move that gets you to it. Deliberately a
    /// named op rather than three calls at the click site: the three have to
    /// happen together, and a surface that did two of them would land on the
    /// right lane with the panel still hidden.
    pub(in crate::workspace) fn reveal_flow_run(
        &mut self,
        lane: daruda_store::project::LaneRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active != lane {
            self.activate_lane(lane, window, cx);
        }
        self.reveal_flows_panel(cx);
    }

    /// Bring the Flows panel into view: the dock open, the main area told to
    /// re-measure, and the panel selected.
    ///
    /// A named op rather than the steps at each surface, because a copy that
    /// does two of the three shows nothing: the tab alone lands behind a
    /// collapsed dock, and a dock opened without `pending_resize` leaves the
    /// main area measured for the width it had before the panel appeared, so
    /// a terminal beside it keeps its old column count. The repaint belongs
    /// here for the same reason — `set_right_dock_view` returns early when
    /// Flows is already the tab, and then only the dock width moved.
    ///
    /// Lane switching is the caller's: a run worth revealing may live in
    /// another lane, while the capture paths want the lane they are on.
    pub(in crate::workspace) fn reveal_flows_panel(&mut self, cx: &mut Context<Self>) {
        self.reveal_right_dock_view(daruda_store::project::RightDockView::Flows, cx);
    }

    /// Open a past run's narrative in the active lane. The same view a run
    /// opens on its own when it finishes, so a run read later looks like
    /// the one read as it ended.
    pub(in crate::workspace) fn open_flow_report(
        &mut self,
        report: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_pane_file_view(
            self.active.lane,
            report.to_path_buf(),
            /* staged = */ false,
            super::main_area::file_view_pane::FileViewMode::Preview,
            window,
            cx,
        );
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
        let dir = super::flow_paths::runs_dir(&self.active_lane_root().unwrap_or_default());
        let shot = super::flow_history::FlowHistory::for_shot(dir);
        self.flow_history.put(self.active, shot);
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
        self.flow_picker
            .ask_profile(FlowPurpose::Run, path, FlowSelection::default(), profiles);
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
        self.runs.insert(
            lane,
            // This capture is of the panel and the chip, which read `doing`;
            // no graph pane is open for it to colour.
            RunHandle::seeded(
                run_dir.clone(),
                super::flow_request::FlowSource::Resumed {
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
                    node: "implement".into(),
                    attempt: 1,
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
        let source = super::flow_request::FlowSource::Resumed {
            run_dir: run_dir.clone(),
        };
        self.seed_flow_run_of_for_test(lane, run_dir, source);
    }

    /// Same, for a run whose origin matters — a graph pane is coloured only
    /// when the run can name the file it is of. Also the seed the
    /// `flow-graph-running` capture uses, which is the same need.
    #[cfg(any(test, feature = "screenshot"))]
    pub(in crate::workspace) fn seed_flow_run_of_for_test(
        &mut self,
        lane: daruda_store::project::LaneRef,
        run_dir: PathBuf,
        source: super::flow_request::FlowSource,
    ) {
        self.runs
            .insert(lane, RunHandle::seeded(run_dir, source, RunStage::Starting));
    }

    fn report_flow_refusal(&mut self, error: FlowSubmitError, cx: &mut Context<Self>) {
        let (message, detail) = match error {
            FlowSubmitError::NoLane => (s::flow_no_lane(), String::new()),
            FlowSubmitError::RemoteLane { agent } => (s::flow_remote_lane(&agent), String::new()),
            FlowSubmitError::LockHeld { pid } => (s::flow_lock_held(pid), String::new()),
            FlowSubmitError::Read { path, message } => (
                s::flow_read_failed_title(),
                format!("{}: {message}", path.display()),
            ),
            FlowSubmitError::Load(daruda_flow::FlowError::Parse(detail)) => {
                (s::flow_parse_failed_title(), detail)
            }
            FlowSubmitError::Load(daruda_flow::FlowError::Validate(issues))
            | FlowSubmitError::Invalid(issues) => (
                s::flow_invalid_title("", issues.len()),
                issue_report(&issues),
            ),
            // The engine's own words: it is the one that decides what can be
            // continued, and a second wording here would be a second answer
            // to the same question.
            FlowSubmitError::Resume(e) => (s::flow_resume_refused(), e.to_string()),
        };
        self.report_error(
            ErrorReport::new(message)
                .severity(ErrorSeverity::Warning)
                .at(file!(), line!())
                .with_context("detail", detail)
                .dedup("flow.submit.refused")
                .build(),
            cx,
        );
    }
}

/// The pair of runners one run drives with.
///
/// A free function rather than an inline literal inside the thread so the
/// one security-relevant decision in it — what `ProcessRunner` is told to
/// unset — can be asserted. Design §9 lets a command node inherit the
/// environment but not the ACP account credentials, and the runner only
/// unsets what the host hands it: `Vec::new()` here is a silent leak into
/// every shell line a committed flow file names, with no other symptom.
fn runners_for(request: &daruda_flow::request::RunRequest, node_install_dir: PathBuf) -> Runners {
    Runners {
        command: ProcessRunner::new(union_strip_env(&request.agents)),
        agent: AcpRunner::new(request.agents.clone(), node_install_dir),
    }
}

/// The profiles a flow declares, for the second question. A file that
/// cannot be read or parsed reports none: the run that follows fails on
/// the same read a moment later and names it properly, and a second
/// error surface here would say it twice in different words.
fn flow_profiles(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| daruda_flow::load::profiles(&text).ok())
        .unwrap_or_default()
}

/// Every problem, in the wording the user gets. `ValidationIssue.message`
/// is developer detail and deliberately never appears here.
pub(super) fn issue_report(issues: &[daruda_flow::error::ValidationIssue]) -> String {
    issues
        .iter()
        .map(|issue| {
            s::flow_issue_line(
                issue.node.as_ref().map(|n| n.as_str()),
                &s::flow_issue(&issue.kind),
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use daruda_acp::LaunchSpec;
    use daruda_flow::error::{ValidationIssue, ValidationKind};
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
            until: None,
            pinned: Vec::new(),
            loaded,
            cwd: PathBuf::from("/lane"),
            run_dir: PathBuf::from("/lane/runs/1"),
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

    /// Design §9's rule, at the one place it is actually applied. Reverting
    /// `runners_for` to `ProcessRunner::new(Vec::new())` — which is what
    /// `examples/run_flow.rs` does, and the obvious thing to copy — leaks
    /// the account's credentials into every shell line a committed flow
    /// file names, and this is the only thing that would notice.
    #[test]
    fn the_command_runner_is_handed_the_credentials_to_strip() {
        let request = request_with(&["ANTHROPIC_API_KEY"]);
        let runners = runners_for(&request, PathBuf::from("/data/node"));
        assert_eq!(runners.command.strip_env(), ["ANTHROPIC_API_KEY"]);
    }

    /// The whole point of the `kind` → wording split: the developer text
    /// carries file paths and internal names, and must not reach a user.
    #[test]
    fn a_report_never_shows_the_developer_message() {
        let issues = vec![ValidationIssue {
            node: Some("gate".into()),
            kind: ValidationKind::Cycle,
            message: "internal detail nobody should read".to_string(),
        }];
        let report = issue_report(&issues);
        assert!(!report.contains("internal detail"), "{report}");
        assert!(report.contains("gate"), "{report}");
    }

    /// Every problem at once — `load` collects within a stage precisely so
    /// an author fixes them in one pass rather than one per run.
    #[test]
    fn the_report_names_every_problem_at_once() {
        let issues = vec![
            ValidationIssue {
                node: Some("a".into()),
                kind: ValidationKind::MissingAgent,
                message: String::new(),
            },
            ValidationIssue {
                node: Some("b".into()),
                kind: ValidationKind::DuplicateOutput,
                message: String::new(),
            },
        ];
        assert_eq!(issue_report(&issues).lines().count(), 2);
    }
}
