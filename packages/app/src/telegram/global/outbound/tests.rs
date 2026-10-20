use super::*;
use crate::telegram::global::{bot::Ownership, install_for_test};
use futures::FutureExt as _;
use std::ops::Not as _;

/// A different bot than `bot`, as a re-pairing to another token produces.
fn another_bot(bot: &ReadyBot) -> crate::telegram::keychain::TokenSnapshot {
    crate::telegram::keychain::TokenSnapshot {
        token: format!("{}-another", bot.token.token),
        revision: bot.token.revision,
    }
}

#[gpui::test]
fn a_telegram_flow_callback_keeps_original_options_and_never_adopts_a_target(
    cx: &mut gpui::TestAppContext,
) {
    use crate::remote_channel::bridge::{FlowAskPrompt, InboundAction, PermissionDecision, Routed};
    cx.update(|cx| {
        let mut outgoing = install_for_test(true, Some(42), cx);
        assert!(TelegramBridge::send_flow_ask(FlowAskPrompt {
            worktree: "/worktree/with/a/long/path".into(),
            run_dir: "/worktree/with/a/long/path/.daruda/flow-runs/run".into(),
            ask_id: 4,
            summary: "Original question".into(),
            buttons: vec![
                ("Keep once".into(), PermissionDecision::Allow("original-once".into())),
                ("Keep always".into(), PermissionDecision::Allow("original-always".into())),
                ("Deny".into(), PermissionDecision::Reject("original-deny".into())),
            ],
        }, super::super::Delivery::Explicit, cx));
        let prepared = prepare(outgoing.receiver.try_recv().unwrap(), cx).unwrap();
        assert!(prepared.pane.is_none());
        let Body::Plain { keyboard: Some(keyboard), .. } = &prepared.body else { panic!("flow card"); };
        assert_eq!(keyboard.rows[0].len(), 3);
        let token = keyboard.rows[0][1].1.clone();
        assert!(token.len() <= 64);
        assert!(!token.contains("worktree"));
        let action = cx.global_mut::<TelegramBridge>().core.routing.route_callback(token.clone());
        assert!(matches!(action, InboundAction::ResolveFlowAsk { ask_id: 4, decision: PermissionDecision::Allow(ref id), .. } if id == "original-always"));
        finish(&prepared.bot, prepared.chat_id, prepared.pane, prepared.card, Some(41), prepared.original, cx);
        assert!(matches!(cx.global_mut::<TelegramBridge>().core.routing.route_text("hello".into(), None), Routed::NeedsTarget(_)));
        cx.set_global(crate::remote_channel::cards::tests::EditCapture::default());
        TelegramBridge::settle_card(&CardId::FlowAsk { run_dir: "/worktree/with/a/long/path/.daruda/flow-runs/run".into(), ask_id: 4 }, "Done", cx);
        assert_eq!(cx.global::<crate::remote_channel::cards::tests::EditCapture>().edits.len(), 1);
        assert!(matches!(cx.global_mut::<TelegramBridge>().core.routing.route_callback(token), InboundAction::ResolveFlowAsk { ask_id: 4, .. }));
    });
}

#[gpui::test]
fn queued_messages_keep_their_owner_and_recipient_without_retargeting(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(|cx| {
        SettingsStore::init(cx);
        let mut config = daruda_config::Config::default();
        config.telegram.enabled = true;
        config.telegram.authorized_chat_id = Some(42);
        cx.global_mut::<SettingsStore>()
            .set_user_for_testing(config.clone());
        let _outbound = install_for_test(true, Some(42), cx);
        let (sender, mut receiver) = futures::channel::mpsc::unbounded();
        cx.global_mut::<TelegramBridge>().outbound_tx = sender;
        cx.global::<TelegramBridge>()
            .send_notice("normal".into(), cx);
        let normal = receiver.next().now_or_never().flatten().unwrap();
        assert!(prepare(normal, cx).is_some());
        cx.global::<TelegramBridge>()
            .send_notice("old recipient".into(), cx);
        let pending = receiver.next().now_or_never().flatten().unwrap();
        config.telegram.authorized_chat_id = Some(99);
        cx.global_mut::<SettingsStore>()
            .set_user_for_testing(config.clone());
        assert!(prepare(pending, cx).is_none());
        cx.global::<TelegramBridge>()
            .send_notice("new recipient before next poll".into(), cx);
        let pending = receiver.next().now_or_never().flatten().unwrap();
        assert_eq!(pending.chat_id, 99);
        assert!(prepare(pending, cx).is_some());
        config.telegram.authorized_chat_id = Some(42);
        cx.global_mut::<SettingsStore>()
            .set_user_for_testing(config);
        cx.global::<TelegramBridge>()
            .send_notice("old bot".into(), cx);
        let pending = receiver.next().now_or_never().flatten().unwrap();
        let original = cx.global::<TelegramBridge>().bot.ready().unwrap().clone();
        cx.global_mut::<TelegramBridge>().bot = Ownership::Ready(Arc::new(ReadyBot {
            token: original.token.clone(),
            inbox: original.inbox.clone(),
            generation: uuid::Uuid::new_v4(),
        }));
        assert!(prepare(pending, cx).is_none());
    });
}

#[gpui::test]
fn a_retired_card_send_cannot_forget_the_replacements_tokens(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        let mut outgoing = install_for_test(true, Some(42), cx);
        let id = crate::control::approval::ApprovalId(77);
        assert!(TelegramBridge::send_approval_card(
            id,
            "Old".into(),
            super::super::Delivery::Explicit,
            cx
        ));
        let old = outgoing.receiver.try_recv().unwrap();
        let original = cx.global::<TelegramBridge>().bot.ready().unwrap().clone();
        cx.global_mut::<TelegramBridge>().bot = Ownership::Ready(Arc::new(ReadyBot {
            token: another_bot(&original),
            inbox: original.inbox.clone(),
            generation: uuid::Uuid::new_v4(),
        }));
        assert!(TelegramBridge::send_approval_card(
            id,
            "New".into(),
            super::super::Delivery::Explicit,
            cx
        ));
        let new = outgoing.receiver.try_recv().unwrap();
        let Outbound::Approval(prompt) = &new.outbound else {
            panic!("approval card")
        };
        let token = prompt.buttons[0].1.clone();
        assert!(prepare(old, cx).is_none());
        assert!(matches!(
            cx.global_mut::<TelegramBridge>()
                .core
                .routing
                .route_callback(token),
            crate::remote_channel::bridge::InboundAction::ResolveApproval { .. }
        ));
    });
}

#[gpui::test]
fn a_duplicate_send_failure_does_not_forget_delivered_card_tokens(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        let mut outgoing = install_for_test(true, Some(42), cx);
        assert!(TelegramBridge::send_approval_card(
            crate::control::approval::ApprovalId(77),
            "Create worktree".into(),
            super::super::Delivery::Explicit,
            cx,
        ));
        let queued = outgoing.receiver.try_recv().unwrap();
        let Outbound::Approval(prompt) = &queued.outbound else {
            panic!("approval card");
        };
        let token = prompt.buttons[0].1.clone();
        let prepared = prepare(queued, cx).unwrap();
        finish(
            &prepared.bot,
            prepared.chat_id,
            prepared.pane,
            prepared.card.clone(),
            Some(47),
            prepared.original.clone(),
            cx,
        );
        finish(
            &prepared.bot,
            prepared.chat_id,
            prepared.pane,
            prepared.card,
            None,
            prepared.original,
            cx,
        );
        assert!(matches!(
            cx.global_mut::<TelegramBridge>()
                .core
                .routing
                .route_callback(token),
            crate::remote_channel::bridge::InboundAction::ResolveApproval { .. }
        ));
    });
}

#[gpui::test]
fn a_fresh_telegram_receipt_extends_the_deadline_but_a_duplicate_does_not(
    cx: &mut gpui::TestAppContext,
) {
    use crate::control::approval::{self, ApprovalOutcome};

    let (prepared, answer) = cx.update(|cx| {
        let mut outgoing = install_for_test(true, Some(42), cx);
        cx.set_global(crate::remote_channel::cards::tests::EditCapture::default());
        let (_, answer) = approval::request(
            "Create worktree".into(),
            super::super::Delivery::Explicit,
            cx,
        );
        let prepared = prepare(outgoing.receiver.try_recv().unwrap(), cx).unwrap();
        (prepared, answer)
    });
    cx.run_until_parked();
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(200));
    cx.update(|cx| {
        finish(
            &prepared.bot,
            prepared.chat_id,
            prepared.pane,
            prepared.card.clone(),
            Some(44),
            prepared.original.clone(),
            cx,
        );
    });
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(101));
    cx.run_until_parked();
    assert!(
        answer.is_empty(),
        "the receipt leaves two minutes to answer"
    );
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(9));
    cx.update(|cx| {
        finish(
            &prepared.bot,
            prepared.chat_id,
            prepared.pane,
            prepared.card.clone(),
            Some(44),
            prepared.original.clone(),
            cx,
        );
    });
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(11));
    cx.run_until_parked();
    assert_eq!(answer.try_recv(), Ok(ApprovalOutcome::TimedOut));
}

#[gpui::test]
fn an_old_telegram_owner_cannot_extend_a_replacement_attempt(cx: &mut gpui::TestAppContext) {
    use crate::control::approval::{self, ApprovalOutcome};

    let (old, replacement, answer) = cx.update(|cx| {
        let mut outgoing = install_for_test(true, Some(42), cx);
        cx.set_global(crate::remote_channel::cards::tests::EditCapture::default());
        let (id, answer) = approval::request(
            "Create worktree".into(),
            super::super::Delivery::Explicit,
            cx,
        );
        let old = prepare(outgoing.receiver.try_recv().unwrap(), cx).unwrap();
        cx.global_mut::<TelegramBridge>().bot = Ownership::Ready(Arc::new(ReadyBot {
            token: another_bot(&old.bot),
            inbox: old.bot.inbox.clone(),
            generation: uuid::Uuid::new_v4(),
        }));
        assert!(TelegramBridge::send_approval_card(
            id,
            "Replacement".into(),
            super::super::Delivery::Explicit,
            cx,
        ));
        let replacement = prepare(outgoing.receiver.try_recv().unwrap(), cx).unwrap();
        (old, replacement, answer)
    });
    cx.run_until_parked();
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(290));
    cx.update(|cx| {
        finish(
            &old.bot,
            old.chat_id,
            old.pane,
            old.card,
            Some(45),
            old.original,
            cx,
        );
        assert!(
            cx.global::<TelegramBridge>()
                .cards
                .borrow()
                .ledger()
                .is_sending(replacement.card.as_ref().unwrap())
        );
    });
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(11));
    cx.run_until_parked();
    assert_eq!(answer.try_recv(), Ok(ApprovalOutcome::TimedOut));
}

#[gpui::test]
fn telegram_settings_changes_retire_cards_without_a_workspace(cx: &mut gpui::TestAppContext) {
    let claim = cx.update(|cx| {
        let mut outgoing = install_for_test(true, Some(42), cx);
        assert!(TelegramBridge::send_approval_card(
            crate::control::approval::ApprovalId(77),
            "Create worktree".into(),
            super::super::Delivery::Explicit,
            cx,
        ));
        prepare(outgoing.receiver.try_recv().unwrap(), cx)
            .unwrap()
            .card
            .unwrap()
    });
    cx.update(|cx| {
        let mut config = SettingsStore::global(cx).user().clone();
        config.telegram.only_when_away = !config.telegram.only_when_away;
        cx.global_mut::<SettingsStore>()
            .set_user_for_testing(config);
    });
    cx.run_until_parked();
    cx.read(|cx| {
        assert!(
            cx.global::<TelegramBridge>()
                .cards
                .borrow()
                .ledger()
                .owns(&claim)
        );
    });
    for recipient in [Some(99), None, Some(42)] {
        cx.update(|cx| {
            let mut config = SettingsStore::global(cx).user().clone();
            config.telegram.authorized_chat_id = recipient;
            cx.global_mut::<SettingsStore>()
                .set_user_for_testing(config);
        });
        cx.run_until_parked();
        cx.read(|cx| {
            assert!(
                cx.global::<TelegramBridge>()
                    .cards
                    .borrow()
                    .ledger()
                    .owns(&claim)
                    .not()
            );
        });
    }
}

#[gpui::test]
fn a_receipt_from_before_recipient_rotation_cannot_extend_an_approval(
    cx: &mut gpui::TestAppContext,
) {
    use crate::control::approval::{self, ApprovalOutcome};

    let (old, answer) = cx.update(|cx| {
        let mut outgoing = install_for_test(true, Some(42), cx);
        cx.set_global(crate::remote_channel::cards::tests::EditCapture::default());
        let (_, answer) = approval::request(
            "Create worktree".into(),
            super::super::Delivery::Explicit,
            cx,
        );
        (
            prepare(outgoing.receiver.try_recv().unwrap(), cx).unwrap(),
            answer,
        )
    });
    cx.run_until_parked();
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(290));
    for recipient in [99, 42] {
        cx.update(|cx| {
            let mut config = SettingsStore::global(cx).user().clone();
            config.telegram.authorized_chat_id = Some(recipient);
            cx.global_mut::<SettingsStore>()
                .set_user_for_testing(config);
        });
        cx.run_until_parked();
    }
    cx.update(|cx| {
        finish(
            &old.bot,
            old.chat_id,
            old.pane,
            old.card,
            Some(46),
            old.original,
            cx,
        );
    });
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(11));
    cx.run_until_parked();
    assert_eq!(answer.try_recv(), Ok(ApprovalOutcome::TimedOut));
}

/// Releasing the bot and serving it again with the same token — an inbox
/// reopen or a keychain revision — keeps the card it sent: no second card is
/// offered, and the first answer edits the one on screen.
#[gpui::test]
fn a_same_bot_replacement_keeps_its_card_and_settles_it(cx: &mut gpui::TestAppContext) {
    use crate::control::approval::{self, ApprovalOutcome};
    cx.update(|cx| {
        let mut outgoing = install_for_test(true, Some(42), cx);
        cx.set_global(crate::remote_channel::cards::tests::EditCapture::default());
        let (id, answer) = approval::request(
            "Create worktree".into(),
            super::super::Delivery::Explicit,
            cx,
        );
        outgoing.begin_send(50, cx).unwrap()(cx);
        let original = cx.global::<TelegramBridge>().bot.ready().unwrap().clone();
        cx.global_mut::<TelegramBridge>().release_claim();
        TelegramBridge::sync_card_owner(cx);
        cx.global_mut::<TelegramBridge>().bot = Ownership::Ready(Arc::new(ReadyBot {
            token: original.token.clone(),
            inbox: original.inbox.clone(),
            generation: uuid::Uuid::new_v4(),
        }));
        TelegramBridge::sync_card_owner(cx);

        approval::relay_outstanding(cx);
        assert!(
            outgoing.try_recv().is_err(),
            "the chat already shows this card"
        );
        assert_eq!(super::super::pending_approval_tokens_for_test(cx), 2);

        assert!(approval::resolve_desktop(id.0, "approve", cx));
        assert_eq!(answer.try_recv(), Ok(ApprovalOutcome::Approved));
        let edits = &cx
            .global::<crate::remote_channel::cards::tests::EditCapture>()
            .edits;
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].ids, ["50"]);
    });
}

#[gpui::test]
fn an_offline_settlement_survives_replacement_and_an_edit_failure(cx: &mut gpui::TestAppContext) {
    use crate::control::approval::{self, ApprovalOutcome};
    use crate::remote_channel::cards::tests::EditCapture;
    cx.update(|cx| {
        let mut outgoing = install_for_test(true, Some(42), cx);
        cx.set_global(EditCapture::default());
        let (id, answer) =
            approval::request("Recover".into(), super::super::Delivery::Explicit, cx);
        outgoing.begin_send(71, cx).unwrap()(cx);
        let original = cx.global::<TelegramBridge>().bot.ready().unwrap().clone();
        cx.global_mut::<TelegramBridge>().release_claim();
        assert!(approval::resolve_desktop(id.0, "approve", cx));
        assert_eq!(answer.try_recv(), Ok(ApprovalOutcome::Approved));
        assert!(cx.global::<EditCapture>().edits.is_empty());
        cx.global_mut::<TelegramBridge>().bot = Ownership::Ready(Arc::new(ReadyBot {
            token: original.token.clone(),
            inbox: original.inbox.clone(),
            generation: uuid::Uuid::new_v4(),
        }));
        TelegramBridge::sync_card_owner(cx);
        cx.global_mut::<EditCapture>().fail = true;
        flush_edits(cx);
        assert_eq!(cx.global::<EditCapture>().edits.len(), 1);
        cx.global_mut::<EditCapture>().fail = false;
        flush_edits(cx);
        assert_eq!(cx.global::<EditCapture>().edits.len(), 2);
        flush_edits(cx);
        assert_eq!(cx.global::<EditCapture>().edits.len(), 2);
        assert!(
            cx.global::<EditCapture>()
                .edits
                .iter()
                .all(|edit| edit.ids == ["71"]
                    && edit.label == crate::surface::strings::control::approval_approved_desktop())
        );
    });
}

#[gpui::test]
fn a_late_send_response_keeps_its_settlement_while_telegram_reopens(cx: &mut gpui::TestAppContext) {
    use crate::control::approval;
    use crate::remote_channel::cards::tests::EditCapture;
    cx.update(|cx| {
        let mut outgoing = install_for_test(true, Some(42), cx);
        cx.set_global(EditCapture::default());
        let (id, _answer) = approval::request(
            "Recover in flight".into(),
            super::super::Delivery::Explicit,
            cx,
        );
        let finish = outgoing.begin_send(74, cx).unwrap();
        let original = cx.global::<TelegramBridge>().bot.ready().unwrap().clone();
        cx.global_mut::<TelegramBridge>().release_claim();
        assert!(approval::resolve_desktop(id.0, "approve", cx));
        finish(cx);
        assert!(cx.global::<EditCapture>().edits.is_empty());
        cx.global_mut::<TelegramBridge>().bot = Ownership::Ready(Arc::new(ReadyBot {
            token: original.token.clone(),
            inbox: original.inbox.clone(),
            generation: uuid::Uuid::new_v4(),
        }));
        TelegramBridge::sync_card_owner(cx);
        flush_edits(cx);
        let edits = &cx.global::<EditCapture>().edits;
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].ids, ["74"]);
        assert_eq!(
            edits[0].label,
            crate::surface::strings::control::approval_approved_desktop()
        );
        approval::relay_outstanding(cx);
        assert!(outgoing.try_recv().is_err());
    });
}
