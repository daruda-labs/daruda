//! What a window does when a flow run in one of its worktrees changes.
//!
//! The run belongs to the process ([`crate::runs::RunsHost`]), which folds
//! its notices and questions into the view every window reads and then tells
//! the windows through [`crate::runs::RunsEffects`]. A window that holds the
//! run's worktree repaints, recolours its graph and raises a question; one
//! window — the lead — also speaks for the run's end: the toast and the
//! refusal. The phone that asked for a run is answered before any window,
//! so it is answered with none open. Windows that do not hold the worktree
//! are not touched at all.

use std::path::{Path, PathBuf};

use daruda_flow::event::RunEnd;
use daruda_flow::lock::CanonicalTree;
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::project::LaneRef;
use gpui::{App, Context, Window};

use super::Workspace;
use super::flow_ops::issue_report;
use crate::runs::{AnswerRefused, FlowChange, FlowChanged, FlowEnded, RunsHost};
use crate::surface::strings as s;
use crate::ui::WindowExt as _;
use crate::window_registry::WindowRegistry;

/// Hand `change` to every window, with the one that speaks for the run
/// chosen first: the window in front if it holds the worktree, else one
/// showing that worktree, else the first that holds it, else — for a run
/// whose window closed — any window, so the end is still said somewhere.
pub(in crate::workspace) fn deliver_flow_change(change: &FlowChange, cx: &mut App) {
    super::flow_ask_delivery::deliver(change, cx);
    let lead = match &change.what {
        FlowChanged::Ended(ended) => {
            if ended.owes_remote_answer {
                answer_remote_caller(ended, cx);
            }
            lead_window_for(&change.worktree, cx)
        }
        _ => None,
    };
    WindowRegistry::for_each_workspace(cx, |ws, window, cx| {
        let is_lead = lead == Some(cx.entity_id());
        ws.apply_flow_change(change, is_lead, window, cx);
    });
}

/// Tell the phone that asked for a run how it ended. Sent here, once per
/// end, rather than by a window: the app keeps running with every window
/// closed, and the caller is owed an answer either way.
fn answer_remote_caller(ended: &FlowEnded, cx: &App) {
    let refusal = end_refusal(&ended.end);
    let notice = flow_outcome_notice(&ended.end, refusal, ended.overage.as_ref());
    crate::remote_channel::global::RemoteChannels::send_notice(notice.clone(), cx);
    if let Some(bridge) = cx.try_global::<crate::telegram::global::TelegramBridge>() {
        bridge.send_notice(notice, cx);
    }
}

fn lead_window_for(worktree: &CanonicalTree, cx: &mut App) -> Option<gpui::EntityId> {
    let mut holders = Vec::new();
    let mut showing = None;
    let mut any = None;
    WindowRegistry::for_each_workspace(cx, |ws, _window, cx| {
        any.get_or_insert(cx.entity_id());
        if let Some(lane) = ws.lane_at(worktree, cx) {
            holders.push(cx.entity_id());
            if lane == ws.active {
                showing.get_or_insert(cx.entity_id());
            }
        }
    });
    let front = WindowRegistry::active_workspace(cx).map(|(_, weak)| weak.entity_id());
    front
        .filter(|front| holders.contains(front))
        .or(showing)
        .or_else(|| holders.first().copied())
        .or(any)
}

impl Workspace {
    /// React to a change in the run at `change.worktree`.
    fn apply_flow_change(
        &mut self,
        change: &FlowChange,
        lead: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_stale_flow_question(window, cx);
        if matches!(
            change.what,
            FlowChanged::Asked { .. } | FlowChanged::AskClosed { .. }
        ) {
            cx.notify();
        }
        let lanes = self.lanes_at(&change.worktree, cx);
        let lane = lanes.first().copied();
        if self.active_page() == Some(super::pages::Page::Flows)
            && matches!(
                change.what,
                FlowChanged::Ended(_) | FlowChanged::Progressed { .. }
            )
            && let Some(detail) = &self.pages.flows.detail
            && lanes.contains(&detail.lane)
            && matches!(
                detail.body,
                super::pages::flows::detail::FlowDetailBody::Run(_)
            )
        {
            if matches!(change.what, FlowChanged::Ended(_)) {
                self.refresh_execution(detail.id, cx);
            } else {
                self.refresh_execution_progress(detail.id, cx);
            }
        }
        match (&change.what, lane) {
            (FlowChanged::Ended(ended), lane) => {
                for &held in &lanes {
                    // The run just wrote its completion marker, so the list
                    // that reads those markers is now wrong.
                    self.invalidate_flow_history(held);
                    self.colour_flow_graph(held, ended.colouring.clone(), cx);
                }
                self.on_flow_run_ended(lane, change.worktree.as_path(), ended, lead, cx);
            }
            (_, None) => {}
            (FlowChanged::Repainted | FlowChanged::AskClosed { .. }, Some(_)) => cx.notify(),
            (FlowChanged::Progressed { left_setup }, Some(_)) => {
                let colouring = RunsHost::flow_run(change.worktree.as_path(), cx)
                    .and_then(|run| run.view().colouring());
                for &held in &lanes {
                    // Leaving setup is the first moment the run directory on
                    // disk is the swept one: retention runs after the run
                    // announces itself and before its first node.
                    if *left_setup {
                        self.invalidate_flow_history(held);
                    }
                    self.colour_flow_graph(held, colouring.clone(), cx);
                }
                cx.notify();
            }
            (
                FlowChanged::Asked {
                    run_dir,
                    ask_id,
                    alone,
                    ..
                },
                Some(lane),
            ) => {
                cx.notify();
                // Only a question that is alone comes forward: a dialog
                // stacked over one being answered would take the click.
                if *alone {
                    let worktree = change.worktree.as_path();
                    self.raise_flow_question(lane, worktree, run_dir, *ask_id, window, cx);
                }
            }
        }
    }

    fn raise_flow_question(
        &mut self,
        lane: LaneRef,
        worktree: &Path,
        run_dir: &Path,
        ask_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(row) = RunsHost::flow_run(worktree, cx)
            .filter(|run| run.view().run_dir() == run_dir)
            .and_then(|run| run.view().question(ask_id))
            .filter(|ask| !ask.is_closed())
            .map(super::flow_rows::AskRowData::of)
        else {
            return;
        };
        let raised = super::flow_ask_modal::FlowAskModal::raise_if_in_view(
            cx.weak_entity(),
            self.active,
            lane,
            run_dir.to_path_buf(),
            row,
            window,
            cx,
        );
        if let Some(modal) = raised {
            self.flows.ask_modal = Some(modal.downgrade());
        }
    }

    /// Close this window's question modal once its question is no longer
    /// open — answered or stopped in another window, abandoned, or its run
    /// ended — so no buttons stay up that answer nothing.
    fn close_stale_flow_question(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(modal) = self.flows.ask_modal.as_ref().and_then(|m| m.upgrade()) else {
            self.flows.ask_modal = None;
            return;
        };
        let (lane, run_dir, ask_id) = modal.read(cx).question();
        let open = self.flow_run_in(lane, cx).is_some_and(|run| {
            run.view().run_dir() == run_dir
                && run
                    .view()
                    .question(ask_id)
                    .is_some_and(|ask| !ask.is_closed())
        });
        if open {
            return;
        }
        self.flows.ask_modal = None;
        // Another dialog stacked over it would close in its place.
        if modal.read(cx).is_in_front(window, cx) {
            window.close_dialog(cx);
        }
    }

    /// Answer question `ask_id` of the run in `lane`.
    ///
    /// `ask_id` is checked rather than trusted: a surface can still be
    /// painted with a question that has already been answered, and that
    /// click does nothing. A question the run had already stopped waiting
    /// for is said to have failed, rather than looking answered.
    pub(in crate::workspace) fn answer_flow_ask(
        &mut self,
        lane: LaneRef,
        run_dir: &Path,
        ask_id: u64,
        decision: daruda_acp::PermissionDecision,
        cx: &mut Context<Self>,
    ) {
        let Some(worktree) = self.lane_for(lane).map(|lane| lane.path.clone()) else {
            return;
        };
        match RunsHost::answer_flow_ask(&worktree, run_dir, ask_id, decision, cx) {
            Ok(()) | Err(AnswerRefused::NotOpen | AnswerRefused::AlreadyAnswered) => {}
            Err(AnswerRefused::Closed) => self.report_error(
                ErrorReport::new(s::flow::ask_closed())
                    .severity(ErrorSeverity::Warning)
                    .at(file!(), line!())
                    .dedup("flow.ask.closed")
                    .build(),
                cx,
            ),
        }
    }

    /// A run in `worktree` ended, in `lane` — this window's lane there, the
    /// one in view first. Every window holding it marks it; the lead alone
    /// says so.
    fn on_flow_run_ended(
        &mut self,
        lane: Option<LaneRef>,
        worktree: &Path,
        ended: &FlowEnded,
        lead: bool,
        cx: &mut Context<Self>,
    ) {
        if lane.is_some() {
            cx.notify();
        }
        if lead {
            if let Some(message) = end_refusal(&ended.end) {
                self.report_flow_end_refusal(message, worktree, &ended.end, cx);
            }
            // Whichever lane is on screen: the Flows list shows only
            // "canceled", which reads like a Stop by hand.
            if let (RunEnd::Canceled { .. }, Some(limit)) = (&ended.end, &ended.overage) {
                self.toast_flow_overage_stop(limit, cx);
            }
        }
        if report_to_open(&ended.end, &ended.run_dir).is_none() {
            return;
        }
        // The report waits in the Flows page's past runs; nothing opens over
        // what the person is looking at. A worktree off screen is marked
        // unread, the way an agent's finished turn marks it; the one on
        // screen, which no mark can reach, gets a toast instead — and so does
        // a lead holding no lane there, which no mark can reach either.
        match lane {
            Some(lane) if lane != self.active => self.mark_lane_unread(lane, cx),
            Some(lane) if lead && !self.shows_flows_of(lane) => {
                self.toast_flow_run_end(&ended.end, cx);
            }
            None if lead => self.toast_flow_run_end(&ended.end, cx),
            Some(_) | None => {}
        }
    }

    /// Say why a run ended without telling its own story. The wording stays
    /// short — the phone is told the same sentence — so where to look goes in
    /// the detail.
    fn report_flow_end_refusal(
        &mut self,
        message: String,
        worktree: &Path,
        end: &RunEnd,
        cx: &mut Context<Self>,
    ) {
        let detail = match end {
            RunEnd::LockHeld { .. } => super::flow_paths::lane_lock_dir(&self.lock_root, worktree)
                .map(|dir| s::flow::lock_held_detail(dir.join(".lock").display().to_string())),
            _ => None,
        };
        self.report_error(
            ErrorReport::new(message)
                .severity(ErrorSeverity::Warning)
                .at(file!(), line!())
                .with_context("detail", detail.unwrap_or_default())
                .dedup("flow.run.ended")
                .build(),
            cx,
        );
    }

    /// Say a run was stopped for extra usage. A limit is account-wide, so
    /// runs trip together: one key of its own merges them into one toast
    /// with a repeat count, never into a "finished" toast.
    fn toast_flow_overage_stop(
        &mut self,
        limit: &daruda_acp::RateLimitView,
        cx: &mut Context<Self>,
    ) {
        self.report_error(
            ErrorReport::new(overage_stopped(limit))
                .severity(ErrorSeverity::Warning)
                .at(file!(), line!())
                .dedup("flow.run.overage_stopped")
                .build(),
            cx,
        );
    }

    /// Whether the Flows page is showing `lane`'s runs — its list or graph
    /// already says how a run there ended.
    pub(super) fn shows_flows_of(&self, lane: LaneRef) -> bool {
        self.active_page() == Some(super::pages::Page::Flows)
            && match &self.pages.flows.detail {
                Some(detail) => detail.lane == lane,
                None => self.flow_browser_lane() == lane,
            }
    }

    /// Say a run ended, when it finished or failed on its own.
    fn toast_flow_run_end(&mut self, end: &RunEnd, cx: &mut Context<Self>) {
        let (message, severity) = match end {
            RunEnd::Done => (s::flow::run_finished_toast(), ErrorSeverity::Info),
            RunEnd::Failed { .. } => (s::flow::run_failed_toast(), ErrorSeverity::Warning),
            // Stopped by hand, toasted above as an overage stop, or already
            // toasted as a refusal.
            _ => return,
        };
        self.report_error(
            ErrorReport::new(message)
                .severity(severity)
                .at(file!(), line!())
                .dedup("flow.run.ended_here")
                .build(),
            cx,
        );
    }

    /// The worktree `lane` names, for a test driving its run.
    #[cfg(test)]
    fn worktree_for_test(&self, lane: LaneRef) -> PathBuf {
        self.lane_for(lane)
            .expect("a lane of this window")
            .path
            .clone()
    }

    /// Drive one stream event through the run's real path — `RunEnded`
    /// included. What it does to the windows lands when this update ends.
    #[cfg(test)]
    pub(in crate::workspace) fn apply_flow_event_for_test(
        &mut self,
        lane: LaneRef,
        event: &daruda_flow::event::FlowEvent,
        cx: &mut Context<Self>,
    ) {
        let worktree = self.worktree_for_test(lane);
        RunsHost::apply_flow_event_for_test(&worktree, event, cx);
    }

    /// Park a question on a seeded run, so a test can exercise answering
    /// without an adapter. A dialog it raises opens when this update ends.
    #[cfg(test)]
    pub(in crate::workspace) fn park_flow_ask_for_test(
        &mut self,
        lane: LaneRef,
        run_dir: &Path,
        pending: daruda_flow::runner::PendingAsk,
        cx: &mut Context<Self>,
    ) {
        let worktree = self.worktree_for_test(lane);
        RunsHost::park_flow_ask(&worktree, run_dir, pending, cx);
    }

    /// Stop `lane`'s run for `limit` as its engine would.
    #[cfg(test)]
    pub(in crate::workspace) fn trip_flow_overage_for_test(
        &mut self,
        lane: LaneRef,
        limit: &daruda_acp::RateLimitView,
        cx: &mut Context<Self>,
    ) {
        let worktree = self.worktree_for_test(lane);
        RunsHost::trip_flow_overage_for_test(&worktree, limit, cx);
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

/// What the phone is told about a finished run.
///
/// `refusal` is [`end_refusal`]'s wording, reused rather than re-translated so
/// the phone and the desktop say the same thing about the same outcome. The
/// three outcomes it has no wording for are the ones a person reads the run
/// report for, so those get a one-line verdict and a pointer to it.
fn flow_outcome_notice(
    end: &RunEnd,
    refusal: Option<String>,
    overage: Option<&daruda_acp::RateLimitView>,
) -> String {
    if let Some(message) = refusal {
        return s::control::flow_refused_notice(clamp_notice_reason(&message));
    }
    // Spelled out rather than defaulted: which ends `end_refusal` answers for
    // is its business, not this function's, so a variant added there without a
    // wording here must be a compile error and not a confident "failed".
    match end {
        RunEnd::Done => s::control::flow_finished_notice(),
        RunEnd::Canceled { .. } => {
            overage.map_or_else(s::control::flow_canceled_notice, overage_stopped)
        }
        RunEnd::Failed { .. } => s::control::flow_failed_notice(),
        // Every one of these produced a refusal above, so they are unreachable
        // here — named anyway so the compiler, not a reader, checks that.
        RunEnd::BudgetExhausted { .. }
        | RunEnd::Io { .. }
        | RunEnd::Log { .. }
        | RunEnd::NotResumed { .. }
        | RunEnd::LockHeld { .. }
        | RunEnd::Invalid { .. }
        | RunEnd::Unprovisioned { .. }
        | RunEnd::Stalled { .. }
        | RunEnd::ShutDown => s::control::flow_failed_notice(),
    }
}

/// Why a run was canceled for extra usage, with the reset moment when the
/// report carried one. One wording for the toast and the phone.
fn overage_stopped(limit: &daruda_acp::RateLimitView) -> String {
    match limit
        .resets_at
        .and_then(|secs| chrono::DateTime::from_timestamp(secs, 0))
    {
        Some(at) => s::flow::overage_stopped_resets(s::flow::overage_reset_at(at)),
        None => s::flow::overage_stopped(),
    }
}

/// Cap the engine's own words before they go into a phone message.
///
/// `end_refusal` can hand back a whole multi-line validation report, and the
/// notice goes out as one message against Telegram's 4096-character limit —
/// where an over-long body is rejected outright and the failure is only
/// logged, so the outcome would be lost exactly when it matters most.
fn clamp_notice_reason(reason: &str) -> String {
    /// Enough for a sentence or two of engine detail; the full report is on
    /// disk and the notice already points at daruda for it.
    const MAX_CHARS: usize = 300;
    reason.chars().take(MAX_CHARS).collect()
}

/// What to say about a run that has ended, or `None` when it ended the way
/// it was meant to and `run.md` is the whole story.
fn end_refusal(end: &RunEnd) -> Option<String> {
    match end {
        RunEnd::Done | RunEnd::Failed { .. } | RunEnd::Canceled { .. } => None,
        RunEnd::BudgetExhausted { limit } => Some(s::flow::budget_exhausted(*limit)),
        RunEnd::Io { message, .. } => Some(message.clone()),
        RunEnd::Log { message, .. } => Some(message.clone()),
        RunEnd::NotResumed { message } => Some(message.clone()),
        RunEnd::LockHeld { holder } => Some(s::flow::lock_held(holder.pid)),
        RunEnd::Invalid { issues } => Some(issue_report(issues)),
        RunEnd::Unprovisioned { agent, message } => Some(s::flow::unprovisioned(agent, message)),
        RunEnd::Stalled { nodes } => Some(s::flow::stalled(node_list(nodes))),
        RunEnd::ShutDown => Some(s::flow::shut_down()),
    }
}

/// The held nodes as one phrase. Backticked like the engine's own wording
/// so a reader sees the same names in `run.md` and in the toast.
fn node_list(nodes: &[daruda_flow::NodeId]) -> String {
    nodes
        .iter()
        .map(|node| format!("`{node}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every outcome a phone can be told about, including that a refusal
    /// carries the engine's *own* wording through rather than a second
    /// translation of it.
    #[test]
    fn every_outcome_has_its_own_notice() {
        let done = flow_outcome_notice(&RunEnd::Done, None, None);
        let canceled = flow_outcome_notice(&RunEnd::Canceled { node: None }, None, None);
        assert_eq!(done, s::control::flow_finished_notice());
        assert_eq!(canceled, s::control::flow_canceled_notice());
        assert_ne!(done, canceled, "finished must not read as stopped");

        let holder = daruda_flow::lock::LockHolder {
            pid: 4321,
            run_id: "01J".to_string(),
            started_unix_secs: 0,
        };
        let end = RunEnd::LockHeld { holder };
        let refusal = end_refusal(&end).expect("a held lock is a refusal");
        let notice = flow_outcome_notice(&end, Some(refusal.clone()), None);
        assert!(
            notice.contains(&refusal),
            "the desktop's wording must reach the phone verbatim: {notice}"
        );
        assert!(notice.contains("4321"), "the detail survives: {notice}");
    }

    fn billed(resets_at: Option<i64>) -> daruda_acp::RateLimitView {
        daruda_acp::RateLimitView {
            window: Some(daruda_acp::RateLimitWindow::FiveHour),
            resets_at,
            is_using_overage: true,
        }
    }

    /// A run the overage switch stopped is told apart from one a person
    /// stopped, with the reset moment when the report carried one.
    #[test]
    fn a_run_stopped_for_extra_usage_says_so() {
        let canceled = RunEnd::Canceled { node: None };
        assert_eq!(
            flow_outcome_notice(&canceled, None, None),
            s::control::flow_canceled_notice()
        );
        assert_eq!(
            flow_outcome_notice(&canceled, None, Some(&billed(None))),
            s::flow::overage_stopped()
        );
        let at = chrono::DateTime::from_timestamp(1_791_340_200, 0).expect("valid");
        assert_eq!(
            flow_outcome_notice(&canceled, None, Some(&billed(Some(1_791_340_200)))),
            s::flow::overage_stopped_resets(s::flow::overage_reset_at(at))
        );
        // Only a cancel is the switch's to explain.
        assert_eq!(
            flow_outcome_notice(&RunEnd::Done, None, Some(&billed(None))),
            s::control::flow_finished_notice()
        );
    }

    /// `end_refusal` can hand back a whole validation report, and the notice
    /// goes out as one message against Telegram's limit — where an over-long
    /// body is rejected outright and only logged, losing the outcome exactly
    /// when it matters.
    #[test]
    fn a_long_refusal_is_bounded_before_it_is_interpolated() {
        let huge = "x".repeat(10_000);
        let notice = flow_outcome_notice(&RunEnd::Done, Some(huge), None);
        assert!(
            notice.chars().count() < 500,
            "{} chars",
            notice.chars().count()
        );
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
    /// story is `run.md`, in the page's past runs either way. Only a run that
    /// never got to tell one needs a message.
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
