//! `Workspace` ops for the Agent chat pane's prompt queue: send / queue /
//! edit / cancel routing for the bottom-dock composer and the queued-prompt
//! strip, plus the user-cancel (Stop) path. Split out of
//! [`super::agent_chat_ops`] (which keeps notification, pane construction,
//! mode/config, and misc accessors) since the queue is its own cohesive
//! concern with a shared shape (one prompt in flight at a time).

use gpui::{Context, Window};

use super::slash_dispatch::{LocalSlashCommand, SlashDispatch, classify_slash};
use super::view::{
    EscapeOutcome, PromptDispatch, PromptId, PromptOrigin, TelegramFirstResponseEffect,
};
use crate::workspace::Workspace;
use crate::workspace::main_area::pane_tree::PaneId;

impl Workspace {
    /// Send `text` as a prompt to an Agent chat pane. Shim for the bottom-dock
    /// input: routes into the view, which echoes the prompt locally, forwards it
    /// over the session, and marks a turn in flight.
    pub(in crate::workspace) fn send_agent_prompt_text(
        &mut self,
        pane_id: PaneId,
        text: String,
        cx: &mut Context<Self>,
    ) {
        let _ = self.send_agent_prompt_text_with_origin(pane_id, text, PromptOrigin::InApp, cx);
    }

    pub(super) fn relay_telegram_first_response_effect(
        &self,
        pane_id: PaneId,
        effect: TelegramFirstResponseEffect,
        cx: &Context<Self>,
    ) {
        match effect {
            TelegramFirstResponseEffect::None => {}
            TelegramFirstResponseEffect::Relay(outcome) => {
                self.relay_first_response_to_telegram(pane_id, outcome, cx);
            }
            TelegramFirstResponseEffect::Fallback => {
                self.relay_first_response_fallback_to_telegram(pane_id, cx);
            }
        }
    }

    /// Telegram-specific submit shim. It uses the same Workspace funnel as the
    /// bottom-dock composer (slash handling, post-turn flush, activity reconcile)
    /// but preserves the dispatch result so `inject_bot_reply` knows whether the
    /// reply reached the agent now or only entered the pane queue.
    pub(in crate::workspace) fn send_agent_prompt_text_from_telegram(
        &mut self,
        pane_id: PaneId,
        text: String,
        cx: &mut Context<Self>,
    ) -> Option<PromptDispatch> {
        self.send_agent_prompt_text_with_origin(pane_id, text, PromptOrigin::Telegram, cx)
    }

    fn send_agent_prompt_text_with_origin(
        &mut self,
        pane_id: PaneId,
        text: String,
        origin: PromptOrigin,
        cx: &mut Context<Self>,
    ) -> Option<PromptDispatch> {
        let dispatch = classify_slash(&text);
        match dispatch {
            SlashDispatch::Local(LocalSlashCommand::Clear) => {
                self.reset_agent_chat_session(pane_id, cx);
                None
            }
            SlashDispatch::Forward => {
                let view = self.agent_chat_view(pane_id).cloned()?;
                // Flush a not-yet-quiesced post-turn follow-up before this new
                // turn subsumes it (else its delta would be lost/merged).
                if let Some(delta) = view.update(cx, |v, _| v.take_pending_post_turn()) {
                    self.relay_post_turn_to_telegram(pane_id, delta, cx);
                }
                let prompt_dispatch = if origin == PromptOrigin::Telegram {
                    view.update(cx, |v, cx| v.send_prompt_text_for_telegram(text, cx))
                } else {
                    view.update(cx, |v, cx| v.send_prompt_text(text, cx));
                    PromptDispatch::SentNow
                };
                // Open the activity span on the idle→busy edge (stamps the
                // working-indicator elapsed anchor at send). A returned
                // `Some` is unexpected on open but harmless to fire — the
                // single completion firing point stays consistent.
                let edge = view.update(cx, |v, _| v.reconcile_activity(std::time::Instant::now()));
                if let Some(outcome) = edge {
                    self.fire_activity_completion(pane_id, outcome, cx);
                }
                Some(prompt_dispatch)
            }
        }
    }

    /// Remove a single queued prompt from an Agent chat pane. Shim for the
    /// bottom-dock queued-prompt strip's per-item × button: routes into the
    /// view, which drops the entry and notifies (one-way data flow). No-op when
    /// `pane_id` is gone or is not an Agent chat pane.
    pub(in crate::workspace) fn remove_queued_prompt(
        &mut self,
        pane_id: PaneId,
        id: PromptId,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.agent_chat_view(pane_id).cloned() {
            view.update(cx, |v, cx| v.remove_queued(id, cx));
        }
    }

    /// Clear every queued prompt from an Agent chat pane. Shim for the
    /// bottom-dock queued-prompt strip's "clear all" button: routes into the
    /// view, which empties the queue and notifies (one-way data flow). No-op
    /// when `pane_id` is gone or is not an Agent chat pane.
    ///
    /// If a queued prompt was being edited when the queue is cleared, the
    /// composer still holds that (now-deleted) slot's text; empty it too so it
    /// doesn't linger as a phantom draft — mirrors `cancel_edit_queued_prompt`.
    pub(in crate::workspace) fn clear_queued_prompts(
        &mut self,
        pane_id: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.agent_chat_view(pane_id).cloned() else {
            return;
        };
        let was_editing = view.read(cx).queue.editing_prompt.is_some();
        view.update(cx, |v, cx| v.clear_queue(cx));
        if was_editing {
            self.terminal_input
                .update(cx, |s, cx_state| s.set_value("", window, cx_state));
        }
    }

    /// Begin editing a queued prompt: pull its text into the bottom-dock
    /// composer (cursor at end) and mark the view editing that slot. Shim for
    /// the queued-prompt strip's ✎ button and for ↑ in an empty composer. On
    /// the next send, [`AgentChatView::send_prompt_text`] replaces the slot in
    /// place (order preserved). No-op when `pane_id` is gone, is not an Agent
    /// chat pane, or `id` is no longer queued.
    pub(in crate::workspace) fn begin_edit_queued_prompt(
        &mut self,
        pane_id: PaneId,
        id: PromptId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.agent_chat_view(pane_id).cloned() else {
            return;
        };
        // Reading the view entity here is safe — it is a different entity from
        // `self` (Workspace) and from `terminal_input`.
        let v = view.read(cx);
        let found = v
            .queue
            .pending_prompts
            .iter()
            .chain(v.queue.paused_prompts.iter())
            .find(|q| q.id == id)
            .map(|q| q.text.clone());
        let Some(text) = found else {
            return;
        };
        // Pull the text into the composer, cursor at end (mirrors
        // `do_history_navigate`). Separate `entity.update` from the view update
        // below — never nest two updates on entities in one call.
        self.terminal_input.update(cx, |s, cx_state| {
            s.set_value(&text, window, cx_state);
            s.move_cursor_to_end(cx_state);
        });
        view.update(cx, |v, cx| v.begin_edit(id, cx));
    }

    /// Cancel an in-progress queued-prompt edit: clear the view's editing flag
    /// and empty the composer. Shim for the strip's cancel (↩) button. No-op on
    /// the view side when `pane_id` is gone or is not an Agent chat pane; the
    /// composer is emptied regardless.
    pub(in crate::workspace) fn cancel_edit_queued_prompt(
        &mut self,
        pane_id: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.agent_chat_view(pane_id).cloned() {
            view.update(cx, |v, cx| v.cancel_edit(cx));
        }
        self.terminal_input
            .update(cx, |s, cx_state| s.set_value("", window, cx_state));
    }

    /// Resume a parked prompt queue that a Stop preserved. Shim for the
    /// bottom-dock queued-prompt strip's Resume button: routes into the view,
    /// which moves the parked prompts back to the front of the live queue and
    /// pumps the first one (one-way data flow). No-op when `pane_id` is gone,
    /// is not an Agent chat pane, or nothing is parked.
    pub(in crate::workspace) fn resume_queued_prompts(
        &mut self,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.agent_chat_view(pane_id).cloned() {
            view.update(cx, |v, cx| v.resume_queue(cx));
        }
    }

    /// Request cancellation of the active turn. Shim for the bottom-dock "Stop"
    /// button: routes into the view, which sends `session/cancel` and drains any
    /// pending permission request.
    pub(in crate::workspace) fn cancel_agent_turn(
        &mut self,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.agent_chat_view(pane_id).cloned() {
            view.update(cx, |v, cx| v.cancel_turn(cx));
            // `cancel_turn` settles the turn locally, so this reconcile hits the
            // busy→idle edge and fires the stashed outcome (a live-turn Stop's
            // `Stopped`, or a trailing-subagent Stop's preserved completion) via
            // the single completion firing point.
            let edge = view.update(cx, |v, _| v.reconcile_activity(std::time::Instant::now()));
            if let Some(outcome) = edge {
                self.fire_activity_completion(pane_id, outcome, cx);
            }
        }
    }

    /// Cancel `pane_id`'s turn only when the pane is actually busy (a prompt turn
    /// in flight OR a background subagent still running). Backs the Escape
    /// shortcut (the keyboard counterpart of the "Stop" button): returns `true`
    /// when it cancelled, `false` when `pane_id` is not an Agent chat pane or is
    /// idle — in which case the caller lets Escape propagate as usual. Mirrors
    /// the `agent_stop_pane` snapshot condition that shows the Stop button.
    pub(in crate::workspace) fn cancel_agent_turn_if_active(
        &mut self,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(view) = self.agent_chat_view(pane_id).cloned() else {
            return false;
        };
        // The pane's own state machine decides: cancel a running turn, discard a
        // parked queue (Esc twice), cancel a trailing subagent, or do nothing.
        match view.update(cx, |v, cx| v.handle_escape(cx)) {
            EscapeOutcome::Cancelled => {
                // Same settle-edge firing as `cancel_agent_turn`.
                let edge = view.update(cx, |v, _| v.reconcile_activity(std::time::Instant::now()));
                if let Some(outcome) = edge {
                    self.fire_activity_completion(pane_id, outcome, cx);
                }
                true
            }
            // Clearing a parked queue has no turn to complete; still handled, so
            // Escape is consumed rather than propagating.
            EscapeOutcome::ClearedQueue => true,
            // Nothing to act on — report not-handled so Escape keeps propagating.
            EscapeOutcome::Ignored => false,
        }
    }
}
