use super::*;
use crate::remote_channel::bridge::{
    BridgePing, MessageTail, PermissionDecision, PermissionPromptRef,
};
use crate::remote_channel::cards::tests::EditCapture;
use crate::telegram::global::{TelegramBridge, TestOutbox};
use daruda_acp::{PermissionChoice, PermissionItem, PermissionKindView};
use gpui::TestAppContext;

fn permission(id: u64) -> PermissionItem {
    PermissionItem {
        id,
        tool_call_id: String::new(),
        tool_title: Some("Write file".into()),
        raw_input_summary: None,
        options: vec![
            PermissionChoice {
                option_id: "allow".into(),
                name: "Allow".into(),
                kind: PermissionKindView::AllowOnce,
            },
            PermissionChoice {
                option_id: "reject".into(),
                name: "Reject".into(),
                kind: PermissionKindView::RejectOnce,
            },
        ],
        resolved: None,
    }
}

fn send(
    pane: crate::remote_channel::bridge::PaneRef,
    session: u64,
    request: u64,
    outbox: &mut TestOutbox,
    cx: &mut App,
) -> impl FnOnce(&mut App) + use<> {
    cx.global::<TelegramBridge>().send(
        BridgePing {
            pane,
            header: "Project".into(),
            tail: MessageTail::Plain(format!("Permission {request}")),
            permission: Some(PermissionPromptRef {
                session,
                perm_id: request,
                buttons: vec![("Allow".into(), PermissionDecision::Allow("allow".into()))],
            }),
        },
        cx,
    );
    outbox
        .begin_send(i64::try_from(session * 1000 + request).unwrap(), cx)
        .unwrap()
}

#[gpui::test]
fn desktop_permission_and_stop_settle_the_phone_cards_through_host_events(cx: &mut TestAppContext) {
    let fixture = crate::test_support::workspace_with_agent_chat(cx);
    cx.run_until_parked();
    let pane = fixture.pane_ref(cx);
    let view = fixture
        .workspace
        .read_with(cx, |ws, _| ws.agent_chat_view(pane.pane).unwrap().clone());
    view.update(cx, |view, cx| {
        for id in 1..=3 {
            view.hold_permission_card_for_test(permission(id), cx);
        }
    });
    cx.update(|cx| {
        let mut outbox = crate::telegram::global::install_for_test(true, Some(42), cx);
        cx.set_global(EditCapture::default());
        let session = view.read(cx).permission_session();
        for id in 1..=3 {
            send(pane, session, id, &mut outbox, cx)(cx);
        }
    });
    view.update(cx, |view, cx| {
        view.respond_permission(1, "allow".into(), PermissionKindView::AllowOnce, cx)
    });
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(cx.global::<EditCapture>().edits.len(), 1);
        assert_eq!(
            cx.global::<EditCapture>().edits[0].label,
            crate::surface::strings::notification::telegram_permission_allowed()
        );
    });
    view.update(cx, |view, cx| view.cancel_turn(cx));
    cx.run_until_parked();
    cx.update(|cx| {
        let edits = &cx.global::<EditCapture>().edits;
        assert_eq!(edits.len(), 3);
        assert_eq!(
            edits
                .iter()
                .filter(|edit| edit.label
                    == crate::surface::strings::notification::telegram_permission_cancelled())
                .count(),
            2
        );
    });
}

#[gpui::test]
fn a_replaced_sessions_late_receipt_and_button_cannot_answer_its_reused_request_id(
    cx: &mut TestAppContext,
) {
    let fixture = crate::test_support::workspace_with_agent_chat(cx);
    cx.run_until_parked();
    let pane = fixture.pane_ref(cx);
    let view = fixture
        .workspace
        .read_with(cx, |ws, _| ws.agent_chat_view(pane.pane).unwrap().clone());
    view.update(cx, |view, cx| {
        view.hold_permission_card_for_test(permission(0), cx)
    });
    let old_session = view.read_with(cx, |view, _| view.permission_session());
    let (mut outbox, old_sent) = cx.update(|cx| {
        let mut outbox = crate::telegram::global::install_for_test(true, Some(42), cx);
        cx.set_global(EditCapture::default());
        let sent = send(pane, old_session, 0, &mut outbox, cx);
        (outbox, sent)
    });
    view.update(cx, |view, cx| {
        view.reset_for_new_session(cx);
        view.hold_permission_card_for_test(permission(0), cx);
    });
    cx.run_until_parked();
    let new_session = view.read_with(cx, |view, _| view.permission_session());
    assert_ne!(new_session, old_session);
    fixture.workspace.update(cx, |ws, cx| {
        assert_eq!(
            ws.respond_bot_permission(
                pane.pane,
                old_session,
                0,
                PermissionDecision::Allow("allow".into()),
                cx
            ),
            crate::remote_channel::bridge::BotPermissionOutcome::Stale
        );
    });
    assert!(view.read_with(cx, |view, _| view.is_permission_outstanding(0)));
    cx.update(|cx| {
        old_sent(cx);
        send(pane, new_session, 0, &mut outbox, cx)(cx);
    });
    view.update(cx, |view, cx| {
        view.respond_permission(0, "reject".into(), PermissionKindView::RejectOnce, cx)
    });
    cx.run_until_parked();
    cx.update(|cx| {
        let edits = &cx.global::<EditCapture>().edits;
        assert_eq!(edits.len(), 2);
        assert_eq!(
            edits[0].label,
            crate::surface::strings::notification::telegram_permission_cancelled()
        );
        assert_eq!(
            edits[1].label,
            crate::surface::strings::notification::telegram_permission_rejected()
        );
    });
}
