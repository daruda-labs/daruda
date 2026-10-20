use super::*;
use crate::control::approval::{self, ApprovalChoice, ApprovalOutcome};
use crate::remote_channel::cards::tests::EditCapture;
use crate::remote_channel::global::Delivery;
use futures::FutureExt as _;

mod flow_asks;

fn queued(
    cx: &mut App,
) -> (
    futures::channel::mpsc::UnboundedReceiver<Pending>,
    crate::telegram::global::TestOutbox,
) {
    let remote = crate::remote_channel::global::tests::setup(cx);
    let telegram = crate::telegram::global::install_for_test(true, Some(42), cx);
    cx.set_global(EditCapture::default());
    (remote, telegram)
}

fn begin(
    remote: &mut futures::channel::mpsc::UnboundedReceiver<Pending>,
    cx: &mut App,
) -> Vec<(Pending, Prepared)> {
    let mut sending = Vec::new();
    while let Some(Some(pending)) = remote.next().now_or_never() {
        let prepared = prepare(&pending, cx).unwrap();
        sending.push((pending, prepared));
    }
    sending
}

fn complete(sending: Vec<(Pending, Prepared)>, cx: &mut App) {
    for (pending, prepared) in sending {
        finish(
            &pending,
            prepared.pane,
            vec![format!("{}-1", pending.id), format!("{}-2", pending.id)],
            prepared.original,
            cx,
        );
    }
}

#[gpui::test]
fn a_desktop_approval_settles_every_channel_using_its_delivered_ids(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        let (mut remote, mut telegram) = queued(cx);
        let (id, answer) = approval::request("Create worktree".into(), Delivery::Explicit, cx);
        assert!(
            !crate::telegram::global::TelegramBridge::send_approval_card(
                id,
                "duplicate".into(),
                Delivery::Explicit,
                cx
            )
        );
        assert_eq!(
            crate::telegram::global::pending_approval_tokens_for_test(cx),
            2
        );
        complete(begin(&mut remote, cx), cx);
        telegram.begin_send(10, cx).unwrap()(cx);
        assert!(approval::resolve_desktop(id.0, "approve", cx));
        assert_eq!(answer.try_recv(), Ok(ApprovalOutcome::Approved));
        let edits = &cx.global::<EditCapture>().edits;
        assert_eq!(edits.len(), 3);
        let mut channels: Vec<_> = edits.iter().map(|edit| edit.channel.as_str()).collect();
        channels.sort();
        assert_eq!(channels, ["personal", "telegram", "work"]);
        assert!(edits.iter().all(|edit| edit.original == "Create worktree"
            && edit.label == crate::surface::strings::control::approval_approved_desktop()));
        assert_eq!(edits.iter().map(|edit| edit.ids.len()).sum::<usize>(), 5);
        assert!(!approval::resolve(
            id,
            ApprovalChoice::Refused,
            approval::Answerer::Telegram,
            cx
        ));
        assert_eq!(cx.global::<EditCapture>().edits.len(), 3);
        assert!(
            cx.global::<RemoteChannels>()
                .connections
                .values()
                .all(|connection| !connection
                    .cards
                    .borrow()
                    .ledger()
                    .contains(&CardId::Approval(id))
                    && connection.core.pending_approval_token_count() == 0)
        );
        assert_eq!(
            crate::telegram::global::pending_approval_tokens_for_test(cx),
            0
        );
    });
}

#[gpui::test]
fn withdrawing_while_sending_settles_all_late_receipts(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        let (mut remote, mut telegram) = queued(cx);
        let (id, answer) = approval::request("Create worktree".into(), Delivery::Explicit, cx);
        let sending = begin(&mut remote, cx);
        let telegram_sent = telegram.begin_send(11, cx).unwrap();
        assert!(approval::withdraw(id, cx));
        assert_eq!(answer.try_recv(), Ok(ApprovalOutcome::Withdrawn));
        assert!(cx.global::<EditCapture>().edits.is_empty());
        complete(sending, cx);
        telegram_sent(cx);
        assert_eq!(cx.global::<EditCapture>().edits.len(), 3);
        assert!(
            cx.global::<EditCapture>()
                .edits
                .iter()
                .all(|edit| edit.label == crate::surface::strings::control::approval_cancelled())
        );
        assert!(
            cx.global::<RemoteChannels>()
                .connections
                .values()
                .all(|connection| !connection
                    .cards
                    .borrow()
                    .ledger()
                    .contains(&CardId::Approval(id)))
        );
    });
}

#[gpui::test]
fn settlement_before_preparation_drops_queued_cards_and_tokens(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        let (mut remote, mut telegram) = queued(cx);
        let (id, _) = approval::request("Create worktree".into(), Delivery::Explicit, cx);
        assert!(approval::resolve_desktop(id.0, "refuse", cx));
        while let Some(Some(pending)) = remote.next().now_or_never() {
            assert!(prepare(&pending, cx).is_none());
        }
        assert!(telegram.begin_send(12, cx).is_none());
        assert!(cx.global::<EditCapture>().edits.is_empty());
        assert!(
            cx.global::<RemoteChannels>()
                .connections
                .values()
                .all(|connection| connection.core.pending_approval_token_count() == 0)
        );
        assert_eq!(
            crate::telegram::global::pending_approval_tokens_for_test(cx),
            0
        );
    });
}

#[gpui::test]
fn failed_edits_keep_the_first_decision_and_retire_live_card_entries(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(|cx| {
        let (mut remote, mut telegram) = queued(cx);
        let (id, answer) = approval::request("Create worktree".into(), Delivery::Explicit, cx);
        complete(begin(&mut remote, cx), cx);
        telegram.begin_send(13, cx).unwrap()(cx);
        cx.global_mut::<EditCapture>().fail = true;
        assert!(approval::resolve(
            id,
            ApprovalChoice::Approved,
            approval::Answerer::Telegram,
            cx
        ));
        assert_eq!(answer.try_recv(), Ok(ApprovalOutcome::Approved));
        assert_eq!(cx.global::<EditCapture>().edits.len(), 3);
        assert!(!approval::resolve_desktop(id.0, "refuse", cx));
        assert!(
            cx.global::<RemoteChannels>()
                .connections
                .values()
                .all(|connection| !connection
                    .cards
                    .borrow()
                    .ledger()
                    .contains(&CardId::Approval(id)))
        );
    });
}

#[gpui::test]
fn a_retired_card_send_cannot_forget_the_replacements_tokens(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        let mut outgoing = crate::remote_channel::global::tests::setup(cx);
        let id = approval::ApprovalId(77);
        assert!(RemoteChannels::send_approval(
            id,
            "Old".into(),
            Delivery::Explicit,
            cx
        ));
        let old = outgoing.next().now_or_never().flatten().unwrap();
        while outgoing.next().now_or_never().flatten().is_some() {}
        let connection = cx
            .global_mut::<RemoteChannels>()
            .connections
            .get_mut(&old.id)
            .unwrap();
        connection.cards = Default::default();
        connection.generation = uuid::Uuid::new_v4();
        connection.publish_bot(crate::remote_channel::global::tests::ready_bot());
        assert!(RemoteChannels::send_approval(
            id,
            "New".into(),
            Delivery::Explicit,
            cx
        ));
        let new = outgoing.next().now_or_never().flatten().unwrap();
        let Outbound::Approval(prompt) = &new.outbound else {
            panic!("approval card")
        };
        let token = prompt.buttons[0].1.clone();
        assert!(prepare(&old, cx).is_none());
        assert!(matches!(
            cx.global_mut::<RemoteChannels>()
                .connections
                .get_mut(&new.id)
                .unwrap()
                .core
                .route_callback(token),
            crate::remote_channel::bridge::InboundAction::ResolveApproval { .. }
        ));
    });
}

#[gpui::test]
fn a_duplicate_send_failure_does_not_forget_delivered_card_tokens(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        let mut outgoing = crate::remote_channel::global::tests::setup(cx);
        assert!(RemoteChannels::send_approval(
            approval::ApprovalId(77),
            "Create worktree".into(),
            Delivery::Explicit,
            cx,
        ));
        let pending = outgoing.try_recv().unwrap();
        let prepared = prepare(&pending, cx).unwrap();
        let Outbound::Approval(prompt) = &pending.outbound else {
            panic!("approval card");
        };
        let token = prompt.buttons[0].1.clone();
        finish(
            &pending,
            prepared.pane,
            vec!["message".into()],
            prepared.original.clone(),
            cx,
        );
        finish(&pending, prepared.pane, vec![], prepared.original, cx);
        assert!(matches!(
            cx.global_mut::<RemoteChannels>()
                .connections
                .get_mut(&pending.id)
                .unwrap()
                .core
                .route_callback(token),
            crate::remote_channel::bridge::InboundAction::ResolveApproval { .. }
        ));
    });
}

#[gpui::test]
fn a_fresh_remote_receipt_extends_the_deadline_but_a_duplicate_does_not(
    cx: &mut gpui::TestAppContext,
) {
    let (sending, answer) = cx.update(|cx| {
        let (mut remote, _telegram) = queued(cx);
        let (_, answer) = approval::request("Create worktree".into(), Delivery::Explicit, cx);
        (begin(&mut remote, cx), answer)
    });
    cx.run_until_parked();
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(200));
    cx.update(|cx| {
        for (pending, prepared) in &sending {
            finish(
                pending,
                prepared.pane,
                vec!["message".into()],
                prepared.original.clone(),
                cx,
            );
        }
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
        for (pending, prepared) in &sending {
            finish(
                pending,
                prepared.pane,
                vec!["duplicate".into()],
                prepared.original.clone(),
                cx,
            );
        }
    });
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(11));
    cx.run_until_parked();
    assert_eq!(answer.try_recv(), Ok(ApprovalOutcome::TimedOut));
}

#[gpui::test]
fn an_old_remote_owner_cannot_extend_a_replacement_attempt(cx: &mut gpui::TestAppContext) {
    let (old, prepared, replacement, answer) = cx.update(|cx| {
        let (mut remote, _telegram) = queued(cx);
        let (id, answer) = approval::request(
            "Create worktree".into(),
            Delivery::Presence { away: false },
            cx,
        );
        let (old, prepared) = begin(&mut remote, cx).pop().unwrap();
        let connection = cx
            .global_mut::<RemoteChannels>()
            .connections
            .get_mut(&old.id)
            .unwrap();
        connection.cards = Default::default();
        connection.generation = uuid::Uuid::new_v4();
        connection.publish_bot(crate::remote_channel::global::tests::ready_bot());
        assert!(RemoteChannels::send_approval(
            id,
            "Replacement".into(),
            Delivery::Presence { away: false },
            cx,
        ));
        let (replacement, _) = begin(&mut remote, cx).pop().unwrap();
        (old, prepared, replacement, answer)
    });
    cx.run_until_parked();
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(290));
    cx.update(|cx| {
        finish(
            &old,
            prepared.pane,
            vec!["retired-message".into()],
            prepared.original,
            cx,
        );
        assert!(
            cx.global::<RemoteChannels>().connections[&replacement.id]
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

fn relay_keeps_channel_claimed(delivered: bool, cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        let (mut remote, mut telegram) = queued(cx);
        let (id, _) = approval::request(
            "Create worktree".into(),
            Delivery::Presence { away: false },
            cx,
        );
        let (pending, prepared) = begin(&mut remote, cx).pop().unwrap();
        assert_eq!(pending.id, "personal");
        finish(
            &pending,
            prepared.pane,
            if delivered {
                vec!["sent".into()]
            } else {
                Vec::new()
            },
            prepared.original,
            cx,
        );
        assert!(crate::remote_channel::send_approval_card(
            id,
            "Create worktree".into(),
            Delivery::Presence { away: true },
            cx,
        ));
        let next = remote.try_recv().unwrap();
        assert_eq!(next.id, "work");
        assert!(remote.try_recv().is_err());
        assert!(telegram.try_recv().is_ok());
        assert!(!crate::remote_channel::send_approval_card(
            id,
            "Create worktree".into(),
            Delivery::Presence { away: true },
            cx,
        ));
        assert!(approval::withdraw(id, cx));
    });
}

#[gpui::test]
fn away_relay_keeps_a_sent_channel_claimed(cx: &mut gpui::TestAppContext) {
    relay_keeps_channel_claimed(true, cx);
}

#[gpui::test]
fn away_relay_keeps_an_ambiguous_channel_claimed(cx: &mut gpui::TestAppContext) {
    relay_keeps_channel_claimed(false, cx);
}

/// Recovering the inbox, and rewriting the credential with the same bot, keep
/// the cards that bot sent: the pump does not send a second card, and the
/// first answer still edits the one on screen.
#[gpui::test]
fn a_same_bot_recovery_keeps_its_cards_and_settles_them(cx: &mut gpui::TestAppContext) {
    use crate::remote_channel::global::tests::ready_bot;
    use crate::remote_channel::runtime::Status;
    cx.update(|cx| {
        let (mut remote, _telegram) = queued(cx);
        let (id, answer) = approval::request("Create worktree".into(), Delivery::Explicit, cx);
        complete(begin(&mut remote, cx), cx);

        let (generation, bot) = {
            let connection = &cx.global::<RemoteChannels>().connections["work"];
            (connection.generation, connection.bot.clone().unwrap())
        };
        RemoteChannels::recover_inbox("work", generation, &bot, cx);
        let connection = cx
            .global_mut::<RemoteChannels>()
            .connections
            .get_mut("work")
            .unwrap();
        connection.publish_bot(ready_bot());
        connection.status = Status::Connected;

        let config = cx.global::<RemoteChannels>().connections["personal"]
            .config
            .clone();
        RemoteChannels::suspend(config, cx);
        RemoteChannels::restart("personal", cx);
        cx.global_mut::<RemoteChannels>()
            .connections
            .get_mut("personal")
            .unwrap()
            .publish_bot(ready_bot());

        approval::relay_outstanding(cx);
        assert!(
            remote.next().now_or_never().is_none(),
            "the recovered channels already show this card"
        );
        for connection in cx.global::<RemoteChannels>().connections.values() {
            assert_eq!(connection.core.pending_approval_token_count(), 2);
        }

        assert!(approval::resolve_desktop(id.0, "approve", cx));
        assert_eq!(answer.try_recv(), Ok(ApprovalOutcome::Approved));
        let mut edited: Vec<_> = cx
            .global::<EditCapture>()
            .edits
            .iter()
            .map(|edit| edit.channel.clone())
            .collect();
        edited.sort();
        assert_eq!(edited, ["personal", "work"]);
    });
}

/// Another bot cannot edit the old messages or receive their taps, so its
/// arrival forgets the old cards' tokens with their records, and a request
/// still waiting is offered to it exactly once.
#[gpui::test]
fn a_different_bot_forgets_old_cards_and_is_offered_once(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        let (mut remote, _telegram) = queued(cx);
        let (id, _answer) = approval::request("Create worktree".into(), Delivery::Explicit, cx);
        complete(begin(&mut remote, cx), cx);
        cx.global_mut::<RemoteChannels>()
            .connections
            .get_mut("work")
            .unwrap()
            .publish_bot(crate::remote_channel::global::tests::ready_bot_as(
                "another-bot",
            ));
        let work = &cx.global::<RemoteChannels>().connections["work"];
        assert_eq!(work.core.pending_approval_token_count(), 0);
        assert!(!work.cards.borrow().ledger().contains(&CardId::Approval(id)));

        approval::relay_outstanding(cx);
        let offered = remote.next().now_or_never().flatten().unwrap();
        assert_eq!(offered.id, "work");
        approval::relay_outstanding(cx);
        assert!(remote.next().now_or_never().is_none(), "exactly once");
        assert!(approval::withdraw(id, cx));
    });
}

#[gpui::test]
fn settlement_during_recovery_waits_for_the_same_bot_and_failed_edits_retry(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(|cx| {
        let (mut remote, _telegram) = queued(cx);
        let (id, answer) = approval::request("Recover".into(), Delivery::Explicit, cx);
        complete(begin(&mut remote, cx), cx);
        let (generation, bot) = {
            let connection = &cx.global::<RemoteChannels>().connections["work"];
            (connection.generation, connection.bot.clone().unwrap())
        };
        RemoteChannels::recover_inbox("work", generation, &bot, cx);
        assert!(approval::resolve_desktop(id.0, "approve", cx));
        assert_eq!(answer.try_recv(), Ok(ApprovalOutcome::Approved));
        assert_eq!(cx.global::<EditCapture>().edits.len(), 1);
        cx.global_mut::<RemoteChannels>()
            .connections
            .get_mut("work")
            .unwrap()
            .publish_bot(crate::remote_channel::global::tests::ready_bot());
        cx.global_mut::<RemoteChannels>()
            .connections
            .get_mut("work")
            .unwrap()
            .status = crate::remote_channel::runtime::Status::Connected;
        cx.global_mut::<EditCapture>().fail = true;
        flush_edits(cx);
        assert_eq!(cx.global::<EditCapture>().edits.len(), 2);
        cx.global_mut::<EditCapture>().fail = false;
        flush_edits(cx);
        assert_eq!(cx.global::<EditCapture>().edits.len(), 3);
        flush_edits(cx);
        assert_eq!(
            cx.global::<EditCapture>().edits.len(),
            3,
            "successful edits leave the queue"
        );
        assert!(cx.global::<EditCapture>().edits.iter().all(
            |edit| edit.label == crate::surface::strings::control::approval_approved_desktop()
        ));
    });
}

#[gpui::test]
fn an_inflight_receipt_keeps_its_settlement_across_a_same_bot_recovery(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(|cx| {
        let (mut remote, _telegram) = queued(cx);
        let (id, answer) = approval::request("Recover in flight".into(), Delivery::Explicit, cx);
        let sending = begin(&mut remote, cx);
        let connection = &cx.global::<RemoteChannels>().connections["work"];
        let (generation, bot) = (connection.generation, connection.bot.clone().unwrap());
        RemoteChannels::recover_inbox("work", generation, &bot, cx);
        assert!(approval::resolve_desktop(id.0, "approve", cx));
        assert_eq!(answer.try_recv(), Ok(ApprovalOutcome::Approved));
        complete(sending, cx);
        assert_eq!(
            cx.global::<EditCapture>().edits.len(),
            1,
            "the offline receipt waits"
        );
        let connection = cx
            .global_mut::<RemoteChannels>()
            .connections
            .get_mut("work")
            .unwrap();
        connection.publish_bot(crate::remote_channel::global::tests::ready_bot());
        connection.status = crate::remote_channel::runtime::Status::Connected;
        flush_edits(cx);
        assert_eq!(cx.global::<EditCapture>().edits.len(), 2);
        assert!(cx.global::<EditCapture>().edits.iter().all(
            |edit| edit.label == crate::surface::strings::control::approval_approved_desktop()
        ));
        approval::relay_outstanding(cx);
        assert!(remote.try_recv().is_err());
    });
}
