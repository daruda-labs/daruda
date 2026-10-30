//! Connection, cancellation acknowledgements and terminal prompt/session events.

use super::super::{ActivitySpan, AgentChatView, AgentSessionStatus, Replay, TurnOutcome};
use super::{EventChanges, PhoneTurnAction};
use crate::strings as s;
use daruda_acp::ChatItem;
use daruda_store::project::PaneCwd;
use gpui::Context;

impl AgentChatView {
    pub(super) fn finish_connected(
        &mut self,
        session_id: String,
        changes: &mut EventChanges,
        cx: &mut Context<Self>,
    ) {
        // A real resume (`session/load`) returns the same id we asked to
        // load; a resume the agent couldn't load was downgraded to a fresh
        // `session/new` with a NEW id. The gate carries the id it asked
        // for, so it can tell those apart — otherwise a downgraded resume
        // keeps the prior session's title and skips fresh-session setup.
        // Compare before overwriting `session_id`.
        let resumed = self.replay.resumed(&session_id);
        // Whether this reply is what closed a replay gate — true for a
        // real resume and for a load the agent downgraded alike, since
        // both keep whatever the replay already put in `items`.
        changes.finished_restore = self.replay.is_loading();
        self.replay = Replay::Live;
        // A replay closes with this reply, not a `TurnEnded`, so its tail
        // is still flagged live and nothing more is coming for it.
        if changes.finished_restore {
            self.settle_run_state(cx);
            self.settle_prompts();
        }
        // Record the live session id so it persists — and so a later
        // launch resumes this session instead of starting fresh.
        self.session_id = Some(session_id);
        if resumed {
            // Resume (`session/load`): the replayed `session/update`s
            // already populated the conversation, plan, and title before
            // this reply — keep them. The tail runs the single
            // coalesced catch-up off `changes.finished_restore`.
        } else {
            // Fresh session (`session/new`): clear stale plan/title so
            // they don't flash before the new agent sends its first
            // updates, and drop the prior session's subagent activity so
            // its timestamps can't hold the new session's badge "busy".
            self.plan.clear();
            self.session_title = None;
            self.session_updated_at = None;
            self.plan_collapsed = false;
            self.session_usage = None;
            // The markers stay: they index `items`, which a fresh
            // session keeps.
            self.overage_notice = None;
            self.activity.subagent_last_activity.clear();
            // Reset the activity-span tracker so a prior session's edge
            // state / captured outcome can't leak into the fresh one.
            self.activity.span = ActivitySpan::Idle;
            self.activity.pending_completion = None;
            self.queue.turn.acknowledge_cancel();
            // A fresh session gets its own chance to report a dropped-
            // output mismatch instead of inheriting the prior session's
            // silence.
            self.warned_dropped_terminal_output = false;
        }
        self.pump_pending_prompt(cx);
    }
    pub(super) fn apply_cancel_ack(&mut self, changes: &mut EventChanges, cx: &mut Context<Self>) {
        // The terminal signal (a `cancelled` `TurnEnded`, or a
        // `TurnFailed` if the prompt errored as the cancel raced it) for a
        // turn a Stop already settled locally — its `Stopped` fired at
        // cancel time. Don't re-complete or push an error item; do settle
        // what landed inside the window, then close it and drain any
        // re-prompt the user buffered while it was open (as a fresh turn). A buffered
        // re-prompt was never put on the wire (see `send_prompt_text`'s
        // `cancel_in_flight` guard), so nothing raced this ack and a
        // second Stop could still have cleared it.
        self.queue.turn.acknowledge_cancel();
        // No further chunk for this turn can arrive. Settle what landed
        // inside the cancel window first: Stop's own settle ran before
        // those chunks existed, and each one started a *fresh* streaming
        // block (the previous was already finalized), so without this the
        // stopped run keeps reading `Rollup::Running`. Then the marker
        // Stop pushed can take its final position.
        self.settle_run_state(cx);
        self.settle_stop_marker();
        // Two effects the tail owes that settle: revisit the calls it
        // just made terminal, and remeasure the rows whose streaming
        // height it just fixed.
        self.mark_turn_settled(changes);
        changes.phone_turn_action = PhoneTurnAction::Clear;
        self.pump_pending_prompt(cx);
    }
    /// The turn is over: the event tail remeasures every row for it, which is
    /// what lets the prompts that ran fold now rather than mid-turn. Called
    /// before any queued prompt is pumped, so the next prompt stays open.
    fn mark_turn_settled(&mut self, changes: &mut EventChanges) {
        changes.turn_settled = true;
        self.settle_prompts();
    }

    pub(super) fn apply_turn_ended(
        &mut self,
        stop_reason: &str,
        completed_normally: bool,
        usage: Option<daruda_acp::model::TurnUsageView>,
        changes: &mut EventChanges,
        cx: &mut Context<Self>,
    ) {
        // Filed here, off the turn's own clock — not at the pane's
        // busy→idle settle edge. The settle edge is right for the
        // *completion* side effects (Pitfall 11: it trails `end_turn`
        // while background subagents finish), but it is the wrong ruler
        // for "how long did this answer take": a queued batch drains
        // without ever going idle between turns, so every turn but the
        // last would get no record and the last would report the whole
        // batch. `Turn::InFlight` carries the per-turn start, which is
        // exactly the span being reported.
        self.record_turn(usage.map(|u| u.output_tokens));
        // The wire calls a cut-short answer a completion; the transcript says
        // otherwise, or a truncated reply reads exactly like a finished one.
        // It settles as an error too, as a flow node does: a task must not be
        // marked done, nor an asking agent handed half an answer as the reply.
        let early = match daruda_acp::TurnStop::from_reported(stop_reason) {
            daruda_acp::TurnStop::Early(stop) => Some(stop),
            _ => None,
        };
        if let Some(stop) = early {
            self.items
                .push(ChatItem::Failure(daruda_acp::AcpFailure::Stopped(stop)));
        }
        // Settle the turn: finalize streaming, cancel any tool the agent
        // left non-terminal (e.g. a `Cancelled` stop reason), and drain a
        // still-pending permission so no card keeps live buttons.
        self.settle_turn(cx);
        self.mark_turn_settled(changes);
        changes.phone_turn_action = PhoneTurnAction::Finish;
        // Capture the outcome; it fires only when the pane settles
        // busy→idle (via `reconcile_activity`), which may trail this
        // `end_turn` while trailing subagents finish.
        self.activity.pending_completion = Some(match (completed_normally, early) {
            (false, _) => TurnOutcome::Stopped,
            (true, Some(_)) => TurnOutcome::Errored,
            (true, None) => TurnOutcome::Completed,
        });
        // Drain the next buffered prompt (if any) now that the turn
        // completed — one per `TurnEnded`, so the queue advances a single
        // turn at a time and `turn.is_in_flight()` keeps tracking exactly
        // one live turn. A no-op when nothing is buffered or on a cancelled
        // turn whose queue is empty. Pumping here (not inside
        // `settle_turn`) is deliberate: `settle_turn` is also the Stop /
        // `Error` teardown, and pumping from all three would double-drain
        // (a cancelled turn's later idempotent `TurnEnded` re-runs
        // settle) — only a natural completion should advance the queue.
        self.pump_pending_prompt(cx);
        // Auto-collapse the plan region so the completed checklist
        // recedes to a one-line summary. The next `PlanChanged` will
        // re-expand it (see below).
        if !self.plan.is_empty() {
            self.plan_collapsed = true;
        }
    }
    pub(super) fn apply_turn_failed(
        &mut self,
        failure: daruda_acp::AcpFailure,
        changes: &mut EventChanges,
        cx: &mut Context<Self>,
    ) {
        // A single `session/prompt` failed (e.g. the adapter hit a usage
        // / session limit → `-32603`), but the ACP connection is alive —
        // the error was a normal JSON-RPC response, not a transport
        // failure. So unlike the terminal `Error` arm below, DO NOT set
        // `status = Error` and DO NOT drop the handle: keep the session
        // Connected and usable so the user can re-prompt (e.g. once the
        // limit resets) without reconnecting.
        //
        // Surface the failure inline in the conversation, then settle the
        // turn exactly as a `TurnEnded` would — otherwise a streaming
        // block stays `streaming: true` and an `InProgress` tool stays
        // live, so the rollup glyph blinks forever and the footer reads
        // `Running` after the turn is already over.
        self.items.push(ChatItem::Failure(failure));
        // (A `TurnFailed` while `cancel_in_flight` is handled by the
        // guarded arm above; here the turn was not being cancelled.)
        self.settle_turn(cx);
        self.mark_turn_settled(changes);
        changes.phone_turn_action = PhoneTurnAction::Finish;
        // Capture the errored outcome; it fires (notification +
        // backing-task done) on the busy→idle settle edge that
        // `reconcile_activity` detects, same as a normal completion.
        self.activity.pending_completion = Some(TurnOutcome::Errored);
        // Advance the prompt queue one turn like a natural completion, so
        // a prompt the user buffered while this turn ran still runs. No-op
        // when nothing is buffered (the common single-prompt case).
        self.pump_pending_prompt(cx);
    }
    pub(super) fn apply_session_error(
        &mut self,
        failure: daruda_acp::AcpFailure,
        changes: &mut EventChanges,
        cx: &mut Context<Self>,
    ) {
        // Read before `set_error` replaces it: what the queue is told
        // depends on whether this session ever connected.
        let drop_reason = if self.status == AgentSessionStatus::Connected {
            super::super::PromptDropReason::SessionFailed
        } else {
            super::super::PromptDropReason::ConnectFailed
        };
        let message = super::super::super::agent_chat_helpers::failure_message(&failure);
        let error_message = match &self.cwd {
            Some(PaneCwd::Remote(_)) => {
                format!(
                    "{}\n\n{}",
                    message,
                    s::agent_chat::remote_connect_error_hint()
                )
            }
            _ => message,
        };
        self.set_error(error_message, failure.remedy(), cx);
        // A session-level error terminates every outstanding turn,
        // including any cancel we were still awaiting an ack for — close
        // the cancel window so a post-reconnect turn isn't misread.
        self.queue.turn.acknowledge_cancel();
        // A load that fails mid-replay must still render whatever was
        // replayed — release the coalescing gate so the tail rebuilds.
        self.replay = Replay::Live;
        // A mid-turn failure must settle the turn like a Stop would —
        // otherwise a streaming block stays `streaming: true` and an
        // `InProgress` tool stays live, so the rollup glyph blinks
        // forever and the response bar reads `Running` after the session
        // is already dead.
        self.settle_turn(cx);
        self.mark_turn_settled(changes);
        changes.phone_turn_action = PhoneTurnAction::Finish;
        // Capture the failure outcome; it fires on the busy→idle settle
        // edge (via `reconcile_activity`), same as a normal completion.
        self.activity.pending_completion = Some(TurnOutcome::Errored);
        // The session is dead with no reconnect path, so any buffered
        // prompts can never be delivered — drop both the live queue and
        // a parked queue (a Stop may have parked one before this Error)
        // rather than leaving them to be pumped or shown with a Resume
        // button that can't send (they were never echoed, so nothing
        // dangles in the transcript).
        let dropped = self.queue.discard_all();
        self.report_dropped(dropped, drop_reason, cx);
        // Drop the now-dead handle. The connection task has ended (this
        // `Error` is its terminal signal), so its command channel is
        // closed — a lingering `Some(handle)` would let `send_prompt_text`
        // send into a dead channel (silently dropped) and mark a turn
        // in-flight that never ends, stranding the pane on a phantom
        // "Working". With `None`, a post-error prompt buffers instead of
        // stranding. (Distinct from `TurnFailed`, which keeps the handle:
        // there the connection is still alive.)
        self.detach_handle();
    }
}

#[cfg(test)]
mod tests;
