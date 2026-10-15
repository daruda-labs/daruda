//! The `Workspace` side of [`AgentChatEvent`]. One subscription per view, made
//! where the view is built, so every chat — a tab's or the orchestrator's — is
//! heard exactly once however often a tab re-wraps it.

use gpui::{App, Context, Entity, Window};

use crate::workspace::main_area::pane_menu::ResourceRightClick;

use crate::workspace::Workspace;
use crate::workspace::main_area::pane_tree::PaneId;
use daruda_chat::view::{AgentChatEvent, AgentChatView};

impl Workspace {
    pub(super) fn subscribe_agent_chat(
        &mut self,
        pane_id: PaneId,
        view: &Entity<AgentChatView>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        App::subscribe(cx, view, |_, event: &AgentChatEvent, cx| {
            record_task_delivery(event, cx)
        })
        .detach();
        let pane = self.run_pane(pane_id);
        view.update(cx, |_, cx| {
            cx.on_release(move |_, cx| {
                crate::runs::RunsHost::release_execution(
                    pane,
                    daruda_store::tasks::machine::UndeliveredReason::PaneClosed,
                    cx,
                )
            })
            .detach();
        });
        // Detached: gpui drops the handler once either side is released, and a
        // chat never changes window, so the Workspace that built it is its host
        // for life. The view is owned by that Workspace, so no event can
        // outlive the host it would have gone to.
        cx.subscribe_in(view, window, move |ws, _, event, window, cx| {
            ws.on_agent_chat_event(pane_id, event, window, cx)
        })
        .detach();
    }

    fn on_agent_chat_event(
        &mut self,
        pane_id: PaneId,
        event: &AgentChatEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            AgentChatEvent::ReportError(report) => self.report_error(report.clone(), cx),
            AgentChatEvent::PrefsChanged => self.mutate_durable(cx, |_, _| {}),
            AgentChatEvent::RetryConnect => self.retry_agent_chat_connect(pane_id, cx),
            AgentChatEvent::Reauthenticate => self.reauthenticate_pane_account(pane_id, cx),
            AgentChatEvent::OpenLink(target) => {
                self.open_link_target(pane_id, target.clone(), window, cx);
            }
            AgentChatEvent::OpenDiffInFileView(path) => {
                self.open_diff_in_file_view(pane_id, path.clone(), window, cx);
            }
            AgentChatEvent::OpenFileExternally(path) => {
                self.open_pane_file_externally(pane_id, path.clone(), cx);
            }
            AgentChatEvent::ResourceRightClicked {
                position,
                uri,
                mime,
            } => self.record_resource_right_click(ResourceRightClick {
                position: *position,
                uri: uri.clone(),
                mime: mime.clone(),
            }),
            AgentChatEvent::PromptSent { .. } | AgentChatEvent::PromptsDropped { .. } => {}
        }
    }
}

fn record_task_delivery(event: &AgentChatEvent, cx: &mut App) {
    match event {
        AgentChatEvent::PromptSent { task, session_id } => {
            crate::runs::RunsHost::prompt_sent(task.clone(), session_id.clone(), cx)
        }
        AgentChatEvent::PromptsDropped { tasks, reason } => {
            use daruda_chat::view::PromptDropReason;
            use daruda_store::tasks::machine::UndeliveredReason;
            let reason = match reason {
                PromptDropReason::StartFailed => UndeliveredReason::StartFailed,
                PromptDropReason::ConnectFailed => UndeliveredReason::ConnectFailed,
                PromptDropReason::SessionTornDown => UndeliveredReason::SessionTornDown,
                PromptDropReason::RemovedByUser => UndeliveredReason::RemovedByUser,
            };
            for exec in tasks {
                crate::runs::RunsHost::prompt_dropped(exec.clone(), reason, cx);
            }
        }
        _ => {}
    }
}
