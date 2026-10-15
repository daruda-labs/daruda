//! Inbound Telegram relay: phone-relayed replies and permission decisions
//! routed back into the triggering pane.

use crate::telegram::bridge::BotPermissionOutcome;
use crate::workspace::Workspace;
use crate::workspace::main_area::pane_tree::PaneId;
use daruda_chat::view::PromptDispatch;
use gpui::Context;

impl Workspace {
    /// Inject a phone-relayed reply as a prompt in this pane through the same
    /// Workspace submit funnel as the bottom-dock composer. If it has to queue
    /// behind an in-flight turn, sends the immediate "queued" notice (there's
    /// nothing yet to watch a first response for); otherwise it dispatches
    /// straight onto the wire and the view itself arms the first-response
    /// watch. Local-only slash commands never reach the agent, so they receive
    /// the fixed fallback ack immediately. A `pub(crate)` entry point for
    /// `crate::telegram::global`'s poll loop to call into (which lives outside
    /// `workspace/` and can't reach the `pub(super)` version).
    /// `false` means the pane is gone. Reported rather than swallowed: the
    /// caller resolves the target from a *remembered* selection or last-pinged
    /// pane, either of which can name a pane the user has since closed — and a
    /// silent return there loses the message and leaves the same dead target in
    /// place for every message after it.
    pub(crate) fn inject_bot_reply(
        &mut self,
        pane_id: PaneId,
        text: String,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.agent_chat_view(pane_id).is_none() {
            return false;
        }
        match self.send_agent_prompt_text_from_telegram(pane_id, text, cx) {
            Some(PromptDispatch::Queued) => self.relay_queued_notice_to_telegram(pane_id, cx),
            // Reported, not lost: the sender is on a phone and would otherwise
            // read the silence as the prompt having been accepted.
            Some(PromptDispatch::QueueFull) => {
                self.relay_queue_full_notice_to_telegram(pane_id, cx)
            }
            Some(PromptDispatch::SentNow) => {}
            Some(PromptDispatch::ReadOnly | PromptDispatch::Unavailable) => return false,
            None => self.relay_first_response_fallback_to_telegram(pane_id, cx),
        }
        true
    }

    /// Resolve a phone-tapped Allow/Reject button against this pane's
    /// currently-outstanding permission. Delegates to the existing
    /// `AgentChatView::respond_permission`, which already: resolves
    /// the card carrying this `perm_id`, sends the decision over the ACP
    /// session, and reflows the row list — the same path the in-app
    /// buttons use.
    ///
    /// The bridge's `perm_id` (captured when the ping was built) must still be
    /// outstanding. A stale phone button should not resolve a request the user
    /// already answered in-app or that was cancelled.
    pub(crate) fn respond_bot_permission(
        &mut self,
        pane_id: PaneId,
        perm_id: u64,
        decision: crate::telegram::bridge::PermissionDecision,
        cx: &mut Context<Self>,
    ) -> BotPermissionOutcome {
        let Some(view) = self.agent_chat_view(pane_id).cloned() else {
            return BotPermissionOutcome::Gone;
        };
        if !view.read(cx).is_permission_outstanding(perm_id) {
            return BotPermissionOutcome::Stale;
        }
        let (option_id, kind) = match decision {
            crate::telegram::bridge::PermissionDecision::Allow(id) => {
                (id, daruda_acp::PermissionKindView::AllowOnce)
            }
            crate::telegram::bridge::PermissionDecision::Reject(id) => {
                (id, daruda_acp::PermissionKindView::RejectOnce)
            }
        };
        view.update(cx, |v, cx| {
            v.respond_permission(perm_id, option_id, kind, cx)
        });
        BotPermissionOutcome::Applied
    }
}
