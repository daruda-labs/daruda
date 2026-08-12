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
use super::command::flow_picker::{FlowPick, FlowPicker, FlowPurpose};
use super::flow_request::{FlowSubmission, FlowSubmitError, union_strip_env};
use crate::surface::strings as s;

/// One running flow, flattened for the render pass.
///
/// Shared by the two surfaces that draw runs — the status bar chip, which
/// lists every lane, and the Flows panel, which lists one. They differ in
/// which rows they get, not in what a row is.
///
/// `PartialEq` without `Eq`: the adapter's permission choices carry only
/// `PartialEq`, and the one thing this derive is for — the right dock's
/// notify-on-change diff — needs nothing more.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::workspace) struct FlowRunRow {
    pub lane: daruda_store::project::LaneRef,
    /// `<project>/<lane>` — a run in another lane is the case the chip
    /// exists for, so the row has to say which lane it is.
    pub lane_label: gpui::SharedString,
    /// What that run is doing, already worded (see `RunStage`).
    pub doing: gpui::SharedString,
    /// The question this run is waiting on, projected for display. The
    /// reply channel stays behind in `flow_runs` — the snapshot is compared
    /// field by field (`RightDockSnapshot::content_differs`) and a channel
    /// has no equality, so it could not travel here even if it should.
    pub asking: Option<AskRowData>,
    /// How many more are behind the one being shown. Zero for a run with a
    /// single question, which is every serial run.
    pub also_waiting: usize,
}

/// A parked question in the shape a surface draws, plus the id an answer
/// has to quote back.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::workspace) struct AskRowData {
    pub ask_id: u64,
    pub tool: gpui::SharedString,
    pub detail: Option<gpui::SharedString>,
    pub options: Vec<daruda_acp::PermissionChoice>,
}

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
    /// Waiting for a person to answer a permission question. Both clocks
    /// are stopped, so the only thing that ends this is an answer or a Stop.
    ///
    /// A variant rather than a flag beside an `Option`: a stage that is
    /// "asking" and has no question is not a state worth representing.
    ///
    /// More than one at a time is now possible — nodes running together ask
    /// independently — so the rest wait behind the one on screen. The queue
    /// lives *in* the variant because a queue of questions in a run that is
    /// not asking is not a state either.
    Asking {
        question: std::sync::Arc<ParkedAsk>,
        /// Arrived while `question` was up. Answering promotes the front.
        queued: std::collections::VecDeque<std::sync::Arc<ParkedAsk>>,
    },
}

/// A question waiting for an answer, and the way to send one.
///
/// `PartialEq` by `ask_id` alone: the reply channel has no equality, and
/// what the comparison is actually for is "is this the same question" —
/// which the id answers. `RunStage` keeps its derive because of it.
#[derive(Debug)]
pub(in crate::workspace) struct ParkedAsk {
    pub ask_id: u64,
    pub node: String,
    /// The attempt that asked — what the run goes back to being on once
    /// the question is answered.
    pub attempt: u32,
    pub tool: String,
    pub detail: Option<String>,
    pub options: Vec<daruda_acp::PermissionChoice>,
    reply: smol::channel::Sender<daruda_acp::PermissionDecision>,
}

impl PartialEq for ParkedAsk {
    fn eq(&self, other: &Self) -> bool {
        self.ask_id == other.ask_id
    }
}
impl Eq for ParkedAsk {}

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
            // Names the node like every other stage does: dropping it while
            // asking would leave the one stage that does not say where the
            // run is. For a repair's fix session the engine sends the gate's
            // name, which is what makes that case renderable at all.
            RunStage::Asking { question, .. } => {
                s::status_bar_flow_stage_asking(&question.node, &question.tool)
            }
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
            _ => self.flow_picker.open(
                purpose,
                super::flow_paths::list_flows(
                    &cwd,
                    &super::flow_paths::global_flows_dir(&self.data_dir),
                ),
            ),
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

        // A flow that declares profiles has one more question to answer,
        // so the picker stays up for it. Everything else is decided.
        if let Some(FlowPick::Flow(purpose, path)) = &picked {
            let profiles = flow_profiles(path);
            if !profiles.is_empty() {
                self.flow_picker
                    .ask_profile(*purpose, path.clone(), profiles);
                cx.notify();
                return;
            }
        }

        self.flow_picker.close();
        cx.notify();

        if was_stopping {
            self.stop_flow_run(cx);
            return;
        }
        let Some((purpose, path, profile)) = acted_on(picked) else {
            return;
        };
        match purpose {
            FlowPurpose::Validate => self.validate_flow(&path, profile.as_deref(), window, cx),
            FlowPurpose::Run => self.submit_flow_run(&path, profile.as_deref(), cx),
        }
    }

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
        let name = file_label(path);
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

    fn submit_flow_run(&mut self, path: &Path, profile: Option<&str>, cx: &mut Context<Self>) {
        match self.build_flow_request(path, profile, cx) {
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
            node_install_dir,
            events,
            asks,
        } = submission;
        let cancel = CancelToken::default();
        let run_dir = request.run_dir.clone();
        let run_dir_for_asks = run_dir.clone();
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
        if let Some(handle) = self.flow_runs.get_mut(&lane) {
            handle.cancel.cancel();
            // A stopped run has no question anyone should answer. Settled
            // here rather than when the run's `RunEnded` arrives, for the
            // same reason answering settles immediately: until then the
            // question and its buttons are still on screen with nothing to
            // say the Stop landed. The engine releases its own side — the
            // interrupt arm answers the adapter `Cancelled`.
            // The queue goes with it. Every question behind this one
            // belongs to the same stopped run, and the engine releases them
            // the same way it releases the one on screen.
            if let RunStage::Asking { question, .. } = &handle.doing {
                handle.doing = RunStage::Node {
                    id: question.node.clone(),
                    attempt: question.attempt,
                };
            }
        }
        cx.notify();
    }

    /// Every run this window started, in the shape the status bar draws.
    /// Across lanes on purpose — a run the user cannot currently see is
    /// exactly what the chip is for.
    pub(in crate::workspace) fn flow_status_rows(&self) -> Vec<FlowRunRow> {
        self.flow_rows_matching(|_| true)
    }

    /// The runs in the lane the right dock is showing. The panel answers
    /// questions and opens run directories, and both of those belong to one
    /// lane — `flow-runs/` is per working directory, so a panel spanning
    /// lanes could not show a coherent history beside them.
    pub(in crate::workspace) fn flow_rows_for_active_lane(&self) -> Vec<FlowRunRow> {
        let active = self.active;
        self.flow_rows_matching(|lane| lane == active)
    }

    /// The one place a run becomes a row, so the two surfaces cannot drift
    /// in how they word a stage or order themselves.
    fn flow_rows_matching(
        &self,
        keep: impl Fn(daruda_store::project::LaneRef) -> bool,
    ) -> Vec<FlowRunRow> {
        let mut rows: Vec<FlowRunRow> = self
            .flow_runs
            .iter()
            .filter(|(lane, _)| keep(**lane))
            .map(|(lane, handle)| FlowRunRow {
                lane: *lane,
                lane_label: self.lane_label_for(*lane).into(),
                doing: handle.doing.describe().into(),
                asking: match &handle.doing {
                    RunStage::Asking { question, .. } => Some(AskRowData {
                        ask_id: question.ask_id,
                        tool: question.tool.clone().into(),
                        detail: question.detail.clone().map(Into::into),
                        options: question.options.clone(),
                    }),
                    _ => None,
                },
                also_waiting: match &handle.doing {
                    RunStage::Asking { queued, .. } => queued.len(),
                    _ => 0,
                },
            })
            .collect();
        // A `HashMap` has no order, and a surface that reshuffles every
        // repaint is unreadable.
        rows.sort_by(|a, b| a.lane_label.cmp(&b.lane_label));
        rows
    }

    /// The active lane's past runs, reading them if the cache cannot
    /// answer. The **one** place the history is built.
    ///
    /// Derived here rather than pushed from each transition: the active
    /// lane changes at five call sites (activate, project add / close /
    /// rename, restore), and a refresh hook on each is a set the next one
    /// forgets to join. Comparing against the cache's own lane cannot be
    /// forgotten.
    ///
    /// Reads disk only when the Flows tab is showing and the cache is
    /// absent or built for another lane — so a tab the user is not on
    /// costs nothing, and the tab they are on costs one listing per
    /// change rather than one per frame.
    pub(in crate::workspace) fn flow_history_for_panel(
        &mut self,
    ) -> Option<super::flow_history::FlowHistory> {
        if self.right_dock_view != daruda_store::project::RightDockView::Flows {
            return None;
        }
        let lane = self.active;
        let fresh = self
            .flow_history
            .as_ref()
            .is_some_and(|cached| !cached.is_stale_for(lane));
        if !fresh {
            let cwd = self.active_lane_root()?;
            self.flow_history = Some(super::flow_history::FlowHistory::read(
                lane,
                &super::flow_paths::runs_dir(&cwd),
            ));
        }
        self.flow_history.clone()
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
        self.mutate_durable(cx, |ws, cx| {
            ws.right_dock.update(cx, |d, _| d.open());
            ws.main_area.pending_resize = true;
        });
        self.set_right_dock_view(daruda_store::project::RightDockView::Flows, cx);
        cx.notify();
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

    /// Drop the cached history so the next snapshot reads disk again.
    /// Scoped to the lane it happened in — another lane's run says nothing
    /// about this lane's directory.
    fn invalidate_flow_history(&mut self, lane: daruda_store::project::LaneRef) {
        if self
            .flow_history
            .as_ref()
            .is_some_and(|cached| !cached.is_stale_for(lane))
        {
            self.flow_history = None;
        }
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

    /// Drain the run's questions onto the UI. A second pump beside the
    /// event one: the two carry different things in different directions,
    /// and folding a reply channel into the narration stream would make
    /// "can this host answer" indistinguishable from "does it watch".
    fn watch_flow_asks(
        &mut self,
        lane_ref: daruda_store::project::LaneRef,
        run_dir: PathBuf,
        asks: smol::channel::Receiver<daruda_flow::runner::PendingAsk>,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            while let Ok(pending) = asks.recv().await {
                let run_dir = run_dir.clone();
                // `update_in` rather than `update`: parking also raises the
                // question as a modal when its lane is the one in view, and
                // opening a dialog needs the window.
                if this
                    .update_in(cx, |workspace, window, cx| {
                        workspace.park_flow_ask(lane_ref, &run_dir, pending, window, cx);
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    /// Hold a question until somebody answers it.
    ///
    /// `run_dir` names the run the question came from. A channel drains
    /// before it closes, so a question the engine queued as its run was
    /// stopping can still arrive after the next run in that lane has taken
    /// the slot — and `ask_id` restarts at 1 per run, so it would land on
    /// the newcomer looking exactly like its own. Dropping it releases the
    /// old run's engine, which is the only thing still waiting on it.
    fn park_flow_ask(
        &mut self,
        lane_ref: daruda_store::project::LaneRef,
        run_dir: &Path,
        pending: daruda_flow::runner::PendingAsk,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(handle) = self.flow_runs.get_mut(&lane_ref) else {
            return;
        };
        if handle.run_dir != run_dir {
            return;
        }
        let arrived = std::sync::Arc::new(ParkedAsk {
            ask_id: pending.ask_id,
            node: pending.node,
            attempt: pending.attempt,
            tool: pending.request.tool,
            detail: pending.request.detail,
            options: pending.request.options,
            reply: pending.reply,
        });
        // Behind the one already up, never over it. Nodes running together
        // ask independently, and a second question that replaced the first
        // would leave that first node parked on a reply nobody can send —
        // the run would wait forever on a question no longer on screen.
        let now_showing = match &mut handle.doing {
            RunStage::Asking { queued, .. } => {
                queued.push_back(arrived);
                false
            }
            doing => {
                *doing = RunStage::Asking {
                    question: arrived,
                    queued: std::collections::VecDeque::new(),
                };
                true
            }
        };
        cx.notify();

        // Only for the question that is now on screen. A queued arrival
        // would otherwise raise a modal for the *front* question — the one
        // already being answered — stacking a second copy of it, with the
        // click landing on whichever is on top.
        if !now_showing {
            return;
        }

        // Rows for *this* lane, not the active one: whether it may take the
        // window is the modal's rule, and pre-filtering here would enforce
        // it twice — the copy that never runs is the one nothing tests.
        if let Some(row) = self
            .flow_rows_matching(|lane| lane == lane_ref)
            .into_iter()
            .find(|row| row.lane == lane_ref)
            .and_then(|row| row.asking)
        {
            super::flow_ask_modal::FlowAskModal::raise_if_in_view(
                cx.weak_entity(),
                self.active,
                lane_ref,
                row,
                window,
                cx,
            );
        }
    }

    /// Answer the question `lane` is holding.
    ///
    /// `ask_id` is checked rather than trusted: a surface can still be
    /// painted with a question that has already been answered, and a click
    /// on that frame must do nothing instead of answering the *next* one.
    pub(in crate::workspace) fn answer_flow_ask(
        &mut self,
        lane: daruda_store::project::LaneRef,
        ask_id: u64,
        decision: daruda_acp::PermissionDecision,
        cx: &mut Context<Self>,
    ) {
        let Some(handle) = self.flow_runs.get_mut(&lane) else {
            return;
        };
        let RunStage::Asking { question, queued } = &mut handle.doing else {
            return;
        };
        if question.ask_id != ask_id {
            return;
        }
        // Bounded to one, so this cannot block and a second send cannot
        // land.
        let _ = question.reply.try_send(decision);
        // Off this question now, not when the run's next event happens to
        // arrive: the agent goes back to work for as long as it likes, and
        // until then the question and its buttons would still be on screen
        // with no sign the click did anything. Observed as "the button does
        // nothing" — and answered again, and again.
        //
        // The next one takes its place if there is one. Answering must not
        // hide a question that is still waiting for an answer.
        handle.doing = match queued.pop_front() {
            Some(next) => RunStage::Asking {
                question: next,
                queued: std::mem::take(queued),
            },
            None => RunStage::Node {
                id: question.node.clone(),
                attempt: question.attempt,
            },
        };
        cx.notify();
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
        if handle.doing == stage {
            return;
        }
        // Leaving `Starting` is the first moment the run directory on disk
        // is the swept one: retention runs during start-up, after the run
        // announces itself and before its first node. Refreshing on the
        // announcement instead would read the pre-sweep listing and leave
        // deleted runs on screen for the length of the run.
        let past_setup = handle.doing == RunStage::Starting;
        handle.doing = stage;
        if past_setup {
            self.invalidate_flow_history(lane_ref);
        }
        cx.notify();
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
        // The run just wrote its completion marker, so the list that reads
        // those markers is now wrong.
        self.invalidate_flow_history(lane_ref);
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

    /// Put a killed run in the panel's history, for `--screenshot`. The
    /// one row with a way back into it — and the only state here that no
    /// scenario can reach without writing into a real repository.
    #[cfg(feature = "screenshot")]
    pub(in crate::workspace) fn seed_crashed_run_for_shot(&mut self, cx: &mut Context<Self>) {
        // The dock too, not only the tab: it restores closed as often as
        // open, and a capture of the panel behind a collapsed dock shows
        // nothing at all.
        self.right_dock.update(cx, |dock, _| dock.open());
        self.main_area.pending_resize = true;
        self.set_right_dock_view(daruda_store::project::RightDockView::Flows, cx);
        let lane = self.active;
        let dir = super::flow_paths::runs_dir(&self.active_lane_root().unwrap_or_default());
        self.flow_history = Some(super::flow_history::FlowHistory::for_shot(lane, dir));
        cx.notify();
    }

    /// Put the picker on its second question, for `--screenshot`.
    ///
    /// Reaches the state directly instead of picking a flow: picking one
    /// under `FlowPurpose::Run` *submits* it, and a capture that starts a
    /// real run leaves a run directory behind every time it is taken.
    #[cfg(feature = "screenshot")]
    pub(in crate::workspace) fn ask_flow_profile_for_shot(&mut self, cx: &mut Context<Self>) {
        let Some(cwd) = self.active_lane_root() else {
            return;
        };
        let global = super::flow_paths::global_flows_dir(&self.data_dir);
        let Some((path, profiles)) = super::flow_paths::list_flows(&cwd, &global)
            .into_iter()
            .find_map(|found| {
                let names = flow_profiles(&found.path);
                (!names.is_empty()).then_some((found.path, names))
            })
        else {
            return;
        };
        self.flow_picker
            .ask_profile(FlowPurpose::Run, path, profiles);
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
        self.flow_runs.insert(
            lane,
            RunHandle {
                cancel: CancelToken::default(),
                run_dir: run_dir.clone(),
                doing: RunStage::Node {
                    id: "verdict".to_string(),
                    attempt: 2,
                },
                _thread: std::thread::spawn(|| {}),
            },
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
                    node: "implement".to_string(),
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

    /// Drive one stream event through the real stage machine. The window
    /// is only needed by the `RunEnded` arm (it opens the report), so a
    /// test of the stage transitions reaches the same code without one.
    #[cfg(test)]
    pub(in crate::workspace) fn apply_flow_event_for_test(
        &mut self,
        lane: daruda_store::project::LaneRef,
        event: &FlowEvent,
        cx: &mut Context<Self>,
    ) {
        self.advance_flow_stage(lane, event, cx);
    }

    /// Park a question on a seeded run, so a test can exercise answering
    /// without an adapter.
    #[cfg(test)]
    pub(in crate::workspace) fn park_flow_ask_for_test(
        &mut self,
        lane: daruda_store::project::LaneRef,
        run_dir: &Path,
        pending: daruda_flow::runner::PendingAsk,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.park_flow_ask(lane, run_dir, pending, window, cx);
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

/// What a pick asks for: the flow, and the profile if a second question
/// was answered. Separate from the dispatch below so the unpacking is
/// checkable — the arm that drops the name here is the one that would make
/// every profiled run silently run as plain `defaults`.
fn acted_on(picked: Option<FlowPick>) -> Option<(FlowPurpose, PathBuf, Option<String>)> {
    match picked {
        Some(FlowPick::Flow(purpose, path)) => Some((purpose, path, None)),
        Some(FlowPick::Profile(purpose, path, profile)) => Some((purpose, path, profile)),
        None => None,
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
    /// The second question's answer has to survive the unpacking. Dropping
    /// it here is invisible from either surface — the picker looks right,
    /// the run starts, and it runs as plain `defaults`.
    #[test]
    fn an_answered_profile_reaches_what_the_run_is_built_from() {
        use super::{FlowPick, FlowPurpose, acted_on};
        let path = std::path::PathBuf::from("/lane/.daruda/flows/ship.yaml");

        assert_eq!(
            acted_on(Some(FlowPick::Profile(
                FlowPurpose::Run,
                path.clone(),
                Some("cheap".to_string())
            ))),
            Some((FlowPurpose::Run, path.clone(), Some("cheap".to_string())))
        );
        // A flow that declares none is run as written, not under a name
        // invented for it.
        assert_eq!(
            acted_on(Some(FlowPick::Flow(FlowPurpose::Validate, path.clone()))),
            Some((FlowPurpose::Validate, path, None))
        );
        assert_eq!(acted_on(None), None);
    }
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
