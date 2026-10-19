//! Deliver literal run pointers without touching composer state or phone routing.
use super::Workspace;
use crate::control::notices;
use daruda_chat::view::{ActivityState, PromptDispatch};
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
        let Some(batch) = notices::batch(cx) else {
            return;
        };
        let dispatch = view.update(cx, |view, cx| {
            view.send_internal_prompt_text(batch.prompt.clone(), cx)
        });
        match dispatch {
            PromptDispatch::SentNow | PromptDispatch::Queued => {
                notices::accept(batch, cx);
                self.reconcile_agent_chat_activity(pane, cx);
            }
            failure => notices::log_failure(&format!("{failure:?}")),
        }
    }
}
#[cfg(test)]
#[path = "tests/run_notices.rs"]
mod tests;
