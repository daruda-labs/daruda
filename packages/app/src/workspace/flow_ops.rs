//! Running a flow from the app, start to finish: the picker funnel (the
//! guard, the profile question, the dispatch), the static check that can
//! refuse before anything is spawned, submission and resume, the stop
//! switch, and the two ways to a run that is no longer in front of you —
//! the panel it belongs to and the report it wrote. Every refusal along
//! the way is worded in one place here, so a flow that cannot run says why
//! in the same voice however it was started.
//!
//! The engine's `execute` blocks, so a run owns a thread — and the process,
//! not this window, owns the run ([`crate::runs::RunsHost`]): its thread, its
//! stop switch and the one pump that drains its stream and questions. What a
//! window does when the run changes is [`super::flow_events`]'s; a live run's
//! display shape is [`super::flow_rows`]'s, and a finished one's is
//! [`super::flow_history`]'s.
//!
//! The `--screenshot` and test seeds reach run state directly rather than
//! by starting a flow: a capture or a test that submitted a real one would
//! leave a run directory behind in whichever repository was open.

use crate::agent::launch_resolve::ConnectCommandError;
use std::path::{Path, PathBuf};

use daruda_flow::runner::{AcpRunner, OverageStop, ProcessRunner, Runners};
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use gpui::{Context, Window};

use super::Workspace;
use super::command::flow_picker::{FlowPick, FlowPicker, FlowPurpose};
use super::command::picker::PickerKey;
use super::command::picker_key::picker_keystroke;
use super::flow_request::{FlowSelection, FlowSubmission, FlowSubmitError, union_strip_env};
use crate::surface::strings as s;

pub(in crate::workspace) mod seeds;

/// Everything a dispatch has been told, once every question the picker asks
/// has an answer. One value because the five always travel together, and
/// three functions passing them one by one is how one of them goes missing.
struct FlowDispatch<'a> {
    lane: daruda_store::project::LaneRef,
    purpose: FlowPurpose,
    path: &'a Path,
    profile: Option<&'a str>,
    selection: &'a FlowSelection,
}

/// Why a submitted run did not start. Typed, so each caller answers in its
/// own terms: the desktop with a toast or its stop offer, a control caller
/// with an error it can read.
#[derive(Debug)]
pub(in crate::workspace) enum FlowStartRefused {
    /// The request could not be built.
    Submit(FlowSubmitError),
    /// This process already holds a run in the worktree.
    AlreadyRunning,
}

impl Workspace {
    // ---- Picker ----

    pub(in crate::workspace) fn on_run_flow(
        &mut self,
        _: &super::RunFlow,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_flow_picker(self.active, FlowPurpose::Run, cx);
    }

    pub(in crate::workspace) fn on_validate_flow(
        &mut self,
        _: &super::ValidateFlow,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_flow_picker(self.active, FlowPurpose::Validate, cx);
    }

    pub(in crate::workspace) fn open_flow_picker(
        &mut self,
        lane: daruda_store::project::LaneRef,
        purpose: FlowPurpose,
        cx: &mut Context<Self>,
    ) {
        if self.overlays.flow_picker.is_open() {
            self.overlays.flow_picker.close();
            cx.notify();
            return;
        }
        if !self.flow_run_guard(lane, purpose, cx) {
            return;
        }
        let listed = self
            .flow_sources_for(lane)
            .map(|sources| sources.list_flows())
            .unwrap_or_default();
        self.overlays.flow_picker.open(lane, purpose, listed);
        cx.notify();
    }

    /// Whether `purpose` may go ahead in `lane`, having already said why not
    /// when it may not.
    ///
    /// What says a run is going is the lock, not a field this app keeps — so
    /// a run started by a previous session, or by
    /// another window, is recognised the same way. Every way in asks this
    /// first, whether or not a list of flows is part of it.
    fn flow_run_guard(
        &mut self,
        lane: daruda_store::project::LaneRef,
        purpose: FlowPurpose,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(cwd) = self.lane_for(lane).map(|l| l.path.clone()) else {
            self.report_flow_refusal(FlowSubmitError::NoLane, cx);
            return false;
        };
        // A purpose no running flow stands in the way of does not read the
        // lock at all: `lane_holder` goes to disk and then probes the pid it
        // names, for an answer that could not change the outcome.
        if !purpose.blocked_by_a_running_flow() {
            return true;
        }
        // The host first: a run this process holds — for *this* worktree —
        // is one we can stop, whether or not its thread has taken the lock
        // yet. Between a start and its lock only the host knows of it, and a
        // second start then would replace it with no stop switch left.
        if self.is_flow_running_in(lane, cx) {
            self.offer_to_stop_flow_in(lane, cx);
            return false;
        }
        match self.lane_holder(&cwd) {
            // Offering "stop it" for a run we cannot reach would be a
            // button that does nothing.
            Some(holder) => {
                let lock_dir = super::flow_paths::lane_lock_dir(&self.lock_root, &cwd);
                self.report_flow_refusal(
                    FlowSubmitError::LockHeld {
                        pid: holder.pid,
                        lock_dir,
                    },
                    cx,
                );
                false
            }
            None => true,
        }
    }

    /// Answer a start in `lane`, which already has a run this process holds,
    /// by offering to stop that run.
    fn offer_to_stop_flow_in(
        &mut self,
        lane: daruda_store::project::LaneRef,
        cx: &mut Context<Self>,
    ) {
        let Some(run_dir) = self
            .flow_run_in(lane, cx)
            .map(|run| run.view().run_dir().to_path_buf())
        else {
            return;
        };
        self.overlays.flow_picker = FlowPicker::Stopping { lane, run_dir };
        cx.notify();
    }

    /// The live process holding this lane's run lock, if any. A lock left by
    /// a crashed run names a pid that is gone, and does not stop a new run
    /// — the engine reclaims it.
    ///
    /// The lock lives outside the tree (`flow_paths::lane_lock_dir`), so it
    /// still answers after an agent has cleaned the worktree out from under
    /// a run this build started.
    pub(in crate::workspace) fn lane_holder(
        &self,
        cwd: &Path,
    ) -> Option<daruda_flow::lock::LockHolder> {
        super::flow_paths::lane_lock_dir(&self.lock_root, cwd)
            .and_then(|dir| daruda_flow::lock::read_holder(&dir))
            .filter(|holder| super::flow_request::process_is_alive(holder.pid))
    }

    pub(in crate::workspace) fn close_flow_picker(&mut self, cx: &mut Context<Self>) {
        self.overlays.flow_picker.close();
        cx.notify();
    }

    /// One keystroke while the picker is up.
    pub(in crate::workspace) fn on_flow_picker_key(
        &mut self,
        ev: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.overlays.flow_picker.is_open() {
            return;
        }
        let Some((key, ch)) = picker_keystroke(ev) else {
            return;
        };
        match self.overlays.flow_picker.on_key(key, ch) {
            PickerKey::Confirm => self.execute_flow_picker_selection(window, cx),
            PickerKey::Dismiss => self.close_flow_picker(cx),
            PickerKey::Changed => cx.notify(),
            PickerKey::Unchanged => {}
        }
        cx.stop_propagation();
    }

    /// Act on the row the mouse named. Focusing it first is what makes a
    /// click the same gesture as arrowing there and pressing Enter.
    pub(in crate::workspace) fn pick_flow_row(
        &mut self,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.overlays.flow_picker.focus(ix);
        self.execute_flow_picker_selection(window, cx);
    }

    /// Act on the focused row and close.
    pub(in crate::workspace) fn execute_flow_picker_selection(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let picked = self.overlays.flow_picker.focused_pick();
        // The worktree the prompt was raised for, not whichever is on screen
        // now: the picker can have been opened for a parked one, and a stop
        // must end the run it offered to stop.
        let stopping = match &self.overlays.flow_picker {
            FlowPicker::Stopping { lane, run_dir } => Some((*lane, run_dir.clone())),
            _ => None,
        };

        match picked {
            // Which flow, answered. Whatever is left of it belongs to
            // `start_flow`, which is also where the graph's ▶ comes in —
            // that button knows the flow already and skips only this question.
            Some(FlowPick::Flow {
                lane,
                purpose,
                path,
            }) => self.start_flow(lane, purpose, path, FlowSelection::default(), window, cx),
            // The second question, so nothing is left to ask.
            Some(FlowPick::Profile {
                lane,
                purpose,
                path,
                selection,
                profile,
            }) => {
                self.overlays.flow_picker.close();
                cx.notify();
                self.dispatch_flow(
                    FlowDispatch {
                        lane,
                        purpose,
                        path: &path,
                        profile: profile.as_deref(),
                        selection: &selection,
                    },
                    window,
                    cx,
                );
            }
            None => {
                self.overlays.flow_picker.close();
                cx.notify();
                if let Some((lane, run_dir)) = stopping {
                    self.stop_flow_run_at(lane, &run_dir, cx);
                }
            }
        }
    }

    /// Run or check `path` without asking which flow. `false` means the guard
    /// refused — it has already said why, on screen.
    ///
    /// The guard still runs: a surface that names the flow does not thereby
    /// know whether the lane is free, and skipping it here would be a second
    /// answer to "is a run already going" for the lock to disagree with. The
    /// control surface does not come through here: it needs the typed
    /// refusal, which `submit_flow_run` returns without showing anything.
    #[must_use = "a refused run has already been shown on screen; bind it to \
                  say the caller relies on that"]
    pub(in crate::workspace) fn run_flow_at(
        &mut self,
        lane: daruda_store::project::LaneRef,
        path: &Path,
        purpose: FlowPurpose,
        selection: FlowSelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.flow_run_guard(lane, purpose, cx) {
            return false;
        }
        self.start_flow(lane, purpose, path.to_path_buf(), selection, window, cx);
        true
    }

    /// Everything a named flow still has to go through: the profile question
    /// when the file declares any, then the act itself.
    ///
    /// The one place that decides whether a profile is asked for. The guard is
    /// deliberately not here — the picker ran it before it listed anything, and
    /// asking twice would drop a `Stopping` picker over a list already shown.
    fn start_flow(
        &mut self,
        lane: daruda_store::project::LaneRef,
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
                self.overlays
                    .flow_picker
                    .ask_profile(lane, purpose, path, selection, profiles);
                cx.notify();
                return;
            }
        }

        self.overlays.flow_picker.close();
        cx.notify();
        self.dispatch_flow(
            FlowDispatch {
                lane,
                purpose,
                path: &path,
                profile: None,
                selection: &selection,
            },
            window,
            cx,
        );
    }

    /// Act on a flow that has every answer it needs.
    fn dispatch_flow(
        &mut self,
        flow: FlowDispatch<'_>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let FlowDispatch {
            lane,
            purpose,
            path,
            profile,
            selection,
        } = flow;
        match purpose {
            FlowPurpose::Validate => self.validate_flow(lane, path, profile, window, cx),
            FlowPurpose::Run => {
                let started = self.submit_flow_run(
                    lane,
                    path,
                    profile,
                    selection,
                    (daruda_flow::request::StartedBy::Desktop, false),
                    cx,
                );
                if let Err(refused) = started {
                    self.answer_flow_start_refusal(lane, refused, cx);
                }
            }
            // The picker's worktree is the one the graph then runs in.
            FlowPurpose::Graph => self.open_flow_graph(lane, path, window, cx),
        }
    }

    // ---- Authoring: the file, not its contents (S4 owns the contents) ----

    // ---- Stage 1: static checks, which cost nothing ----

    /// Report what can be known without running the flow. Opens no session,
    /// takes no lock, and creates no run directory.
    fn validate_flow(
        &mut self,
        lane: daruda_store::project::LaneRef,
        path: &Path,
        profile: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = super::flow_paths::read_flow_name(path);
        let (title, body) = match self.check_flow(lane, path, profile, cx) {
            Ok(issues) if issues.is_empty() => (s::flow::valid_title(), s::flow::valid_body(&name)),
            Ok(issues) => (
                s::flow::invalid_title(&name, issues.len()),
                issue_report(&issues),
            ),
            Err(FlowSubmitError::Load(daruda_flow::FlowError::Validate(issues))) => (
                s::flow::invalid_title(&name, issues.len()),
                issue_report(&issues),
            ),
            Err(FlowSubmitError::Load(daruda_flow::FlowError::Parse(detail))) => {
                (s::flow::parse_failed_title(), detail)
            }
            // Not about the flow: no lane, a remote lane, an unreadable
            // file. The same refusal `Run Flow…` would give.
            Err(other) => {
                self.report_flow_refusal(other, cx);
                return;
            }
        };
        super::dialog_helpers::open_alert_dialog(title, body, s::common::btn_close(), window, cx);
    }

    // ---- Stage 2: the run ----

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
    fn answer_flow_start_refusal(
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
        self.set_flow_scope(super::flow_browser::FlowScope::Worktree(lane), cx);
        self.set_flow_tab(super::flow_browser::FlowTab::Runs, cx);
        self.flows.browser.state.run_filter = super::flow_browser::RunFilter::All;
        self.clear_flow_search(window, cx);
        self.open_page(super::pages::Page::Flows, window, cx);
    }

    fn report_flow_refusal(&mut self, error: FlowSubmitError, cx: &mut Context<Self>) {
        let (message, detail) = flow_refusal_text(error);
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

/// A refusal's headline and detail, worded once for every surface that
/// reports one.
pub(in crate::workspace) fn flow_refusal_text(error: FlowSubmitError) -> (String, String) {
    match error {
        FlowSubmitError::NoLane => (s::flow::no_lane(), String::new()),
        FlowSubmitError::LaneUnresolvable { path } => (
            s::flow::lane_unresolvable(path.display().to_string()),
            String::new(),
        ),
        FlowSubmitError::RemoteLane { agent } => (s::flow::remote_lane(&agent), String::new()),
        FlowSubmitError::LockHeld { pid, lock_dir } => (
            s::flow::lock_held(pid),
            lock_dir.map_or_else(String::new, |dir| {
                s::flow::lock_held_detail(dir.join(".lock").display().to_string())
            }),
        ),
        // The reason carries its own already-localized wording — the same
        // one an agent chat pane shows for the identical refusal.
        FlowSubmitError::AgentLaunchRefused { agent, reason } => (
            s::flow::agent_launch_refused(&agent),
            match reason {
                ConnectCommandError::NoRemotePath => s::agent_chat::no_remote_cwd(),
                ConnectCommandError::JsonStdioRemote => {
                    s::agent_chat::json_stdio_remote_unsupported()
                }
                ConnectCommandError::JsonStdioEnv => s::agent_chat::json_stdio_env_unsupported(),
            },
        ),
        FlowSubmitError::UnusableSessionHost { agent, reason } => (
            s::flow::session_host_unusable(&agent),
            daruda_settings::labels::host_error(reason),
        ),
        FlowSubmitError::Read { path, message } => (
            s::flow::read_failed_title(),
            format!("{}: {message}", path.display()),
        ),
        FlowSubmitError::Load(daruda_flow::FlowError::Parse(detail)) => {
            (s::flow::parse_failed_title(), detail)
        }
        FlowSubmitError::Load(daruda_flow::FlowError::Validate(issues))
        | FlowSubmitError::Invalid(issues) => (
            s::flow::invalid_title("", issues.len()),
            issue_report(&issues),
        ),
        // The engine's own words: it is the one that decides what can be
        // continued, and a second wording here would be a second answer
        // to the same question.
        FlowSubmitError::Resume(e) => (s::flow::resume_refused(), e.to_string()),
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
        agent: AcpRunner::new(request.agents.clone(), node_install_dir).with_overage_stop(overage),
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
            s::flow::issue_line(
                issue.node.as_ref().map(|n| n.as_str()),
                &s::flow::issue(&issue.kind),
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
