use crate::remote_channel::bridge::{FlowAskPrompt, PermissionDecision};
use crate::remote_channel::cards::CardId;
use crate::remote_channel::global::RemoteChannels;
use crate::runs::{FlowAskOutcome, FlowChange, FlowChanged, RunsHost};
use crate::surface::strings as s;
use crate::telegram::global::TelegramBridge;
use crate::window_registry::WindowRegistry;
use gpui::App;

pub(super) fn deliver(change: &FlowChange, cx: &mut App) {
    match &change.what {
        FlowChanged::Asked {
            run_dir,
            ask_id,
            submitted_by,
            solicited,
            ..
        } => {
            let Some(ask) = RunsHost::flow_run(change.worktree.as_path(), cx)
                .filter(|run| run.view().run_dir() == run_dir)
                .and_then(|run| run.view().question(*ask_id))
                .filter(|ask| !ask.is_closed())
            else {
                return;
            };
            let summary = match ask.detail() {
                Some(detail) => format!("{}\n{detail}", super::flow_runs::describe_question(ask)),
                None => super::flow_runs::describe_question(ask),
            };
            let prompt = FlowAskPrompt {
                worktree: change.worktree.as_path().to_path_buf(),
                run_dir: run_dir.clone(),
                ask_id: *ask_id,
                summary,
                buttons: ask
                    .options()
                    .iter()
                    .map(|choice| {
                        let decision = match choice.kind {
                            daruda_acp::PermissionKindView::AllowOnce
                            | daruda_acp::PermissionKindView::AllowAlways => {
                                PermissionDecision::Allow(choice.option_id.clone())
                            }
                            daruda_acp::PermissionKindView::RejectOnce
                            | daruda_acp::PermissionKindView::RejectAlways => {
                                PermissionDecision::Reject(choice.option_id.clone())
                            }
                        };
                        (choice.name.clone(), decision)
                    })
                    .collect(),
            };
            let delivery = crate::remote_channel::delivery::remote_delivery(
                *submitted_by == daruda_flow::request::StartedBy::Phone || *solicited,
                crate::app_presence::is_away(cx),
            );
            RemoteChannels::send_flow_ask(prompt.clone(), delivery, cx);
            TelegramBridge::send_flow_ask(prompt, delivery, cx);
            notify_waiting(change, cx);
        }
        FlowChanged::AskClosed {
            run_dir,
            ask_id,
            outcome,
        } => {
            let label = match outcome {
                FlowAskOutcome::Answered(daruda_acp::PermissionDecision::Allow { .. }) => {
                    s::notification::telegram_permission_allowed()
                }
                FlowAskOutcome::Answered(daruda_acp::PermissionDecision::Reject { .. }) => {
                    s::notification::telegram_permission_rejected()
                }
                FlowAskOutcome::Answered(daruda_acp::PermissionDecision::Cancelled)
                | FlowAskOutcome::Withdrawn => s::flow::ask_withdrawn(),
            };
            let card = CardId::FlowAsk {
                run_dir: run_dir.clone(),
                ask_id: *ask_id,
            };
            RemoteChannels::settle_card(&card, &label, cx);
            TelegramBridge::settle_card(&card, &label, cx);
        }
        _ => {}
    }
}

fn notify_waiting(change: &FlowChange, cx: &mut App) {
    let config = &crate::settings_store::SettingsStore::global(cx)
        .user()
        .notifications;
    if !config.agent_waiting_enabled {
        return;
    }
    let skip_in_view = config.skip_focused_pane;
    let raises_modal = matches!(change.what, FlowChanged::Asked { alone: true, .. });
    let mut in_view = false;
    WindowRegistry::for_each_workspace(cx, |ws, window, cx| {
        in_view |= window.is_window_active()
            && ws.lanes_at(&change.worktree, cx).into_iter().any(|lane| {
                question_in_view(ws.shows_flows_of(lane), lane == ws.active, raises_modal)
            });
    });
    if !super::main_area::agent_chat_host::agent_chat_ops::should_notify_agent_event(
        true,
        skip_in_view,
        crate::platform::attention::is_app_active(),
        in_view,
    ) {
        return;
    }
    let title = s::flow::ask_modal_title();
    let body = s::notification::agent_waiting();
    if let Some(pane) = crate::orchestrator::pane(cx) {
        crate::platform::notifications::show_for(
            &title,
            &body,
            crate::platform::notifications::Target {
                workspace: pane.workspace,
                pane: pane.pane,
            },
        );
    } else {
        crate::platform::notifications::show(&title, &body);
    }
}

fn question_in_view(showing_flows: bool, active_lane: bool, raises_modal: bool) -> bool {
    showing_flows || (active_lane && raises_modal)
}

#[cfg(test)]
mod tests;
