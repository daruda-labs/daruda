//! Desktop flow dispatch: run guards, profile selection, and picker actions.
//! Submission and stopping live in `runtime`; validation, navigation, and
//! shared refusal wording live in `reporting`.
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

use std::path::{Path, PathBuf};

use gpui::{Context, Window};

use super::Workspace;
use super::command::flow_picker::{FlowPick, FlowPicker, FlowPurpose};
use super::command::picker::PickerKey;
use super::command::picker_key::picker_keystroke;
use super::flow_inputs_modal::{FlowInputsModal, PendingFlowRun};
use super::flow_request::{FlowSelection, FlowSubmitError};

mod reporting;
mod runtime;
pub(in crate::workspace) mod seeds;

pub(in crate::workspace) use reporting::flow_refusal_text;
pub(super) use reporting::issue_report;

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
            let profiles = super::flow_declared::declared_profiles(&path);
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
                // The last question a start asks, once the profile is known:
                // the values its declared inputs take this time.
                if selection.inputs.is_none() {
                    let decls = super::flow_declared::declared_inputs(path);
                    if !decls.is_empty() {
                        let pending = PendingFlowRun {
                            lane,
                            path: path.to_path_buf(),
                            profile: profile.map(str::to_owned),
                            selection: selection.clone(),
                        };
                        let _form = FlowInputsModal::open(
                            cx.entity().downgrade(),
                            pending,
                            decls,
                            window,
                            cx,
                        );
                        return;
                    }
                }
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

    /// Run a flow whose inputs the person has just given values for.
    pub(in crate::workspace) fn run_flow_with_inputs(
        &mut self,
        pending: PendingFlowRun,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_flow(
            FlowDispatch {
                lane: pending.lane,
                purpose: FlowPurpose::Run,
                path: &pending.path,
                profile: pending.profile.as_deref(),
                selection: &pending.selection,
            },
            window,
            cx,
        );
    }
}
