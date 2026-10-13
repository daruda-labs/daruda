//! Desktop notifications for agent-chat panes and the busy-to-idle completion
//! side effects, including answering a waiting ask.

use super::should_notify_agent_event;
use crate::surface::strings as s;
use crate::workspace::Workspace;
use crate::workspace::main_area::pane_tree::PaneId;
use daruda_chat::view::TurnOutcome;
use gpui::Context;

impl Workspace {
    /// Show a desktop notification `body` for `pane_id`, gated by `enabled`
    /// and the shared in-view rule. A parked-lane pane is never on screen, so
    /// its completion/wait always fires.
    fn notify_agent_pane(&self, pane_id: PaneId, enabled: bool, body: String, cx: &Context<Self>) {
        if !should_notify_agent_event(
            enabled,
            self.mirrors.notifications.skip_focused_pane,
            crate::platform::attention::is_app_active(),
            self.pane_on_screen(pane_id),
        ) {
            return;
        }
        crate::platform::notifications::show_for(
            &self.pane_title(pane_id, cx),
            &body,
            crate::platform::notifications::Target {
                workspace: self.uuid(),
                pane: pane_id,
            },
        );
    }

    /// The pane's display title for a notification: its live session title, or
    /// the static tab-title fallback before the session reports one. Stays
    /// `pub(super)` because `telegram_ops` also reads it.
    pub(in crate::workspace::main_area::agent_chat_host) fn pane_title(
        &self,
        pane_id: PaneId,
        cx: &Context<Self>,
    ) -> String {
        self.agent_chat_view(pane_id)
            .and_then(|v| v.read(cx).session_title().map(str::to_owned))
            .unwrap_or_else(s::agent_chat::tab_title)
    }

    /// Fire the "waiting for input" notification on a permission request.
    /// Called from the event pump BEFORE folding into the view — distinct
    /// from turn completion, which fires only via [`Self::fire_activity_completion`].
    pub(in crate::workspace::main_area::agent_chat_host) fn maybe_notify_agent_event(
        &mut self,
        pane_id: PaneId,
        event: &daruda_acp::AcpEvent,
        cx: &mut Context<Self>,
    ) {
        let daruda_acp::AcpEvent::PermissionRequested { id, request } = event else {
            return;
        };
        self.notify_agent_pane(
            pane_id,
            self.mirrors.notifications.agent_waiting_enabled,
            s::notification::agent_waiting(),
            cx,
        );
        // Reuse `apply_event`'s protocol→view-model conversion so this relay
        // never names a raw ACP type (daruda_acp's "host never touches protocol
        // types" boundary). `permission_item` always returns `Permission` for a
        // permission request; the `let else` is defensive. Passing the view's
        // current items lets it prefer an already-clean `raw_input` (from an
        // earlier tool_call for this id) over the request's own copy.
        let Some(view) = self.agent_chat_view(pane_id).cloned() else {
            return;
        };
        let daruda_acp::ChatItem::Permission(card) =
            daruda_acp::permission_item(*id, request, view.read(cx).items())
        else {
            return;
        };
        self.relay_permission_wait_to_telegram(pane_id, &card, cx);
    }

    /// Fire the "completed" notification. Called only from
    /// [`Self::fire_activity_completion`] on the busy→idle settle edge, not
    /// the raw `TurnEnded` (which may still have trailing subagents running).
    fn maybe_notify_agent_completed(&self, pane_id: PaneId, cx: &Context<Self>) {
        self.notify_agent_pane(
            pane_id,
            self.mirrors.notifications.agent_completion_enabled,
            s::notification::agent_completed(),
            cx,
        );
    }

    /// Fire the completion signals once a pane's activity span settles: the
    /// "completed" notification (only for `Completed`) and the backing task's
    /// terminal reconcile. The single completion firing point.
    pub(in crate::workspace) fn fire_activity_completion(
        &mut self,
        pane_id: PaneId,
        outcome: TurnOutcome,
        cx: &mut Context<Self>,
    ) {
        if let Some(news) = super::super::lane_status::agent_outcome(&outcome) {
            self.record_agent_outcome(pane_id, news, cx);
        }
        if matches!(outcome, TurnOutcome::Completed) {
            self.maybe_notify_agent_completed(pane_id, cx);
            // `None` = the phone already has this turn's answer; the
            // first-response relay sent the very message this would report.
            if let Some((header, tail)) = self.telegram_completion_parts(pane_id, cx) {
                self.relay_when_presence_allows(pane_id, header, tail, None, cx);
            }
        }
        if matches!(outcome, TurnOutcome::Errored) {
            let (header, tail) = self.telegram_failure_parts(pane_id, cx);
            self.relay_when_presence_allows(pane_id, header, tail, None, cx);
        }
        // Every outcome, not just `Completed`: a turn that errored or was
        // stopped has had its phone conversation ended too, and a ledger left
        // behind outlives the turn it describes — the next turn on this pane
        // is not the phone's, so nothing re-arms it, and its completion would
        // be measured against a message from a turn that is long gone.
        self.close_phone_turn(pane_id, cx);
        let reason = match outcome {
            TurnOutcome::Completed | TurnOutcome::Stopped => {
                daruda_store::tasks::SessionEndReason::Stop
            }
            TurnOutcome::Errored => daruda_store::tasks::SessionEndReason::Error,
        };
        self.apply_agent_chat_task_ended(pane_id, reason, cx);
        // Last, because handing `outcome` on moves it — the matches above
        // bind nothing and so only read it.
        self.answer_waiting_ask(pane_id, outcome, cx);
    }

    /// Answer a `daruda_chat_ask` waiting on this pane, if one is.
    ///
    /// Teed off [`Self::fire_activity_completion`] rather than an
    /// `AcpEvent::TurnEnded` arm, for the reason the notification is: that
    /// event settles before trailing subagents finish, so a call answered
    /// there would report the turn done while work was still running
    /// (AGENTS.md pitfall 11).
    ///
    /// A pane nobody asked about is the common case, and `resolve` says so by
    /// finding nothing.
    fn answer_waiting_ask(
        &mut self,
        pane_id: PaneId,
        outcome: TurnOutcome,
        cx: &mut Context<Self>,
    ) {
        let target = crate::telegram::bridge::PaneRef {
            workspace: self.uuid(),
            pane: pane_id,
        };
        if !crate::control::ask::has_waiter(target, cx) {
            return;
        }
        // The same bounded read `daruda_chat_read` answers with, so the two
        // tools cannot disagree about what the agent said.
        let said = self.control_read(pane_id, cx).ok().flatten();
        // `Stopped` is not an answer. `settle_run_state` finalises whatever was
        // streaming, so the transcript *will* have text — handing it back as
        // the reply would present a sentence somebody cut off as what the
        // agent said.
        let answer = match outcome {
            TurnOutcome::Errored => crate::control::result::PaneAnswer::Failed,
            TurnOutcome::Stopped => crate::control::result::PaneAnswer::Interrupted,
            TurnOutcome::Completed => match said {
                Some(text) => crate::control::result::PaneAnswer::Text { text },
                None => crate::control::result::PaneAnswer::NoAnswer,
            },
        };
        crate::control::ask::resolve(target, answer, cx);
    }
}
