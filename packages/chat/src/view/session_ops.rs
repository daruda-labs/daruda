//! Turn cancel/settle, permission responses, mode/config options, and
//! session teardown/reset for one Agent chat pane. The Stop path and the
//! `/clear`-style resets live here since they all end in the same shared
//! settle/teardown core.

use daruda_acp::{
    ChatItem, PermissionDecision, PermissionKindView, cancel_pending_plan_entries,
    cancel_pending_tools, finalize_streaming,
};
use gpui::Context;

use super::super::agent_chat_helpers::{cancel_pending_permission, permission_card_mut};
use super::super::reconcile::ReconcileScope;
use super::super::session_config::SessionConfig;
use super::super::session_config::model_select;
use super::{ActivitySpan, AgentChatView, Turn, TurnOutcome};

impl AgentChatView {
    /// Stop the active turn: send `session/cancel` *and* end the turn locally
    /// right now, without waiting for the agent's stop reason. `cancel` is
    /// only cooperative — a hung/dead agent may never send the `Cancelled`
    /// `TurnEnded`, leaving the turn pulsing forever otherwise. A later
    /// `TurnEnded` for this turn is idempotent.
    pub fn cancel_turn(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = self.live_handle() {
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
        if self.queue.turn.cancel() {
            self.activity.pending_completion = Some(TurnOutcome::Stopped);
            // Mark where the run was cut — the adapter's live path drops the
            // SDK's own interrupt entry while its replay lets it through, so
            // without this push a stop is invisible until restore. See
            // `daruda_acp::ChatItem::Interrupted`.
            //
            // Gated on a live turn to match when the SDK actually records one:
            // every writer is reached only while a request is being processed —
            // four inside the query generator (abort, query error, pre-stop
            // hook) plus the slash-command expansion's `AbortError` catch
            // (`cli.js`, claude-agent-sdk 2.1.44). An interrupt with nothing in
            // progress writes nothing.
            //
            // `handle.cancel()` above is deliberately *not* gated the same way:
            // Escape's third branch cancels a trailing background subagent with
            // the turn already idle, and that is a real cancel that records no
            // entry. Marking it would invent a row the replay cannot reproduce.
            self.items.push(ChatItem::Interrupted);
        }
        self.settle_turn();
        // Stop can be the only terminal transition a live resource link gets;
        // the agent is allowed to never acknowledge the cooperative cancel.
        self.reconcile_tool_images(&ReconcileScope::All, cx);
        self.clear_phone_turn();
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
        // A park always starts unarmed: a Telegram or macro prompt can start the
        // turn this Stop cancels without any composer change to disarm a gesture
        // an earlier park left set, and one Enter would then fire the new queue.
        self.queue.resume_armed = false;
        // `settle_turn` mutated items (streaming → done, running tools →
        // cancelled, pending card → resolved), changing fold visibility and row
        // heights, so reproject and remeasure before notifying.
        self.rebuild_rows();
        self.resync_all_row_heights("stop");
        cx.notify();
    }

    /// Drop the stop marker back to the end of the transcript if anything landed
    /// behind it. Called on the cancel ack — the point after which no further
    /// chunk for the cancelled turn can arrive.
    ///
    /// Stop pushes the marker immediately (it has to: the ack may never come),
    /// but a chunk already on the wire still lands after it — nothing gates
    /// `AcpEvent::Update` on the open cancel window, and the adapter's stream
    /// path has no cancelled check of its own. Left in place that text reads as
    /// a promptless turn under the marker, since [`agent_run`] treats the marker
    /// as a run boundary; and the SDK persists the assistant message *before*
    /// its interrupt entry, so a restored pane would order it the other way.
    ///
    /// This is the one structural removal in the pane besides `items.clear()`,
    /// so it shifts item indices — but only for items *after* the marker, which
    /// by construction arrived in the window between Stop and its ack. The one
    /// thing that can already point there is a fold the user opened on that
    /// content while the ack was outstanding; it lands on the neighbouring key
    /// and self-corrects on the next toggle. Everything else is keyed by id
    /// (permissions, subagents) or recomputed by `rebuild_rows`.
    ///
    /// [`agent_run`]: super::super::agent_chat_helpers::agent_run
    pub(super) fn settle_stop_marker(&mut self) {
        if matches!(self.items.last(), Some(ChatItem::Interrupted)) {
            return;
        }
        let Some(ix) = self
            .items
            .iter()
            .rposition(|item| matches!(item, ChatItem::Interrupted))
        else {
            return;
        };
        self.items.remove(ix);
        self.items.push(ChatItem::Interrupted);
    }

    /// End the current turn locally and settle every still-live item. Model-only
    /// (no rows/notify) so the three call paths (Stop, `TurnEnded`, `Error`)
    /// share one settle sequence and can't drift. Idempotent.
    pub(super) fn settle_turn(&mut self) {
        self.queue.turn.finish();
        self.settle_run_state();
    }

    /// Settle everything this run left flagged live — the transcript *and* the
    /// plan beside it. [`Self::settle_turn`] is this plus `turn = Idle`; the
    /// other callers are the ends of a run that no `TurnEnded` closes — a
    /// `session/load` replay and the ack of a cancel — which is why this is
    /// reachable from `apply_event`.
    ///
    /// Every such exit owes it, and owes it for every store: a value left
    /// flagged live reads as running forever, while the pulse pump only repaints
    /// panes `is_busy()` calls working — so the glyph gets a blink nothing
    /// drives. One site, so a new store is added here rather than at each exit.
    pub(super) fn settle_run_state(&mut self) {
        finalize_streaming(&mut self.items);
        cancel_pending_tools(&mut self.items);
        cancel_pending_permission(self);
        cancel_pending_plan_entries(&mut self.plan);
    }

    /// Resolve the permission request `request_id` with the chosen option:
    /// mark that card resolved (by id, not position — several may be
    /// outstanding), send the decision, drop the id from the index. No-op if
    /// already answered or cancelled.
    pub fn respond_permission(
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
        if let Some(handle) = self.live_handle() {
            handle.respond_permission(request_id, decision);
        }
        // The card is now resolved, so it no longer force-stays-visible under a
        // collapsed response: reproject so it folds back into the process
        // immediately. It also renders shorter (buttons → outcome line), so
        // remeasure for the reflow (item count unchanged).
        self.rebuild_rows();
        self.resync_all_row_heights("permission-resolved");
        cx.notify();
    }

    /// Switch the active session mode on the *user's* behalf and remember it
    /// as this pane's own pick. A `ModeChanged` replaces the whole state if
    /// the agent disagrees.
    pub fn set_mode(&mut self, mode_id: String, cx: &mut Context<Self>) {
        // A mirror shows the CLI's mode; the pick would only be local.
        if self.is_read_only() {
            return;
        }
        self.picked_mode_id = Some(mode_id.clone());
        self.send_mode(mode_id, cx);
    }

    /// The protocol half [`Self::set_mode`] shares with a followed default:
    /// show the mode immediately, then ask the agent over the live handle
    /// (no-op when absent).
    fn send_mode(&mut self, mode_id: String, cx: &mut Context<Self>) {
        self.session_config
            .set_current_mode_optimistically(mode_id.clone());
        if let Some(h) = self.live_handle() {
            h.set_mode(mode_id);
        }
        cx.notify();
    }

    /// What this pane asks its agent for under `agents` — its own picks ahead
    /// of the catalog defaults.
    pub fn session_preferences(
        &self,
        agents: &[daruda_config::AgentDefinition],
    ) -> daruda_config::SessionPreferences {
        daruda_config::SessionPreferences::resolve(
            agents.iter().find(|a| a.id == self.agent_id),
            self.picked_model_id.as_deref(),
            self.picked_mode_id.as_deref(),
        )
    }

    /// Carry a config edit into the live session: each axis whose preference
    /// moved (`was` → `now`) is requested again, without becoming a pick. A
    /// pane not yet connected needs nothing — its connect reads `now`.
    /// `true` when the mode was switched.
    pub fn follow_session_preferences(
        &mut self,
        was: &daruda_config::SessionPreferences,
        now: &daruda_config::SessionPreferences,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.is_read_only() || self.live_handle().is_none() {
            return false;
        }
        // Model first: a model switch can rebuild the advertised modes.
        if was.model != now.model
            && let Some(model) = &now.model
            && let Some((option_id, current, choices)) =
                model_select(&self.session_config.config_options)
            && current != model
            && choices.iter().any(|c| &c.value == model)
        {
            let option_id = option_id.to_string();
            let value = daruda_acp::ConfigValueView::Id(model.clone());
            self.send_config_option(option_id, value, cx);
        }
        if was.modes != now.modes
            && let Some(state) = &self.session_config.modes
            && let Some(mode) = now
                .modes
                .iter()
                .find(|id| state.available.iter().any(|m| &m.id == *id))
            && *mode != state.current
        {
            self.send_mode(mode.clone(), cx);
            return true;
        }
        false
    }

    /// Change a select config option on the *user's* behalf (the config
    /// chips): show the pick immediately, ask the agent over the live handle,
    /// and remember a `Model`-category pick as this pane's own model. The
    /// reply replaces the whole option set via `ConfigOptionsChanged`.
    pub fn set_config_option(
        &mut self,
        config_id: String,
        value: daruda_acp::ConfigValueView,
        cx: &mut Context<Self>,
    ) {
        // A mirror shows the CLI's options; the pick would only be local, and
        // the model it remembers must stay the run's.
        if self.is_read_only() {
            return;
        }
        // Only the model axis is remembered, and only when this call names it.
        let picked_model = match &value {
            daruda_acp::ConfigValueView::Id(picked) => {
                model_select(&self.session_config.config_options)
                    .filter(|(option_id, _, _)| *option_id == config_id.as_str())
                    .map(|_| picked.clone())
            }
            daruda_acp::ConfigValueView::Bool(_) => None,
        };
        if let Some(model) = picked_model {
            self.picked_model_id = Some(model);
        }
        self.send_config_option(config_id, value, cx);
    }

    /// The protocol half both entry points share: show the value immediately,
    /// then ask the agent over the live handle (no-op when absent).
    fn send_config_option(
        &mut self,
        config_id: String,
        value: daruda_acp::ConfigValueView,
        cx: &mut Context<Self>,
    ) {
        self.session_config
            .set_option_value_optimistically(&config_id, &value);
        if let Some(h) = self.live_handle() {
            h.set_config_option(config_id, value);
        }
        cx.notify();
    }

    /// Shared teardown for a `/clear` reset and a post-`Error` retry: drop the
    /// live handle and event pump, wipe the conversation model + every runtime
    /// cache. Does NOT touch `session_id`/`replay`/`status` — the two
    /// callers differ there (fresh conversation vs. resume via `session/load`).
    fn teardown_transient_session_state(&mut self) {
        cancel_pending_permission(self);
        self.detach_handle();
        self._event_pump = None;
        self.items.clear();
        self.queue.pending_prompts.clear();
        self.queue.paused_prompts.clear();
        self.queue.editing_prompt = None;
        self.queue.resume_armed = false;
        self.pending_permissions.clear();
        self.permissions_told_to_phone.clear();
        self.phone_turn_state = None;
        self.queue.turn = Turn::Idle;
        self.activity.subagent_last_activity.clear();
        self.activity.tool_started_at.clear();
        self.activity.span = ActivitySpan::Idle;
        self.activity.pending_completion = None;
        // The records are keyed by item index with no session identity, so they
        // must not outlive the transcript they index — `/clear` restarts at 0
        // and a reconnect replays the history, either of which would otherwise
        // hang the previous conversation's numbers on the new rows.
        self.activity.turn_records.clear();
        self.session_usage = None;
        self.assets.clear();
        self.fold.clear_overrides();
        self.session_config = SessionConfig::default();
        self.plan.clear();
        self.plan_collapsed = false;
        self.session_title = None;
        self.session_updated_at = None;
    }

    /// Full local reset for `/clear`: teardown, then also drop the persisted
    /// session id. The caller supersedes this with a fresh `connect_agent_chat`
    /// right after, so the view never sits handle-less for a render.
    pub fn reset_for_new_session(&mut self, cx: &mut Context<Self>) {
        // A mirror has no conversation of its own to clear.
        if self.is_read_only() {
            return;
        }
        self.teardown_transient_session_state();
        // Clear the persisted id so a restart resumes the fresh session, not
        // the cleared conversation (Connected re-persists the new id).
        self.session_id = None;
        self.rebuild_rows(); // diff-splices list_state down to 0 rows
        // No resume target: a cleared conversation starts fresh.
        self.begin_connect(None, cx);
    }

    /// Reconnect after a terminal `Error` without losing the conversation:
    /// same teardown as [`Self::reset_for_new_session`] but keeps `session_id`,
    /// so the reconnect resumes via `session/load` and replays the history.
    ///
    /// `resume` is the target the caller is about to reconnect with, passed in
    /// rather than re-derived from `session_id` — the gate and the request it
    /// gates are then decided in one place, by the caller that owns both.
    pub fn retry_for_reconnect(&mut self, resume: Option<String>, cx: &mut Context<Self>) {
        self.teardown_transient_session_state();
        self.rebuild_rows();
        self.begin_connect(resume, cx);
    }

    /// Jump the conversation list to the bottom and re-engage `Tail` follow —
    /// the list re-arms tail-following once it lands at the end.
    pub fn scroll_to_bottom(&mut self, cx: &mut Context<Self>) {
        self.list_state.scroll_to_end();
        cx.notify();
    }
}
