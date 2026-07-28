//! Turn cancel/settle, permission responses, fold/plan/mode toggles, and
//! session teardown/reset for one Agent chat pane. The Stop path and the
//! `/clear`-style resets live here since they all end in the same shared
//! settle/teardown core.

use daruda_acp::{
    PermissionDecision, PermissionKindView, cancel_pending_tools, finalize_streaming,
};
use gpui::Context;

use super::super::agent_chat_helpers::{
    cancel_pending_permission, collect_foldable_keys, fold_active, fold_key_item_index,
    permission_card_mut,
};
use super::super::fold::{FoldKey, FoldState};
use super::super::rows::RowKind;
use super::super::session_config::SessionConfig;
use super::{AgentChatView, AgentSessionStatus, Turn, TurnOutcome};

impl AgentChatView {
    /// Stop the active turn. Sends `session/cancel` *and* ends the turn locally,
    /// right now — it does not wait for the agent's stop reason.
    ///
    /// `session/cancel` is only cooperative: a hung or dead agent may never
    /// return a `Cancelled` stop reason, and `AcpEvent::TurnEnded` (the sole
    /// other place the in-flight turn clears) would then never arrive — leaving
    /// the turn pulsing forever with the input stuck on "Stop". So Stop is
    /// authoritative here: clear the in-flight state, settle streaming text and
    /// still-running tool calls, and drain any pending permission. A later
    /// `TurnEnded` for this turn is idempotent. Mirrors zed's
    /// `AcpThread::cancel`, which takes `running_turn` and marks pending tools
    /// cancelled without awaiting the agent.
    pub(in crate::workspace) fn cancel_turn(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = &self.handle {
            handle.cancel();
        }
        // Settle the turn *locally and immediately* — responsive and hung-safe: a
        // connected agent that never returns the `cancelled` `TurnEnded` (or
        // returns it much later) can't leave the pane stuck busy. When a live
        // foreground turn is being cancelled, stash its `Stopped` (fired at the
        // busy→idle edge by `cancel_agent_turn`'s reconcile) and open the cancel
        // window (`cancel_in_flight`): until the cancel is acked, a re-prompt
        // buffers *client-side* (see `send_prompt_text`) rather than racing onto
        // the wire, so it can't be misattributed to the cancelled turn's ack and
        // a second Stop can still clear it. A trailing-subagent Stop (turn already
        // idle) neither stashes nor opens the window: the foreground turn's
        // already-captured outcome is preserved and fires when the tools settle.
        if self.queue.turn.is_in_flight() {
            self.activity.pending_completion = Some(TurnOutcome::Stopped);
            self.activity.cancel_in_flight = true;
        }
        self.settle_turn();
        self.clear_telegram_first_response_watch();
        // Stop cancels the running turn but PRESERVES the queue: move everything
        // buffered before this Stop into the parked queue rather than dropping
        // it. Parked prompts do NOT auto-drain — they live outside
        // `pending_prompts`, so the cancel-ack / `TurnEnded` pump is a no-op on
        // them. The user resumes them explicitly (the queue strip's Resume
        // button → `resume_queue`) or discards them (a second Esc / clear-all).
        // A prompt typed *after* Stop is pushed onto the now-empty live queue and
        // (idle + connected) sends immediately as a fresh turn, ahead of the
        // parked queue. `append` preserves FIFO order and stacks after any items
        // parked by a prior Stop.
        self.queue
            .paused_prompts
            .append(&mut self.queue.pending_prompts);
        self.queue.editing_prompt = None;
        // `settle_turn` mutated items (streaming → done, running tools →
        // cancelled, pending card → resolved), changing fold visibility and row
        // heights, so reproject and remeasure before notifying.
        self.rebuild_rows();
        self.list_state.remeasure();
        cx.notify();
    }

    /// End the current turn *locally* and settle every still-live item: clear
    /// the in-flight flags, finalize streaming text, mark running tool calls
    /// cancelled, and drain any pending permission. Model-only (no rows /
    /// notify) so the three call paths — the Stop button (`cancel_turn`), a
    /// normal `TurnEnded`, and a terminal `Error` — share one settle sequence
    /// and can never drift (e.g. one leaving streaming/tools live so the rollup
    /// blinks forever). Idempotent: a later `TurnEnded` after a Stop is a no-op.
    pub(super) fn settle_turn(&mut self) {
        self.queue.turn = Turn::Idle;
        self.settle_items();
        // Everything the turn produced is delivered by the completion relay, so
        // reset the post-turn baseline to the current assistant-text count; only
        // messages that arrive *after* this settle count as a follow-up.
        self.snap_post_turn_baseline();
    }

    /// Settle every still-live *item* — finalize streaming text, mark running
    /// tool calls cancelled, drain any pending permission — without touching the
    /// prompt `turn`. [`Self::settle_turn`] is this plus `turn = Idle`;
    /// [`Self::cancel_turn`] calls this alone so the turn stays in-flight until
    /// its real (`cancelled`) `TurnEnded` drives the settle + queue drain.
    fn settle_items(&mut self) {
        finalize_streaming(&mut self.items);
        cancel_pending_tools(&mut self.items);
        cancel_pending_permission(self);
    }

    /// Resolve the permission request `request_id` with the chosen option.
    /// Marks *that* card resolved (found by id, not by position — several may be
    /// outstanding), sends the decision over the session, and drops the id from
    /// the outstanding index. No-op if the request was already answered or
    /// cancelled. `kind` selects Allow vs. Reject semantics.
    pub(in crate::workspace) fn respond_permission(
        &mut self,
        request_id: u64,
        option_id: String,
        kind: PermissionKindView,
        cx: &mut Context<Self>,
    ) {
        if !self.pending_permissions.remove(&request_id) {
            return;
        }
        // Mark the card that carries this request id resolved so its buttons
        // disable and the choice shows — the others stay live.
        if let Some(card) = permission_card_mut(self, request_id) {
            card.resolved = Some(daruda_acp::PermissionResolution::Chosen(option_id.clone()));
        }
        let decision = match kind {
            PermissionKindView::AllowOnce | PermissionKindView::AllowAlways => {
                PermissionDecision::Allow { option_id }
            }
            PermissionKindView::RejectOnce | PermissionKindView::RejectAlways => {
                PermissionDecision::Reject { option_id }
            }
        };
        if let Some(handle) = &self.handle {
            handle.respond_permission(request_id, decision);
        }
        // The card is now resolved, so it no longer force-stays-visible under a
        // collapsed response: reproject so it folds back into the process
        // immediately. It also renders shorter (buttons → outcome line), so
        // remeasure for the reflow (item count unchanged).
        self.rebuild_rows();
        self.list_state.remeasure();
        cx.notify();
    }

    /// Toggle the fold state of one block. Resolves the `active` flag the same
    /// way `render` derives it, so the first click flips the *visible* state
    /// rather than re-deriving from a stale default.
    pub(in crate::workspace) fn toggle_fold(&mut self, key: FoldKey, cx: &mut Context<Self>) {
        // Resolve `active` via the shared `fold_active` — the same source
        // `rows::project` uses to derive the default collapsed state — so the
        // first click flips the *visible* state rather than a stale re-derivation.
        let active = fold_active(&key, &self.items);
        // Resolve before the key moves into `fold.toggle` below: a nested fold
        // (a tool card's own body, one of its diffs, its raw-input disclosure,
        // or a nested subagent card) only changes its *owning row's* rendered
        // height in place — no `RenderRow::hidden` flip anywhere — so
        // `rebuild_rows`'s hidden-range diff can't see it and falls back to
        // remeasuring the tail, leaving that row's cached height stale (content
        // clips or overlaps the next row). Remeasure it explicitly whenever we
        // can resolve which row owns the toggled key.
        let item_ix = fold_key_item_index(&key, &self.items);
        self.fold.toggle(key, active);
        // A fold change flips row `hidden` flags (and may collapse a group):
        // reproject + reflow the affected span.
        self.rebuild_rows();
        if let Some(item_ix) = item_ix
            && let Some(row_ix) = self
                .rows
                .iter()
                .position(|r| matches!(r.kind, RowKind::AgentItem(ix) if ix == item_ix))
        {
            self.list_state.remeasure_items(row_ix..row_ix + 1);
        }
        cx.notify();
    }

    /// Expand or collapse every currently-visible foldable block at once (the
    /// fold toolbar's expand-all / collapse-all).
    pub(in crate::workspace) fn set_all_folds(&mut self, expanded: bool, cx: &mut Context<Self>) {
        let keys = collect_foldable_keys(&self.items);
        self.fold.set_all(keys, expanded);
        // Bulk expand/collapse flips many row `hidden` flags: reproject + reflow.
        self.rebuild_rows();
        // Unlike a single `toggle_fold`, this can change dozens of rows' inner
        // content height at once (every tool card's body, every diff, every
        // raw-input disclosure) with no per-row hidden flip to key a targeted
        // remeasure off of — a full remeasure is the correct (if broader) fix
        // here, same as `respond_permission`'s reflow.
        self.list_state.remeasure();
        cx.notify();
    }

    /// Collapse / expand the bottom plan region. The plan is a derived render
    /// of `plan` (full-replaced by the agent), so this only flips the local
    /// presentation flag and notifies — no model rebuild needed.
    pub(in crate::workspace) fn toggle_plan_collapsed(&mut self, cx: &mut Context<Self>) {
        self.plan_collapsed = !self.plan_collapsed;
        cx.notify();
    }

    /// Dismiss the plan region: drop the entries so `plan_region` renders
    /// nothing. The plan is a derived render of `plan` (full-replaced by the
    /// agent), so clearing it is a pure local presentation reset — a later
    /// turn's `PlanChanged` repopulates it (expanded, since `plan_collapsed` is
    /// reset here). Wired to the header's × button, shown only once every entry
    /// is completed, so the finished checklist no longer lingers as a collapsed
    /// header with no clear close point.
    pub(in crate::workspace) fn dismiss_plan(&mut self, cx: &mut Context<Self>) {
        self.plan.clear();
        self.plan_collapsed = false;
        cx.notify();
    }

    /// Switch the active session mode: show the pick immediately, then ask the
    /// agent over the live handle (no-op when the handle is absent). A
    /// `ModeChanged` replaces the whole state if the agent disagrees.
    pub(in crate::workspace) fn set_mode(&mut self, mode_id: String, cx: &mut Context<Self>) {
        self.session_config
            .set_current_mode_optimistically(mode_id.clone());
        self.last_known_mode_id = Some(mode_id.clone());
        if let Some(h) = &self.handle {
            h.set_mode(mode_id);
        }
        cx.notify();
    }

    /// Change a select config option (model / effort / …): show the pick
    /// immediately, then ask the agent over the live handle (no-op when the
    /// handle is absent). The reply replaces the whole option set via a
    /// `ConfigOptionsChanged` event.
    pub(in crate::workspace) fn set_config_option(
        &mut self,
        config_id: String,
        value: String,
        cx: &mut Context<Self>,
    ) {
        self.session_config
            .set_option_value_optimistically(&config_id, value.clone());
        if let Some(h) = &self.handle {
            h.set_config_option(config_id, value);
        }
        cx.notify();
    }

    /// Shared teardown for a full `/clear` reset and a post-`Error` retry:
    /// drop the live handle and event pump (closing the ACP command channel
    /// and ending the pump loop — the same teardown a pane close performs)
    /// and wipe the conversation model + every runtime cache. Does NOT touch
    /// `session_id`, `restoring`, or `status` — the two callers differ there:
    /// a `/clear` reset drops the session id for a brand-new conversation; a
    /// retry keeps it so the reconnect resumes the same one via
    /// `session/load` instead of losing history.
    fn teardown_transient_session_state(&mut self) {
        cancel_pending_permission(self);
        self.handle = None;
        self._event_pump = None;
        self.items.clear();
        self.queue.pending_prompts.clear();
        self.queue.paused_prompts.clear();
        self.queue.editing_prompt = None;
        self.pending_permissions.clear();
        self.telegram_first_response_watch = None;
        self.queue.turn = Turn::Idle;
        self.activity.subagent_last_activity.clear();
        self.activity.activity_started_at = None;
        self.activity.was_busy = false;
        self.activity.pending_completion = None;
        self.activity.cancel_in_flight = false;
        self.session_usage = None;
        self.assets.clear();
        self.fold = FoldState::default();
        self.session_config = SessionConfig::default();
        self.plan.clear();
        self.plan_collapsed = false;
        self.session_title = None;
        self.session_updated_at = None;
    }

    /// Full local reset for the `/clear` slash command: wipe the conversation
    /// model and every runtime cache via [`Self::teardown_transient_session_state`],
    /// then also drop the persisted session id. The caller
    /// (`Workspace::reset_agent_chat_session`) then calls `connect_agent_chat`
    /// to supersede this with a fresh `session/new`, so the view never sits
    /// handle-less for a render.
    pub(in crate::workspace) fn reset_for_new_session(&mut self, cx: &mut Context<Self>) {
        self.teardown_transient_session_state();
        // Clear the persisted id so a restart resumes the fresh session, not
        // the cleared conversation (Connected re-persists the new id).
        self.session_id = None;
        self.restoring = false;
        // `teardown_transient_session_state` already cleared `items`, so this
        // resets the baseline to 0 — a stray post-turn update queued before
        // teardown can't relay stale text into the fresh session.
        self.snap_post_turn_baseline();
        self.status = AgentSessionStatus::Connecting;
        self.rebuild_rows(); // diff-splices list_state down to 0 rows
        cx.notify();
    }

    /// Reconnect after a terminal `Error` without losing the conversation:
    /// same teardown as `reset_for_new_session` (clears local `items` so a
    /// resume's replay doesn't duplicate what was already rendered before the
    /// failure) but keeps `session_id`, so `connect_agent_chat` resumes via
    /// `session/load` and the adapter replays the exact same history back.
    /// Called by `Workspace::retry_agent_chat_connect`, which then
    /// re-invokes `connect_agent_chat` with `self.session_id` as the resume.
    pub(in crate::workspace) fn retry_for_reconnect(&mut self, cx: &mut Context<Self>) {
        self.teardown_transient_session_state();
        self.restoring = self.session_id.is_some();
        self.status = AgentSessionStatus::Connecting;
        self.rebuild_rows();
        cx.notify();
    }

    /// Jump the conversation list to the bottom and re-engage `Tail` follow.
    /// Backs the floating scroll-to-bottom button. The list internally re-arms
    /// tail-following once it lands at the end, so a fresh streaming chunk keeps
    /// sticking to the bottom.
    pub(in crate::workspace) fn scroll_to_bottom(&mut self, cx: &mut Context<Self>) {
        self.list_state.scroll_to_end();
        cx.notify();
    }
}
