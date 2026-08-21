//! One run's event stream and its parked questions: everything the engine
//! reports, arriving in workspace state.
//!
//! Two pumps, not one. The engine narrates on a `FlowEvent` channel and asks
//! permission on a separate `PendingAsk` channel, and the second carries a
//! reply back — folding it into the narration would make "can this host
//! answer" indistinguishable from "does it watch". Both drain here, and both
//! end where the workspace does.
//!
//! `flow_ops.rs` keeps submission and the stop switch: it decides that a run
//! may start, hands the channels' receiving ends to the two `watch_*` methods
//! below, and holds the [`CancelToken`](daruda_flow::runner::CancelToken) that
//! ends it.

use std::path::{Path, PathBuf};

use daruda_flow::event::{FlowEvent, RunEnd};
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use gpui::{Context, Window};

use super::Workspace;
use super::flow_ops::issue_report;
use super::flow_runs::{Advanced, Parked, ParkedAsk};
use crate::surface::strings as s;

impl Workspace {
    /// Drain the run's stream onto the UI. The channel is unbounded and the
    /// engine never awaits it, so falling behind here cannot slow the run.
    pub(in crate::workspace) fn watch_flow_events(
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
    pub(in crate::workspace) fn watch_flow_asks(
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
    pub(in crate::workspace) fn park_flow_ask(
        &mut self,
        lane_ref: daruda_store::project::LaneRef,
        run_dir: &Path,
        pending: daruda_flow::runner::PendingAsk,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let parked = self
            .runs
            .park_ask(lane_ref, run_dir, ParkedAsk::new(pending));
        cx.notify();

        // Only for the question that is now on screen. A queued arrival would
        // otherwise raise a modal for the *front* question — the one already
        // being answered — stacking a second copy of it, with the click landing
        // on whichever is on top.
        if parked != Parked::Showing {
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
        if self.runs.answer_ask(lane, ask_id, decision) {
            cx.notify();
        }
    }

    fn apply_flow_event(
        &mut self,
        lane_ref: daruda_store::project::LaneRef,
        event: &FlowEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.colour_flow_graph(lane_ref, event, cx);
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
    pub(in crate::workspace) fn advance_flow_stage(
        &mut self,
        lane_ref: daruda_store::project::LaneRef,
        event: &FlowEvent,
        cx: &mut Context<Self>,
    ) {
        // Leaving `Starting` is the first moment the run directory on disk is
        // the swept one: retention runs during start-up, after the run announces
        // itself and before its first node. Refreshing on the announcement
        // instead would read the pre-sweep listing and leave deleted runs on
        // screen for the length of the run.
        match self.runs.advance_stage(lane_ref, event) {
            Advanced::Nothing => return,
            Advanced::Moved { left_setup } => {
                if left_setup {
                    self.invalidate_flow_history(lane_ref);
                }
            }
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
        let run_dir = self.runs.retire(lane_ref);
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
        // Both halves the real path runs, in the same order. Only the
        // `RunEnded` report — which opens a pane and so needs a window — is
        // left to `apply_flow_event` itself.
        self.colour_flow_graph(lane, event, cx);
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
}

/// The run's report, when there is one to open.
///
/// A run the engine refuses before the lock takes nothing and writes nothing
/// — `execute` returns `not_started` before the run directory exists, and
/// `Invalid` and `LockHeld` are always that, so a report left by an earlier
/// run in the same directory is not theirs to open. An I/O failure can fall
/// on either side of the lock, so for every other end the file itself is the
/// answer.
fn report_to_open(end: &RunEnd, run_dir: &Path) -> Option<PathBuf> {
    if matches!(end, RunEnd::Invalid { .. } | RunEnd::LockHeld { .. }) {
        return None;
    }
    super::flow_history::report_in(run_dir)
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
    use super::*;

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
