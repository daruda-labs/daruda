//! Prompt-queue send / edit / cancel routing for one Agent chat pane: the
//! composer send paths, queued/parked prompt list ops, and escape-key dispatch. Stay `impl
//! AgentChatView` (not [`PromptQueue`](super::PromptQueue)) since they also
//! need `items`.

use daruda_acp::ChatItem;
use daruda_store::tasks::ExecutionRef;
use gpui::Context;

use super::super::agent_chat_helpers::fold_context;
use super::super::fold::FoldKey;
use super::super::phone_turn::{PhoneTurn, PhoneTurnMode};
use super::{
    AgentChatEvent, AgentChatView, AgentSessionStatus, EmptySubmitOutcome, EscapeOutcome,
    InternalDispatch, PromptDispatch, PromptDropReason, PromptId, PromptOrigin, PromptQueue,
    QueuedPrompt,
};

mod phone;

#[derive(Clone, Copy)]
pub(super) enum EditSource {
    Composer,
    External,
}

impl PromptQueue {
    /// Empty the live and parked queues together with the edit/resume state
    /// that only means something while they hold prompts, returning the task
    /// runs among the discarded prompts (parked first, in queue order).
    pub(super) fn discard_all(&mut self) -> Vec<ExecutionRef> {
        let tasks = self
            .paused_prompts
            .drain(..)
            .chain(self.pending_prompts.drain(..))
            .filter_map(|q| q.task)
            .collect();
        self.editing_prompt = None;
        self.resume_armed = false;
        tasks
    }
}

impl AgentChatView {
    pub fn discard_pending_prompts(&mut self, reason: PromptDropReason, cx: &mut Context<Self>) {
        let dropped = self.queue.discard_all();
        self.report_dropped(dropped, reason, cx);
        cx.notify();
    }

    /// Test-only hook: run the model half of the queued-prompt drain without a
    /// live ACP handle, as `pump_pending_prompt` does before sending.
    #[cfg(test)]
    pub(super) fn drain_next_queued_prompt_for_test(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        let prompt = self.take_next_queued_prompt(cx)?;
        self.activate_queued_prompt(&prompt, cx);
        Some(prompt.text)
    }

    /// Send `text` from the bottom-dock composer. Sugar over
    /// `send_prompt_text_inner` with [`PromptOrigin::InApp`] — the
    /// dispatch outcome is uninteresting since an in-app prompt never arms
    /// the Telegram watch.
    /// Returns the dispatch for the same reason the Telegram form does: a
    /// full queue refuses the prompt, and a caller that discarded that would
    /// tell the user it was sent.
    pub fn send_prompt_text(&mut self, text: String, cx: &mut Context<Self>) -> PromptDispatch {
        self.send_prompt_text_inner(text, PromptOrigin::InApp, None, EditSource::Composer, cx)
    }

    /// Submit a host-supplied, non-phone prompt without committing a draft
    /// currently being edited in the local composer.
    pub fn send_external_prompt_text(
        &mut self,
        text: String,
        cx: &mut Context<Self>,
    ) -> PromptDispatch {
        self.send_prompt_text_inner(text, PromptOrigin::InApp, None, EditSource::External, cx)
    }

    /// Send a literal host notice only if it can reach the wire now. It never
    /// queues, so it cannot pass parked or queued user prompts. `NotNow` and
    /// `ReadOnly` leave the composer edit, the queue and the phone watch
    /// untouched; `Unavailable` means the handoff itself failed, which reports
    /// the dead transport exactly like any send does (queued prompts dropped,
    /// handle detached, error shown).
    pub fn try_send_internal_prompt_text(
        &mut self,
        text: String,
        cx: &mut Context<Self>,
    ) -> InternalDispatch {
        self.try_send_internal_prompt_text_with_phone_mode(text, None, cx)
    }

    /// Install phone routing only after the literal prompt reaches the wire.
    pub fn try_send_internal_prompt_text_with_phone_mode(
        &mut self,
        text: String,
        phone_mode: Option<PhoneTurnMode>,
        cx: &mut Context<Self>,
    ) -> InternalDispatch {
        if self.is_read_only() {
            return InternalDispatch::ReadOnly;
        }
        if !self.accepts_internal_prompt_now() {
            return InternalDispatch::NotNow;
        }
        if !self.dispatch_now(text, Vec::new(), PromptOrigin::Internal, None, cx) {
            return InternalDispatch::Unavailable;
        }
        if let Some(mode) = phone_mode {
            self.phone_turn_state = Some(PhoneTurn::with_mode(
                mode,
                std::time::Instant::now(),
                self.items.len(),
            ));
        }
        cx.notify();
        InternalDispatch::Sent
    }

    /// Whether [`Self::try_send_internal_prompt_text`] would send now rather
    /// than refuse. A host waiting to retry observes the view against this.
    ///
    /// Prompts parked by Stop count as waiting: a notice never goes ahead of a
    /// user prompt, so it holds until the user resumes, edits away or discards
    /// the parked queue. Only the notice waits; an explicit tool call such as
    /// `daruda_run_wait` does not go through here.
    pub fn accepts_internal_prompt_now(&self) -> bool {
        !self.is_read_only() && self.queued_prompt_count() == 0 && self.can_dispatch_now(&[])
    }

    /// Send `text` on behalf of the task run `task`. Behaves as
    /// [`Self::send_prompt_text`], and additionally reports
    /// [`AgentChatEvent::PromptSent`] when the prompt reaches the wire or
    /// [`AgentChatEvent::PromptsDropped`] if it is discarded unsent.
    pub fn send_task_prompt_text(
        &mut self,
        text: String,
        task: ExecutionRef,
        cx: &mut Context<Self>,
    ) -> PromptDispatch {
        self.send_prompt_text_inner(
            text,
            PromptOrigin::InApp,
            Some(task),
            EditSource::External,
            cx,
        )
    }

    /// Send `text` relayed from a phone-tapped Telegram reply. Sugar over
    /// `send_prompt_text_inner` with [`PromptOrigin::Telegram`] — the
    /// caller needs the returned [`PromptDispatch`] to know whether a
    /// "queued" notice is owed.
    pub fn send_prompt_text_for_telegram(
        &mut self,
        text: String,
        cx: &mut Context<Self>,
    ) -> PromptDispatch {
        self.send_prompt_text_inner(text, PromptOrigin::Telegram, None, EditSource::External, cx)
    }

    /// Send `text`. When connected and idle, echo it and forward it over the
    /// session, marking a turn in flight. Otherwise enqueue it WITHOUT
    /// echoing — [`Self::pump_pending_prompt`] drains and echoes it later.
    /// `origin` arms the Telegram watch at whichever point it dispatches;
    /// `task` is reported to the host when the prompt is sent or dropped.
    fn send_prompt_text_inner(
        &mut self,
        text: String,
        origin: PromptOrigin,
        task: Option<ExecutionRef>,
        edit_source: EditSource,
        cx: &mut Context<Self>,
    ) -> PromptDispatch {
        self.send_prompt_content(text, Vec::new(), origin, task, edit_source, cx)
    }

    pub(super) fn send_prompt_content(
        &mut self,
        text: String,
        attachments: Vec<daruda_acp::PromptAttachment>,
        origin: PromptOrigin,
        task: Option<ExecutionRef>,
        edit_source: EditSource,
        cx: &mut Context<Self>,
    ) -> PromptDispatch {
        if self.is_read_only() {
            return PromptDispatch::ReadOnly;
        }
        // Editing an existing queued prompt: replace that slot's text in place
        // (order preserved) and return — this is not a new turn or a new queue
        // entry. `.take()` clears the editing flag whether or not the target is
        // still queued; if it drained onto the wire while the user was editing,
        // the second `let` fails and we fall through to handle `text` as a
        // brand-new prompt so nothing typed is lost. External submissions
        // never commit an edit in the local composer.
        if matches!(edit_source, EditSource::Composer)
            && let Some(id) = self.queue.editing_prompt.take()
            && let Some(qp) = self
                .queue
                .pending_prompts
                .iter_mut()
                .chain(self.queue.paused_prompts.iter_mut())
                .find(|q| q.id == id)
        {
            qp.text = text;
            qp.attachments.extend(attachments);
            // Queue-only change: `items` (and thus the projected rows) are
            // untouched, so notifying re-stages the strip without a transcript
            // reproject.
            cx.notify();
            return PromptDispatch::Queued;
        }
        let dispatch = if self.can_dispatch_now(&attachments) {
            if !self.dispatch_now(text, attachments, origin, task, cx) {
                return PromptDispatch::Unavailable;
            }
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
            match self.enqueue_attached_prompt(text, attachments, origin, task) {
                Some(_) => PromptDispatch::Queued,
                None => PromptDispatch::QueueFull,
            }
        };
        cx.notify();
        dispatch
    }

    /// Whether a prompt with `attachments` may go on the wire now: connected,
    /// no turn in flight, and no Stop's cancel acknowledgment outstanding.
    fn can_dispatch_now(&self, attachments: &[daruda_acp::PromptAttachment]) -> bool {
        matches!(self.status, AgentSessionStatus::Connected)
            && self.live_handle().is_some()
            && self.queue.turn.can_dispatch()
            && attachments
                .iter()
                .all(|attachment| attachment.supported_by(self.session_capabilities))
    }

    /// Send now, mark the turn in flight, and echo. `false` when the session
    /// refused the handoff. What goes on the wire and what the transcript
    /// shows can differ — see [`Self::wire_text`].
    fn dispatch_now(
        &mut self,
        text: String,
        attachments: Vec<daruda_acp::PromptAttachment>,
        origin: PromptOrigin,
        task: Option<ExecutionRef>,
        cx: &mut Context<Self>,
    ) -> bool {
        // Resolved before the handle is borrowed: taking the briefing needs `&mut self`.
        let wire = self.wire_text(&text);
        if !self.live_handle().is_some_and(|handle| {
            handle
                .try_send_prompt_with_attachments(wire, attachments.clone())
                .is_ok()
        }) {
            self.fail_prompt_handoff(task, cx);
            return false;
        }
        self.report_sent(task, cx);
        self.start_dispatched_turn();
        self.echo_attached_prompt(text, &attachments, cx);
        self.arm_phone_turn_if(origin);
        true
    }

    /// Tell the host a task's prompt was just handed to the live session.
    /// Called right after the handle send on both dispatch paths.
    fn report_sent(&self, task: Option<ExecutionRef>, cx: &mut Context<Self>) {
        if let Some(task) = task {
            self.emit_task_event(
                AgentChatEvent::PromptSent {
                    task,
                    session_id: self.session_id.clone(),
                },
                cx,
            );
        }
    }

    /// Tell the host which task prompts were discarded unsent. Silent when
    /// none were tagged, so a human-typed queue emits nothing new.
    pub(super) fn report_dropped(
        &self,
        tasks: Vec<ExecutionRef>,
        reason: PromptDropReason,
        cx: &mut Context<Self>,
    ) {
        if !tasks.is_empty() {
            self.emit_task_event(AgentChatEvent::PromptsDropped { tasks, reason }, cx);
        }
    }

    fn emit_task_event(&self, event: AgentChatEvent, cx: &mut Context<Self>) {
        let emitter = cx.entity().clone();
        cx.emit(event);
        // SAFETY: GPUI releases dropped emitters before queued events. Retain
        // this emitter through delivery to the process-owned task subscriber.
        cx.defer(move |_| drop(emitter));
    }

    /// Mark a prompt that just reached the wire as the turn in flight. A new
    /// turn retires the previous turn's overage banner.
    fn start_dispatched_turn(&mut self) {
        self.queue.turn.start(std::time::Instant::now());
        self.overage_notice = None;
    }

    /// Append `text` to the transcript as a `UserText` item and refresh the
    /// render (mermaid raster + row projection + scroll-to-end). Shared by the
    /// send-now path and the queue drain — the echo happens at *send* time.
    pub(super) fn echo_prompt(&mut self, text: String, cx: &mut Context<Self>) {
        self.preserve_tail_response_expansion();
        self.items.push(ChatItem::UserText(text));
        // An echo appends a `UserText`, never a `ToolCall`, so all three
        // tool-derived reconciles (diff editors, output editors, tool images)
        // would be no-ops here; they run solely on the event-pump path. A prompt
        // may carry a ` ```mermaid ` fence, so rasterize those.
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
    /// prompt — otherwise the new `UserText` makes it non-last and it
    /// auto-collapses, hiding agent prose right as the user submits a follow-up.
    /// Recorded as a hold, never as an override: an override means "the user
    /// chose this", outranks the Mode chip forever, and would leave every past
    /// response pinned open after a few prompts.
    fn preserve_tail_response_expansion(&mut self) {
        let held = self
            .items
            .iter()
            .rposition(|item| matches!(item, ChatItem::UserText(_)))
            // The bar is keyed by the response's first item, not the user turn
            // it answers — a hold recorded against the anchor would never match.
            .map(|anchor| anchor + 1)
            .filter(|&run_start| {
                let key = FoldKey::Response(run_start);
                self.fold
                    .is_expanded(&key, fold_context(&key, &self.items, &self.live_units))
            });
        self.fold.hold_response(held);
    }

    /// Resume a parked queue: move the parked prompts back to the FRONT of the
    /// live queue (FIFO), then pump so the first dispatches once connected and
    /// idle. No-op when nothing is parked.
    pub fn resume_queue(&mut self, cx: &mut Context<Self>) {
        if self.queue.paused_prompts.is_empty() {
            return;
        }
        let mut resumed = std::mem::take(&mut self.queue.paused_prompts);
        resumed.append(&mut self.queue.pending_prompts);
        self.queue.pending_prompts = resumed;
        self.queue.resume_armed = false;
        // Dispatch the first one now if the session can prompt; a no-op offline
        // (the queue just sits in `pending_prompts` and drains on connect).
        self.pump_pending_prompt(cx);
        cx.notify();
    }

    /// Push `text` onto the pending-prompt queue with a freshly minted
    /// [`PromptId`]. Does NOT echo or notify — the caller does that once after
    /// mutating. `origin` travels with the entry to arm the Telegram watch
    /// when it actually dispatches.
    /// Both queues' depth — what the cap is measured against, and what the
    /// control surface reports. A paused prompt still has to drain before a
    /// new one does, so leaving it out would let the cap be bypassed by
    /// pressing Stop.
    pub fn queued_prompt_count(&self) -> usize {
        self.queue.pending_prompts.len() + self.queue.paused_prompts.len()
    }

    /// An untagged, attachment-free [`Self::enqueue_attached_prompt`] for
    /// the tests and the capture seed.
    #[cfg(any(test, feature = "screenshot"))]
    pub(super) fn enqueue_prompt(
        &mut self,
        text: String,
        origin: PromptOrigin,
    ) -> Option<PromptId> {
        self.enqueue_attached_prompt(text, Vec::new(), origin, None)
    }

    /// Append a prompt, or refuse when the pane is already holding its limit.
    ///
    /// The cap lives here rather than only on the MCP path because every
    /// producer can outrun the drain: an agent's tool calls, a phone `/say`
    /// hammered from a pocket, and a person holding Enter all queue through
    /// this one function.
    fn enqueue_attached_prompt(
        &mut self,
        text: String,
        attachments: Vec<daruda_acp::PromptAttachment>,
        origin: PromptOrigin,
        task: Option<ExecutionRef>,
    ) -> Option<PromptId> {
        if self.queued_prompt_count() >= crate::QUEUE_DEPTH_MAX {
            return None;
        }
        let id = PromptId(self.queue.next_prompt_id);
        self.queue.next_prompt_id += 1;
        self.queue.pending_prompts.push(QueuedPrompt {
            id,
            text,
            attachments,
            origin,
            task,
        });
        Some(id)
    }

    /// Remove the queued prompt `id` from either the live or parked queue,
    /// since the strip renders the × on both. No-op when not present.
    pub fn remove_queued(&mut self, id: PromptId, cx: &mut Context<Self>) {
        let removed = [
            &mut self.queue.pending_prompts,
            &mut self.queue.paused_prompts,
        ]
        .into_iter()
        .find_map(|prompts| {
            let at = prompts.iter().position(|q| q.id == id)?;
            Some(prompts.remove(at))
        });
        if let Some(removed) = removed {
            self.report_dropped(
                removed.task.into_iter().collect(),
                PromptDropReason::RemovedByUser,
                cx,
            );
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

    /// Drop every queued prompt, live and parked (clear-all empties the strip
    /// regardless of parking). No-op when both are already empty.
    pub fn clear_queue(&mut self, cx: &mut Context<Self>) {
        if self.queue.pending_prompts.is_empty() && self.queue.paused_prompts.is_empty() {
            return;
        }
        let dropped = self.queue.discard_all();
        self.report_dropped(dropped, PromptDropReason::RemovedByUser, cx);
        // Queue-only change: the transcript rows are unaffected, so notify
        // re-stages the strip without a transcript reproject.
        cx.notify();
    }

    /// Mark the queued prompt `id` as the one being edited in the composer —
    /// this view only records which slot a subsequent send replaces.
    pub fn begin_edit(&mut self, id: PromptId, cx: &mut Context<Self>) {
        self.queue.editing_prompt = Some(id);
        cx.notify();
    }

    /// Clear the editing flag (the composer edit was cancelled). No-op when
    /// nothing was being edited.
    pub fn cancel_edit(&mut self, cx: &mut Context<Self>) {
        if self.queue.editing_prompt.take().is_some() {
            cx.notify();
        }
    }

    /// Take the next deliverable prompt without starting its turn yet.
    fn take_next_queued_prompt(&mut self, cx: &mut Context<Self>) -> Option<QueuedPrompt> {
        if !self.queue.turn.can_dispatch() || self.queue.pending_prompts.is_empty() {
            return None;
        }
        if self.queue.pending_prompts[0]
            .attachments
            .iter()
            .any(|attachment| !attachment.supported_by(self.session_capabilities))
        {
            cx.emit(super::AgentChatEvent::ReportError(
                daruda_store::observability::error_report::ErrorReport::new(
                    crate::strings::agent_chat::attachment_unsupported(),
                )
                .at(file!(), line!())
                .dedup("agent_chat.attachment.unsupported")
                .build(),
            ));
            return None;
        }
        let qp = self.queue.pending_prompts.remove(0);
        // If the drained entry was the one being edited, drop the stale editing
        // flag so a later send doesn't try to replace an id that is no longer
        // queued.
        if self.queue.editing_prompt == Some(qp.id) {
            self.queue.editing_prompt = None;
        }
        Some(qp)
    }

    fn activate_queued_prompt(&mut self, prompt: &QueuedPrompt, cx: &mut Context<Self>) {
        self.start_dispatched_turn();
        self.echo_attached_prompt(prompt.text.clone(), &prompt.attachments, cx);
        self.arm_phone_turn_if(prompt.origin);
    }

    /// Send the next buffered prompt iff connected and idle. Pops the FRONT of
    /// the queue (FIFO), forwards it, marks the turn in flight, and echoes it
    /// (it was NOT echoed when buffered). Drains one prompt per
    /// turn-completion, so the view never tracks more than one turn at a time.
    pub(super) fn pump_pending_prompt(&mut self, cx: &mut Context<Self>) {
        // Hold the queue until the session is fully connected and while a cancel
        // is still outstanding (`cancel_in_flight`): a handle exists before the
        // ACP handshake/load has completed, but prompt delivery is only safe once
        // `Connected` has opened the session's prompt loop.
        if !matches!(self.status, AgentSessionStatus::Connected) || self.live_handle().is_none() {
            return;
        };
        let Some(drained) = self.take_next_queued_prompt(cx) else {
            return;
        };
        // The wire gets the briefed form; echo only after the handoff succeeds.
        let wire = self.wire_text(&drained.text);
        if !self.live_handle().is_some_and(|handle| {
            handle
                .try_send_prompt_with_attachments(wire, drained.attachments.clone())
                .is_ok()
        }) {
            self.fail_prompt_handoff(drained.task, cx);
            return;
        }
        self.activate_queued_prompt(&drained, cx);
        self.report_sent(drained.task, cx);
        cx.notify();
    }

    fn fail_prompt_handoff(&mut self, task: Option<ExecutionRef>, cx: &mut Context<Self>) {
        let mut tasks: Vec<_> = task.into_iter().collect();
        tasks.extend(self.queue.discard_all());
        self.report_dropped(tasks, PromptDropReason::StartFailed, cx);
        self.detach_handle();
        self.set_error(
            crate::strings::agent_chat::transport_closed(),
            daruda_acp::Remedy::Retry,
            cx,
        );
    }

    /// What actually goes on the wire for `text`.
    ///
    /// Equal to `text` for every pane but a briefed one, where the session's
    /// **first** prompt carries the briefing in front of it and the transcript
    /// keeps showing only what the person typed. The person never wrote it and
    /// would have to scroll past it on every glance; the agent needs it once.
    ///
    /// One-shot by construction: the briefing is taken, so a second prompt
    /// cannot repeat it and no call site has to track whether it already ran.
    fn wire_text(&mut self, text: &str) -> String {
        match self.briefing.take() {
            Some(briefing) => format!("{briefing}\n\n---\n\n{text}"),
            None => text.to_owned(),
        }
    }

    /// Arm the one-shot briefing. Called between inserting the orchestrator's
    /// pane and revealing it, which is what starts the session.
    pub fn set_briefing(&mut self, briefing: String) {
        self.briefing = Some(briefing);
    }

    #[cfg(test)]
    pub(super) fn wire_text_for_test(&mut self, text: &str) -> String {
        self.wire_text(text)
    }

    /// Seed a queue a Stop parked, optionally with the resume gesture already
    /// armed. Goes through the real enqueue + park move so the capture shows
    /// what the Stop path produces, not a hand-built queue.
    #[cfg(feature = "screenshot")]
    pub fn seed_parked_queue_for_shot(&mut self, armed: bool, cx: &mut Context<Self>) {
        for text in super::super::shot_transcript::parked_prompts() {
            self.enqueue_prompt(text, PromptOrigin::InApp);
        }
        self.queue
            .paused_prompts
            .append(&mut self.queue.pending_prompts);
        if armed {
            self.handle_empty_submit(cx);
        }
        cx.notify();
    }

    /// Resolve what an empty-composer submit should do and apply it: with a
    /// parked queue, the first call arms the resume gesture and the second
    /// performs it. Paired with [`Self::handle_escape`], which discards the
    /// same queue.
    pub fn handle_empty_submit(&mut self, cx: &mut Context<Self>) -> EmptySubmitOutcome {
        if self.queue.paused_prompts.is_empty() {
            return EmptySubmitOutcome::Ignored;
        }
        if self.queue.resume_armed {
            // `resume_queue` empties the parked queue and drops the flag.
            self.resume_queue(cx);
            return EmptySubmitOutcome::Resumed;
        }
        self.queue.resume_armed = true;
        cx.notify();
        EmptySubmitOutcome::Armed
    }

    /// Whether a second empty Enter would resume the parked queue — what the
    /// strip renders its confirmation prompt from.
    ///
    /// INVARIANT: the flag alone is not the answer. Requiring a non-empty
    /// parked queue here means a queue emptied under an armed gesture (× on
    /// the last row, clear-all, a drain) can never be observed as armed, so
    /// the invariant lives on this one read instead of at every mutation site.
    pub fn resume_armed(&self) -> bool {
        self.queue.resume_armed && !self.queue.paused_prompts.is_empty()
    }

    /// Drop the armed resume gesture — the composer changed, so the next Enter
    /// is a new prompt rather than a confirmation. No-op when not armed.
    pub fn disarm_resume(&mut self, cx: &mut Context<Self>) {
        if self.queue.resume_armed {
            self.queue.resume_armed = false;
            cx.notify();
        }
    }

    /// Resolve what Escape should do and apply it, in priority order:
    /// 1. In-flight turn → cancel + park the queue.
    /// 2. Else parked queue → discard it (Esc-twice-clears gesture; takes
    ///    precedence over a trailing subagent's quiescence window).
    /// 3. Else a running background subagent → cancel it.
    /// 4. Else nothing to do → propagate Escape.
    pub fn handle_escape(&mut self, cx: &mut Context<Self>) -> EscapeOutcome {
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
