//! Deliver literal run pointers and preserve phone-solicited completion routing.
use super::Workspace;
use crate::control::notices;
use daruda_chat::view::{ActivityState, AgentChatView, InternalDispatch};
use gpui::Context;

impl Workspace {
    pub(crate) fn flush_run_notices(&mut self, cx: &mut Context<Self>) {
        let Some(chat) = &self.orchestrator.chat else {
            return;
        };
        let pane = chat.pane_id;
        let view = chat.view.clone();
        self.reconcile_agent_chat_activity(pane, cx);
        if view.read(cx).activity_state() != ActivityState::Idle {
            return;
        }
        let audience = notices::Audience::new(view.entity_id(), view.read(cx).session_id());
        let Some(batch) = notices::batch(audience, cx) else {
            return;
        };
        let dispatch = view.update(cx, |view, cx| {
            view.try_send_internal_prompt_text_with_phone_mode(
                batch.prompt.clone(),
                batch
                    .solicited
                    .then_some(daruda_chat::phone_turn::PhoneTurnMode::CompletionOnly),
                cx,
            )
        });
        match dispatch {
            InternalDispatch::Sent => {
                notices::accept(batch, cx);
                self.reconcile_agent_chat_activity(pane, cx);
            }
            InternalDispatch::NotNow => notices::retry_when(&view, can_take_notice_now, cx),
            failure => notices::log_failure(&format!("{failure:?}")),
        }
    }
}

#[cfg(test)]
impl Workspace {
    /// Give the orchestrator chat a live session, so a notice can be sent.
    pub(crate) fn connect_orchestrator_chat_for_test(
        &mut self,
        cx: &mut Context<Self>,
    ) -> daruda_acp::HandleProbe {
        let (handle, probe) = daruda_acp::AcpSessionHandle::detached_for_test();
        let view = &self
            .orchestrator
            .chat
            .as_ref()
            .expect("orchestrator chat")
            .view;
        view.update(cx, |view, cx| {
            view.set_status_for_test(daruda_chat::view::AgentSessionStatus::Connected);
            view.attach_handle(handle);
            cx.notify();
        });
        probe
    }

    /// Prompts the orchestrator chat has sent, notices included.
    pub(crate) fn orchestrator_sent_prompts_for_test(&self, cx: &gpui::App) -> usize {
        self.orchestrator.chat.as_ref().map_or(0, |chat| {
            chat.view
                .read(cx)
                .items()
                .iter()
                .filter(|item| matches!(item, daruda_acp::model::ChatItem::UserText(_)))
                .count()
        })
    }
}

/// A notice waits for a live, idle session with no user prompts ahead of it.
fn can_take_notice_now(view: &AgentChatView) -> bool {
    view.activity_state() == ActivityState::Idle && view.accepts_internal_prompt_now()
}

#[cfg(test)]
#[path = "tests/run_notices.rs"]
mod tests;
