//! Running a flow from the app: the picker, static checking, submission,
//! and the stop switch.
//!
//! The engine's `execute` blocks, so a run owns a thread and the workspace
//! owns its [`CancelToken`]. Everything the run has to say comes back on
//! one `FlowEvent` stream — there is no second completion channel, because
//! two would be two things to keep in step.

use std::path::{Path, PathBuf};

use daruda_flow::event::{FlowEvent, RunEnd};
use daruda_flow::runner::{AcpRunner, CancelToken, ProcessRunner, Runners};
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use gpui::{Context, Window};

use super::Workspace;
use super::command::flow_picker::{FlowPicker, FlowPurpose};
use super::flow_request::{FlowSubmission, FlowSubmitError, union_strip_env};
use crate::surface::strings as s;

/// A run in flight. The token is the whole of the stop switch; the handle
/// is kept so the workspace can tell a finished run from a wedged one
/// without asking the engine.
pub(in crate::workspace) struct RunHandle {
    pub cancel: CancelToken,
    pub run_dir: PathBuf,
    /// What the run is doing right now, as its own stream reports it.
    pub doing: RunStage,
    _thread: std::thread::JoinHandle<()>,
}

/// Where a run is, in the only terms that stay true.
///
/// Deliberately not "node N of M": a repair sends nodes back to pending,
/// so a count runs backwards — and repair is this engine's headline
/// feature, not an edge case. What a person actually wants at a glance is
/// which node is on, and whether it is on its second try.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::workspace) enum RunStage {
    /// Submitted; the engine has not announced a node yet.
    Starting,
    Node {
        id: String,
        attempt: u32,
    },
    /// A gate's repair is running its fix session.
    Fixing {
        gate: String,
    },
    /// The fix is done and the gate is about to be re-derived. Not
    /// `Node { attempt: 0 }`: a zeroth attempt does not exist, and the
    /// number would have to be read as a sentinel at every use.
    Rederiving {
        gate: String,
    },
}

impl RunStage {
    /// What to show for this stage. `attempt` is only worth saying past the
    /// first — every node has a first try, and "try 1" on every row is
    /// noise that buries the row where it says "try 3".
    pub(in crate::workspace) fn describe(&self) -> String {
        match self {
            RunStage::Starting => s::status_bar_flow_stage_starting(),
            RunStage::Node { id, attempt } if *attempt > 1 => {
                s::status_bar_flow_stage_node_retry(id, *attempt)
            }
            RunStage::Node { id, .. } => s::status_bar_flow_stage_node(id),
            RunStage::Fixing { gate } => s::status_bar_flow_stage_fixing(gate),
            RunStage::Rederiving { gate } => s::status_bar_flow_stage_rederiving(gate),
        }
    }
}

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
        let Some(cwd) = self.active_lane_root() else {
            self.report_flow_refusal(FlowSubmitError::NoLane, cx);
            return;
        };
        // Design §14's affordance: what says a run is going is the lock,
        // not a field this app keeps — so a run started by a previous
        // session, or by another window, is recognised the same way.
        match (purpose, self.lane_holder(&cwd)) {
            // The stop switch is a `CancelToken`, so the only run we can
            // stop is one whose token we are holding — for *this* lane.
            // The map is the authority, not the lock's pid: a token for
            // another lane belongs to this process too and would not stop
            // this one.
            (FlowPurpose::Run, Some(_)) if self.flow_runs.contains_key(&self.active) => {
                self.flow_picker = FlowPicker::Stopping;
            }
            // Offering "stop it" for a run we cannot reach would be a
            // button that does nothing.
            (FlowPurpose::Run, Some(holder)) => {
                self.report_flow_refusal(FlowSubmitError::LockHeld { pid: holder.pid }, cx);
                return;
            }
            _ => self
                .flow_picker
                .open(purpose, super::flow_paths::list_flows(&cwd)),
        }
        cx.notify();
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
        self.flow_picker.close();
        cx.notify();

        if was_stopping {
            self.stop_flow_run(cx);
            return;
        }
        match picked {
            Some((FlowPurpose::Validate, path)) => self.validate_flow(&path, window, cx),
            Some((FlowPurpose::Run, path)) => self.submit_flow_run(&path, cx),
            None => {}
        }
    }

    // ---- Stage 1: static checks, which cost nothing ----

    /// Report what can be known without running the flow. Opens no session,
    /// takes no lock, and creates no run directory.
    fn validate_flow(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let name = file_label(path);
        let (title, body) = match self.check_flow(path, cx) {
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

    fn submit_flow_run(&mut self, path: &Path, cx: &mut Context<Self>) {
        let submission = match self.build_flow_request(path, cx) {
            Ok(submission) => submission,
            Err(e) => {
                self.report_flow_refusal(e, cx);
                return;
            }
        };
        let FlowSubmission {
            request,
            node_install_dir,
            events,
        } = submission;
        let cancel = CancelToken::default();
        let run_dir = request.run_dir.clone();
        // Captured now: the run belongs to the lane it was submitted from,
        // not to whichever one is active when it ends.
        let lane_ref = self.active;

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
        self.flow_runs.insert(
            lane_ref,
            RunHandle {
                cancel,
                run_dir,
                doing: RunStage::Starting,
                _thread: thread,
            },
        );
        self.watch_flow_events(lane_ref, events, cx);
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
        if let Some(handle) = self.flow_runs.get(&lane) {
            handle.cancel.cancel();
        }
        cx.notify();
    }

    /// Every run this window started, in the shape the status bar draws.
    pub(in crate::workspace) fn flow_status_rows(&self) -> Vec<super::status_bar::FlowRunRow> {
        let mut rows: Vec<super::status_bar::FlowRunRow> = self
            .flow_runs
            .iter()
            .map(|(lane, handle)| super::status_bar::FlowRunRow {
                lane: *lane,
                lane_label: self.lane_label_for(*lane).into(),
                doing: handle.doing.describe().into(),
            })
            .collect();
        // A `HashMap` has no order, and a chip that reshuffles every repaint
        // is unreadable.
        rows.sort_by(|a, b| a.lane_label.cmp(&b.lane_label));
        rows
    }

    /// `<project> / <lane>`, the same shape the lane switcher shows.
    fn lane_label_for(&self, lane_ref: daruda_store::project::LaneRef) -> String {
        let Some(project) = self.projects.iter().find(|p| p.id == lane_ref.project) else {
            return String::new();
        };
        match project.lanes.iter().find(|l| l.id == lane_ref.lane) {
            Some(lane) => format!("{} / {}", project.name, lane.display_name()),
            None => project.name.clone(),
        }
    }

    /// Drain the run's stream onto the UI. The channel is unbounded and the
    /// engine never awaits it, so falling behind here cannot slow the run.
    fn watch_flow_events(
        &mut self,
        lane_ref: daruda_store::project::LaneRef,
        events: smol::channel::Receiver<FlowEvent>,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            while let Ok(event) = events.recv().await {
                let is_end = matches!(event, FlowEvent::RunEnded { .. });
                // An `Err` here is the workspace being gone, which ends the
                // pump — the same shape the MCP and JSONL pumps use, and
                // the reason this is not a silently discarded `Result`.
                if this
                    .update_in(cx, |workspace, window, cx| {
                        workspace.apply_flow_event(lane_ref, &event, window, cx);
                    })
                    .is_err()
                    || is_end
                {
                    break;
                }
            }
        })
        .detach();
    }

    fn apply_flow_event(
        &mut self,
        lane_ref: daruda_store::project::LaneRef,
        event: &FlowEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let FlowEvent::RunEnded { end } = event else {
            self.advance_flow_stage(lane_ref, event, cx);
            return;
        };
        // What a completion toast would have said, in the form a person can
        // actually read afterwards — the run's own narrative. Opened under
        // the lane the run belongs to, which may no longer be the active
        // one by the time a long run ends.
        if let Some(report) = self.settle_flow_run(lane_ref, end, cx) {
            self.open_pane_file_view(
                lane_ref.lane,
                report,
                /* staged = */ false,
                /* file_status = */ None,
                super::main_area::file_view_pane::FileViewMode::Preview,
                window,
                cx,
            );
        }
    }

    /// Move the run's reported stage along. One `cx.notify()` per node
    /// boundary, which is as often as the engine has anything new to say —
    /// far too rarely to be a repaint concern.
    fn advance_flow_stage(
        &mut self,
        lane_ref: daruda_store::project::LaneRef,
        event: &FlowEvent,
        cx: &mut Context<Self>,
    ) {
        let stage = match event {
            FlowEvent::NodeStarted { node, attempt } => RunStage::Node {
                id: node.clone(),
                attempt: *attempt,
            },
            FlowEvent::FixStarted { gate } => RunStage::Fixing { gate: gate.clone() },
            // The fix is over but its gate has not re-run yet, and the
            // members about to be re-derived have not started either.
            FlowEvent::FixEnded { gate, .. } | FlowEvent::Rerunning { gate, .. } => {
                RunStage::Rederiving { gate: gate.clone() }
            }
            // `RunStarted` leaves `Starting`; the passes and failures are
            // already described by whichever node starts next.
            _ => return,
        };
        let Some(handle) = self.flow_runs.get_mut(&lane_ref) else {
            return;
        };
        if handle.doing != stage {
            handle.doing = stage;
            cx.notify();
        }
    }

    /// Retire the run `lane_ref` was holding and say what it left to read.
    /// Separate from opening it so the settling — which lane is released,
    /// and what the user is told — is decided without a `Window`.
    pub(in crate::workspace) fn settle_flow_run(
        &mut self,
        lane_ref: daruda_store::project::LaneRef,
        end: &RunEnd,
        cx: &mut Context<Self>,
    ) -> Option<PathBuf> {
        let run_dir = self
            .flow_runs
            .remove(&lane_ref)
            .map(|handle| handle.run_dir);
        if let Some(message) = end_refusal(end) {
            self.report_error(
                ErrorReport::new(message)
                    .severity(ErrorSeverity::Warning)
                    .at(file!(), line!())
                    .dedup("flow.run.ended")
                    .build(),
                cx,
            );
        }
        cx.notify();
        run_dir.as_deref().and_then(|dir| report_to_open(end, dir))
    }

    /// Park a run handle for `lane` without starting anything, so a test
    /// can exercise which lane a finished run settles.
    /// Put a run on screen without one running, for `--screenshot`. The
    /// chip is only reachable while a flow is mid-flight, which no capture
    /// can wait for.
    #[cfg(feature = "screenshot")]
    pub(in crate::workspace) fn seed_flow_run_for_shot(&mut self, cx: &mut Context<Self>) {
        let lane = self.active;
        self.flow_runs.insert(
            lane,
            RunHandle {
                cancel: CancelToken::default(),
                run_dir: self.active_lane_root().unwrap_or_default(),
                doing: RunStage::Node {
                    id: "verdict".to_string(),
                    attempt: 2,
                },
                _thread: std::thread::spawn(|| {}),
            },
        );
        cx.notify();
    }

    #[cfg(test)]
    pub(in crate::workspace) fn seed_flow_run_for_test(
        &mut self,
        lane: daruda_store::project::LaneRef,
        run_dir: PathBuf,
    ) {
        self.flow_runs.insert(
            lane,
            RunHandle {
                cancel: CancelToken::default(),
                run_dir,
                doing: RunStage::Starting,
                _thread: std::thread::spawn(|| {}),
            },
        );
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

/// The run's report, when there is one to open.
///
/// A run the engine refuses before the lock takes nothing and writes
/// nothing — `execute` returns `not_started` before the run directory
/// exists. `Invalid` and `LockHeld` are always that. An I/O failure can
/// fall on either side of the lock, so for the rest the file itself is the
/// answer; opening a path that is not there would replace the warning the
/// user needs with an unrelated one about a missing file.
fn report_to_open(end: &RunEnd, run_dir: &Path) -> Option<PathBuf> {
    if matches!(end, RunEnd::Invalid { .. } | RunEnd::LockHeld { .. }) {
        return None;
    }
    let report = run_dir.join(daruda_flow::record::RUN_REPORT_FILE);
    report.is_file().then_some(report)
}

/// The file name, which is what the picker showed and therefore what the
/// user is expecting to read about.
fn file_label(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Every problem, in the wording the user gets. `ValidationIssue.message`
/// is developer detail and deliberately never appears here.
fn issue_report(issues: &[daruda_flow::error::ValidationIssue]) -> String {
    issues
        .iter()
        .map(|issue| s::flow_issue_line(issue.node.as_deref(), &s::flow_issue(&issue.kind)))
        .collect::<Vec<_>>()
        .join("\n")
}

/// What to say about a run that has ended, or `None` when it ended the way
/// it was meant to and `run.md` is the whole story.
fn end_refusal(end: &RunEnd) -> Option<String> {
    match end {
        RunEnd::Done | RunEnd::Failed { .. } | RunEnd::Canceled { .. } => None,
        RunEnd::BudgetExhausted { limit } => Some(s::flow_budget_exhausted(*limit)),
        RunEnd::Io { message, .. } => Some(message.clone()),
        RunEnd::LockHeld { holder } => Some(s::flow_lock_held(holder.pid)),
        RunEnd::Invalid { issues } => Some(issue_report(issues)),
        RunEnd::Unprovisioned { agent, message } => Some(s::flow_unprovisioned(agent, message)),
    }
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
        let loaded = daruda_flow::load(MIXED_FLOW).expect("the fixture flow loads");
        RunRequest {
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
            node: Some("gate".to_string()),
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
                node: Some("a".to_string()),
                kind: ValidationKind::MissingAgent,
                message: String::new(),
            },
            ValidationIssue {
                node: Some("b".to_string()),
                kind: ValidationKind::DuplicateOutput,
                message: String::new(),
            },
        ];
        assert_eq!(issue_report(&issues).lines().count(), 2);
    }

    /// A run the engine refused before the lock wrote no report, so there
    /// is nothing to open — and opening a path that is not there replaces
    /// the warning the user needs with an unrelated one about a missing
    /// file.
    #[test]
    fn a_run_that_never_started_has_no_report_to_open() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Even with a stale report from an earlier run sitting right there.
        std::fs::write(
            dir.path().join(daruda_flow::record::RUN_REPORT_FILE),
            "# Run",
        )
        .expect("write");

        let holder = daruda_flow::lock::LockHolder {
            pid: 1,
            run_id: "other".to_string(),
            started_unix_secs: 1,
        };
        assert!(report_to_open(&RunEnd::LockHeld { holder }, dir.path()).is_none());
        assert!(report_to_open(&RunEnd::Invalid { issues: Vec::new() }, dir.path()).is_none());
        assert!(report_to_open(&RunEnd::Done, dir.path()).is_some());

        // An I/O failure falls on either side of the lock, so for anything
        // but the two above the file itself is the only honest answer.
        let empty = tempfile::tempdir().expect("tempdir");
        assert!(report_to_open(&RunEnd::Done, empty.path()).is_none());
    }

    /// A run that ended the way it was asked to has nothing to report — its
    /// story is `run.md`, which opens either way. Only a run that never got
    /// to tell one needs a message.
    #[test]
    fn only_a_run_that_could_not_speak_for_itself_reports() {
        assert!(end_refusal(&RunEnd::Done).is_none());
        assert!(
            end_refusal(&RunEnd::Canceled { node: None }).is_none(),
            "the user pressed stop; telling them they stopped it is noise"
        );
        assert!(
            end_refusal(&RunEnd::Unprovisioned {
                agent: "claude".to_string(),
                message: "no managed Node.js build".to_string(),
            })
            .is_some()
        );
    }
}
