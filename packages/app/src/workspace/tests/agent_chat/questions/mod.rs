//! Who a mid-turn question is shown to: the desktop form, and the phone's
//! card for a turn the phone started.

mod composer;
mod keys;
mod notify;
mod timeout;

use gpui::{AppContext as _, TestAppContext};

use super::super::build_workspace_with;
use crate::workspace::main_area::pane::TabEntry;
use crate::workspace::main_area::pane_tree::{PaneId, PaneLayout};
use daruda_acp::{
    AcpEvent, AcpSessionHandle, QuestionField, QuestionFieldKind, QuestionOption, QuestionRequest,
};
use daruda_chat::view::AgentSessionStatus;

fn question() -> QuestionRequest {
    QuestionRequest {
        message: "Which color?".into(),
        tool_call_id: None,
        delivery: daruda_acp::QuestionDelivery::Blocking,
        fields: vec![QuestionField {
            key: "question_0".into(),
            title: None,
            description: None,
            required: false,
            kind: QuestionFieldKind::SingleSelect(vec![QuestionOption {
                value: "red".into(),
                label: "red".into(),
                description: None,
            }]),
            companion_of: None,
            default: None,
        }],
    }
}

/// A connected chat pane whose session has question 1 parked, the turn in
/// flight and, when `phone`, started from the phone.
fn asked(
    phone: bool,
    cx: &mut TestAppContext,
) -> (
    tempfile::TempDir,
    gpui::Entity<crate::workspace::Workspace>,
    PaneId,
    daruda_acp::QuestionProbe,
) {
    let dir = tempfile::tempdir().unwrap();
    let project = daruda_store::project::Project::from_path(dir.path());
    let (wh, ws) = build_workspace_with(cx, &daruda_config::Config::default(), Some(project));
    cx.run_until_parked();
    let pane = cx
        .update_window(wh.into(), |_, window, cx| {
            ws.update(cx, |ws, cx| {
                let pane = ws.create_agent_chat_pane(
                    None,
                    None,
                    daruda_config::AgentDefinition::claude_default().id,
                    None,
                    window,
                    cx,
                );
                let pane_id = pane.id;
                ws.active_runtime_mut().panes.push(pane);
                let tab_id = ws.alloc_id();
                ws.active_runtime_mut().tabs.push(TabEntry {
                    id: tab_id,
                    layout: PaneLayout::Pane(pane_id),
                    last_focused_pane: pane_id,
                    user_label: None,
                });
                pane_id
            })
        })
        .unwrap();
    let (handle, _probe) = AcpSessionHandle::detached_for_test();
    let answer = handle.park_question_for_test(1);
    ws.update(cx, |ws, cx| {
        let view = ws.agent_chat_view(pane).expect("chat pane").clone();
        view.update(cx, |v, _| {
            v.set_status_for_test(AgentSessionStatus::Connected);
            v.attach_handle(handle);
            v.set_turn_in_flight();
            if phone {
                v.start_phone_turn_for_test(std::time::Instant::now());
            }
        });
        ws.fold_agent_chat_event(
            pane,
            AcpEvent::QuestionAsked {
                id: 1,
                question: question(),
            },
            cx,
        );
    });
    (dir, ws, pane, answer)
}

fn pending(
    ws: &gpui::Entity<crate::workspace::Workspace>,
    pane: PaneId,
    cx: &mut TestAppContext,
) -> bool {
    ws.read_with(cx, |ws, cx| {
        ws.agent_chat_view(pane)
            .expect("chat pane")
            .read(cx)
            .has_pending_question()
    })
}

#[gpui::test]
fn a_desktop_turn_shows_the_question(cx: &mut TestAppContext) {
    let (_dir, ws, pane, mut answer) = asked(false, cx);
    assert!(pending(&ws, pane, cx));
    assert_eq!(answer.answer(), None, "it waits for the person");
}

/// A phone-started turn keeps its question: the relay sends it to the phone
/// as a card, and the card is the turn's first answer there, so no fallback
/// ack follows it.
#[gpui::test]
fn a_phone_turn_keeps_the_question_for_the_phone(cx: &mut TestAppContext) {
    let (_dir, ws, pane, mut answer) = asked(true, cx);
    assert!(pending(&ws, pane, cx));
    assert_eq!(answer.answer(), None, "it waits for the phone");
    ws.read_with(cx, |ws, cx| {
        let view = ws.agent_chat_view(pane).expect("chat pane").read(cx);
        assert!(view.phone_owns_turn(), "the turn is still the phone's");
        assert!(
            !view.is_phone_turn_waiting(),
            "the question answered the wait"
        );
    });
}
