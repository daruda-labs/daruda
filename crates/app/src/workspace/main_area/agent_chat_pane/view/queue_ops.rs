//! Prompt-queue send / edit / cancel routing for one Agent chat pane: the
//! composer send paths, the Telegram first-response watch's lifecycle, the
//! queued/parked prompt list ops, and the escape-key dispatch. Reads and
//! writes [`PromptQueue`](super::PromptQueue) (including `turn`, which stays
//! module-private to [`view`](super) — enforced by
//! `scripts/lint-agent-activity.sh`) plus `items`, so these stay `impl
//! AgentChatView` methods rather than moving onto `PromptQueue` itself.

use daruda_acp::ChatItem;
use gpui::Context;

use super::super::agent_chat_helpers::fold_active;
use super::super::fold::FoldKey;
use super::{
    AgentChatView, AgentSessionStatus, EscapeOutcome, FirstResponseOutcome, FirstResponseWatch,
    PromptDispatch, PromptId, PromptOrigin, QueuedPrompt, TelegramFirstResponseEffect, Turn,
};

impl AgentChatView {
    /// Test-only hook: mark a prompt turn in flight (as `send_prompt_text` does).
    /// The `Turn` field is module-private so production code cannot read it; tests
    /// drive it through these sanctioned accessors instead of touching the field.
    #[cfg(test)]
    pub(in crate::workspace) fn set_turn_in_flight(&mut self) {
        self.queue.turn = Turn::InFlight {
            started_at: std::time::Instant::now(),
        };
    }

    /// Test-only hook: return the turn to idle (as `settle_turn` does).
    #[cfg(test)]
    pub(in crate::workspace) fn set_turn_idle(&mut self) {
        self.queue.turn = Turn::Idle;
    }

    /// Test-only hook: whether the turn is idle (no prompt in flight).
    #[cfg(test)]
    pub(in crate::workspace) fn turn_is_idle(&self) -> bool {
        !self.queue.turn.is_in_flight()
    }

    /// Test-only hook: run the model half of the queued-prompt drain without a
    /// live ACP handle. This mirrors what `pump_pending_prompt` does immediately
    /// before sending the returned text over the handle.
    #[cfg(test)]
    pub(in crate::workspace) fn drain_next_queued_prompt_for_test(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        self.drain_next_queued_prompt(cx)
    }

    /// Test-only hook: arm the Telegram first-response watch without needing a
    /// live ACP handle.
    #[cfg(test)]
    pub(in crate::workspace) fn start_telegram_first_response_watch_for_test(
        &mut self,
        started_at: std::time::Instant,
    ) {
        self.telegram_first_response_watch =
            Some(FirstResponseWatch::start(started_at, self.items.len()));
    }

    /// Send `text` as a prompt from the bottom-dock composer. Sugar over
    /// [`Self::send_prompt_text_inner`] with [`PromptOrigin::InApp`] — an
    /// in-app prompt never arms [`Self::telegram_first_response_watch`], so the
    /// dispatch outcome is uninteresting here.
    pub(in crate::workspace) fn send_prompt_text(&mut self, text: String, cx: &mut Context<Self>) {
        self.send_prompt_text_inner(text, PromptOrigin::InApp, cx);
    }

    /// Send `text` as a prompt relayed from a phone-tapped Telegram reply.
    /// Sugar over [`Self::send_prompt_text_inner`] with [`PromptOrigin::Telegram`]
    /// — the caller (`Workspace::inject_bot_reply`) needs the returned
    /// [`PromptDispatch`] to know whether to send a "queued" notice.
    pub(in crate::workspace) fn send_prompt_text_for_telegram(
        &mut self,
        text: String,
        cx: &mut Context<Self>,
    ) -> PromptDispatch {
        self.send_prompt_text_inner(text, PromptOrigin::Telegram, cx)
    }

    /// Send `text` as a prompt. When the session is connected and idle, echo it
    /// into the transcript and forward it over the session, marking a turn in
    /// flight. Otherwise (not connected yet, a turn already in flight, or a
    /// Stop's cancel still outstanding) enqueue it WITHOUT echoing — a queued
    /// prompt lives only in the queue (surfaced by the bottom-dock strip) until
    /// [`Self::pump_pending_prompt`] drains it and echoes it at send time.
    /// `origin` carries through to [`Self::start_telegram_watch_if`] at
    /// whichever point (here, or later in [`Self::drain_next_queued_prompt`])
    /// the prompt actually reaches the wire.
    fn send_prompt_text_inner(
        &mut self,
        text: String,
        origin: PromptOrigin,
        cx: &mut Context<Self>,
    ) -> PromptDispatch {
        // Editing an existing queued prompt: replace that slot's text in place
        // (order preserved) and return — this is not a new turn or a new queue
        // entry. `.take()` clears the editing flag whether or not the target is
        // still queued; if it drained onto the wire while the user was editing,
        // the second `let` fails and we fall through to handle `text` as a
        // brand-new prompt so nothing typed is lost.
        if origin == PromptOrigin::InApp
            && let Some(id) = self.queue.editing_prompt.take()
            && let Some(qp) = self
                .queue
                .pending_prompts
                .iter_mut()
                .chain(self.queue.paused_prompts.iter_mut())
                .find(|q| q.id == id)
        {
            qp.text = text;
            // Queue-only change: `items` (and thus the projected rows) are
            // untouched, so notifying re-stages the strip without a transcript
            // reproject.
            cx.notify();
            return PromptDispatch::Queued;
        }
        let dispatch = if matches!(self.status, AgentSessionStatus::Connected)
            && let Some(handle) = &self.handle
            && !self.queue.turn.is_in_flight()
            && !self.activity.cancel_in_flight
        {
            // Connected and idle: send now, mark the turn in flight, and echo.
            handle.send_prompt(text.clone());
            self.queue.turn = Turn::InFlight {
                started_at: std::time::Instant::now(),
            };
            self.echo_prompt(text, cx);
            self.start_telegram_watch_if(origin);
            PromptDispatch::SentNow
        } else {
            // Not connected yet (lazy connect happens on first focus), a turn is
            // already in flight, or a Stop's cancel is still outstanding
            // (`cancel_in_flight` — buffer client-side so a second Stop can clear
            // it and it can't race the cancel's ack onto the wire). Enqueue in
            // submission order without echoing; it drains one-per-turn via
            // `pump_pending_prompt` (after `Connected`, on each `TurnEnded`, and
            // when the cancel window closes) and is echoed then. Do *not* mark
            // the turn in flight: nothing new is on the wire yet.
            self.enqueue_prompt(text, origin);
            PromptDispatch::Queued
        };
        cx.notify();
        dispatch
    }

    /// Arms [`Self::telegram_first_response_watch`] the instant a
    /// Telegram-origin prompt actually reaches the wire — called from both
    /// dispatch paths (immediate-send in [`Self::send_prompt_text_inner`],
    /// queue-drain in [`Self::drain_next_queued_prompt`]) so the watch always
    /// starts at the true dispatch point, never at enqueue time. A no-op for
    /// [`PromptOrigin::InApp`].
    pub(super) fn start_telegram_watch_if(&mut self, origin: PromptOrigin) {
        if origin == PromptOrigin::Telegram {
            self.telegram_first_response_watch = Some(FirstResponseWatch::start(
                std::time::Instant::now(),
                self.items.len(),
            ));
        }
    }

    /// Whether this pane is still waiting to produce the first phone-visible
    /// response for a Telegram-origin prompt.
    pub(in crate::workspace) fn is_waiting_for_telegram_first_response(&self) -> bool {
        self.telegram_first_response_watch.is_some()
    }

    /// Resolve and clear the active Telegram first-response watch if a completed
    /// text reply or first tool call has appeared since the watched prompt was
    /// echoed. Returns `None` when there is no watch or the turn has not yet
    /// produced a qualifying visible response.
    pub(super) fn take_telegram_first_response(&mut self) -> Option<FirstResponseOutcome> {
        let watch = self.telegram_first_response_watch?;
        let outcome = watch.resolve(&self.items)?;
        self.telegram_first_response_watch = None;
        Some(outcome)
    }

    /// Finish the active watch at a terminal boundary. A final streaming text is
    /// resolved first (because `settle_turn` has just finalized it); otherwise
    /// the old immediate "received" ack is used as the fallback.
    pub(super) fn finish_telegram_first_response_watch(&mut self) -> TelegramFirstResponseEffect {
        if let Some(outcome) = self.take_telegram_first_response() {
            return TelegramFirstResponseEffect::Relay(outcome);
        }
        if self.telegram_first_response_watch.take().is_some() {
            return TelegramFirstResponseEffect::Fallback;
        }
        TelegramFirstResponseEffect::None
    }

    /// Clear a watch that was superseded by a stronger phone-visible signal
    /// (currently a permission prompt) without emitting the generic fallback.
    pub(super) fn clear_telegram_first_response_watch(&mut self) {
        self.telegram_first_response_watch = None;
    }

    /// Periodic safety net for turns that stay silent for the first-response
    /// window. Returns true only once, clearing the watch so later flush ticks do
    /// not repeat the fallback ack.
    pub(in crate::workspace) fn take_telegram_first_response_fallback_if_overdue(
        &mut self,
        now: std::time::Instant,
        timeout_secs: u64,
    ) -> bool {
        if self
            .telegram_first_response_watch
            .is_some_and(|watch| watch.is_overdue(now, timeout_secs))
        {
            self.telegram_first_response_watch = None;
            return true;
        }
        false
    }

    /// Append `text` to the transcript as a `UserText` item and refresh the
    /// render (mermaid raster + row projection + scroll-to-end). This is the
    /// prompt-echo, shared by the send-now path in [`Self::send_prompt_text`]
    /// and the drain in [`Self::pump_pending_prompt`] — the echo now happens at
    /// *send* time, not when a prompt is queued.
    pub(super) fn echo_prompt(&mut self, text: String, cx: &mut Context<Self>) {
        self.preserve_tail_response_expansion();
        self.items.push(ChatItem::UserText(text));
        // There is no `ToolCall` at a prompt-echo, so the diff reconcile would
        // be a no-op here; diff editors are reconciled solely on the event-pump
        // path. Same reasoning rules out `reconcile_tool_images` here — no
        // ToolCall at a prompt echo, so no tool image to reconcile. The echoed
        // `UserText` renders its markdown directly; a prompt may carry a
        // ` ```mermaid ` fence, so rasterize those (no-op when there are none).
        let dark = Self::host_is_dark(cx);
        self.reconcile_mermaid(dark, cx);
        self.rebuild_rows();
        // Submitting a prompt jumps the view to the bottom so the user sees their
        // message and the streaming response. `scroll_to_end` only repositions
        // the viewport; `FollowMode::Tail` re-engages on the first layout pass
        // that lands at the bottom (gpui `list` re-arms following there), so the
        // streaming response keeps sticking — no manual stick flag needed.
        self.list_state.scroll_to_end();
    }

    /// Preserve the currently visible tail response before appending the next
    /// user prompt. Response folds naturally expand while they are the last
    /// turn; appending a new `UserText` would otherwise make that same response
    /// non-last and auto-collapse it, hiding agent prose at the exact moment the
    /// user submits a follow-up. If the user explicitly collapsed it, the
    /// effective state is already `false`, so no override is written.
    fn preserve_tail_response_expansion(&mut self) {
        let Some(anchor) = self
            .items
            .iter()
            .rposition(|item| matches!(item, ChatItem::UserText(_)))
        else {
            return;
        };
        let key = FoldKey::Response(anchor);
        if self.fold.is_expanded(&key, fold_active(&key, &self.items)) {
            self.fold.set_all([key], true);
        }
    }

    /// Resume a parked queue that a Stop preserved. Moves the parked prompts
    /// back to the FRONT of the live queue (FIFO — they were submitted before
    /// anything queued after the Stop), then pumps so the first one dispatches
    /// when the session is connected and idle. No-op (no notify) when nothing is
    /// parked. Backs the queue strip's Resume button via the
    /// `Workspace::resume_queued_prompts` shim (one-way data flow).
    pub(in crate::workspace) fn resume_queue(&mut self, cx: &mut Context<Self>) {
        if self.queue.paused_prompts.is_empty() {
            return;
        }
        let mut resumed = std::mem::take(&mut self.queue.paused_prompts);
        resumed.append(&mut self.queue.pending_prompts);
        self.queue.pending_prompts = resumed;
        // Dispatch the first one now if the session can prompt; a no-op offline
        // (the queue just sits in `pending_prompts` and drains on connect).
        self.pump_pending_prompt(cx);
        cx.notify();
    }

    /// Push `text` onto the pending-prompt queue with a freshly minted
    /// [`PromptId`], returning that id. Does NOT echo into the transcript and
    /// does NOT notify — the caller notifies once after mutating. `origin`
    /// travels with the entry so [`Self::drain_next_queued_prompt`] knows
    /// whether to arm the Telegram watch once this prompt actually dispatches.
    pub(super) fn enqueue_prompt(&mut self, text: String, origin: PromptOrigin) -> PromptId {
        let id = PromptId(self.queue.next_prompt_id);
        self.queue.next_prompt_id += 1;
        self.queue
            .pending_prompts
            .push(QueuedPrompt { id, text, origin });
        id
    }

    /// Remove the queued prompt with `id` from the queue — the live queue OR the
    /// parked queue, since the strip renders the × affordance on both. No-op (no
    /// notify) when `id` is not present. Backs the bottom-dock strip's per-item
    /// × button via the `Workspace::remove_queued_prompt` shim (one-way data
    /// flow).
    pub(in crate::workspace) fn remove_queued(&mut self, id: PromptId, cx: &mut Context<Self>) {
        let before = self.queue.pending_prompts.len() + self.queue.paused_prompts.len();
        self.queue.pending_prompts.retain(|q| q.id != id);
        self.queue.paused_prompts.retain(|q| q.id != id);
        if self.queue.pending_prompts.len() + self.queue.paused_prompts.len() != before {
            // Deliberately does NOT clear `editing_prompt` if the removed row was
            // the edit target: `send_prompt_text` takes the flag and, finding the
            // id gone from both queues, falls through to enqueue the composer text
            // as a new prompt — so nothing typed is lost (see that path + the
            // `send_prompt_text_editing_target_gone_falls_through_to_new` test).
            // Queue-only change: the transcript rows are unaffected, so notify
            // re-stages the strip without a transcript reproject.
            cx.notify();
        }
    }

    /// Drop every queued prompt — both the live and the parked queue (clear-all
    /// empties the strip regardless of parking). No-op (no notify) when both are
    /// already empty. Backs the bottom-dock strip's "clear all" button via the
    /// `Workspace::clear_queued_prompts` shim (one-way data flow), and the
    /// second-Esc discard via `Workspace::cancel_agent_turn_if_active`.
    pub(in crate::workspace) fn clear_queue(&mut self, cx: &mut Context<Self>) {
        if self.queue.pending_prompts.is_empty() && self.queue.paused_prompts.is_empty() {
            return;
        }
        self.queue.pending_prompts.clear();
        self.queue.paused_prompts.clear();
        self.queue.editing_prompt = None;
        // Queue-only change: the transcript rows are unaffected, so notify
        // re-stages the strip without a transcript reproject.
        cx.notify();
    }

    /// Mark the queued prompt `id` as the one being edited in the composer.
    /// Backs the bottom-dock strip's ✎ button and ↑-in-empty-composer via the
    /// `Workspace::begin_edit_queued_prompt` shim (which pulls the text into the
    /// input); this view only records which slot a subsequent send replaces.
    pub(in crate::workspace) fn begin_edit(&mut self, id: PromptId, cx: &mut Context<Self>) {
        self.queue.editing_prompt = Some(id);
        cx.notify();
    }

    /// Clear the editing flag (the composer edit was cancelled). No-op (no
    /// notify) when nothing was being edited. Backs the strip's cancel (↩)
    /// button via the `Workspace::cancel_edit_queued_prompt` shim, which also
    /// empties the composer.
    pub(in crate::workspace) fn cancel_edit(&mut self, cx: &mut Context<Self>) {
        if self.queue.editing_prompt.take().is_some() {
            cx.notify();
        }
    }

    /// Move the next queued prompt into the active-turn model and return the
    /// text that should be sent over ACP. No-op while a turn or cancel is active.
    fn drain_next_queued_prompt(&mut self, cx: &mut Context<Self>) -> Option<String> {
        if self.queue.turn.is_in_flight()
            || self.activity.cancel_in_flight
            || self.queue.pending_prompts.is_empty()
        {
            return None;
        }
        let qp = self.queue.pending_prompts.remove(0);
        // If the drained entry was the one being edited, drop the stale editing
        // flag so a later send doesn't try to replace an id that is no longer
        // queued.
        if self.queue.editing_prompt == Some(qp.id) {
            self.queue.editing_prompt = None;
        }
        self.queue.turn = Turn::InFlight {
            started_at: std::time::Instant::now(),
        };
        let text = qp.text;
        self.echo_prompt(text.clone(), cx);
        self.start_telegram_watch_if(qp.origin);
        Some(text)
    }

    /// Send the next single buffered prompt iff the session is connected and no
    /// turn is currently in flight. Pops the FRONT of the queue (FIFO), forwards
    /// it over the handle, marks the turn in flight, and notifies. No-op when
    /// there is no handle, a turn is already running, or the buffer is empty.
    ///
    /// This drains the queue one prompt per turn-completion (the connect site
    /// pumps the first; each `TurnEnded` pumps the next) so the view never
    /// tracks more than one turn at a time — the Stop / Send affordance then
    /// reflects the single live turn instead of clearing while later queued
    /// turns are still streaming. A queued prompt was NOT echoed when it was
    /// buffered, so the drain echoes it here (at send time).
    pub(in crate::workspace) fn pump_pending_prompt(&mut self, cx: &mut Context<Self>) {
        // Hold the queue until the session is fully connected and while a cancel
        // is still outstanding (`cancel_in_flight`): a handle exists before the
        // ACP handshake/load has completed, but prompt delivery is only safe once
        // `Connected` has opened the session's prompt loop.
        if !matches!(self.status, AgentSessionStatus::Connected) || self.handle.is_none() {
            return;
        };
        let Some(text) = self.drain_next_queued_prompt(cx) else {
            return;
        };
        if let Some(handle) = &self.handle {
            handle.send_prompt(text);
        }
        cx.notify();
    }

    /// Resolve what the Escape shortcut should do for this pane and apply it,
    /// reporting the kind so the `Workspace` shim fires the settle-edge
    /// completion only when a turn was cancelled. Priority:
    ///
    /// 1. An in-flight turn → cancel + park the queue ([`Self::cancel_turn`]).
    /// 2. Else a parked queue → discard it (the "Esc twice clears the queue"
    ///    gesture). This takes precedence over trailing background-subagent
    ///    liveness so the gesture isn't blocked while a post-Stop subagent is
    ///    still inside its quiescence window (`is_busy()` can stay true there).
    /// 3. Else a still-running background subagent (no parked queue) → cancel it
    ///    (keeps parity with the Stop button, which shows whenever `is_busy()`).
    /// 4. Else nothing to do → propagate Escape.
    pub(in crate::workspace) fn handle_escape(&mut self, cx: &mut Context<Self>) -> EscapeOutcome {
        if self.queue.turn.is_in_flight() {
            self.cancel_turn(cx);
            return EscapeOutcome::Cancelled;
        }
        if !self.queue.paused_prompts.is_empty() {
            self.clear_queue(cx);
            return EscapeOutcome::ClearedQueue;
        }
        if self.is_busy() {
            self.cancel_turn(cx);
            return EscapeOutcome::Cancelled;
        }
        EscapeOutcome::Ignored
    }
}
