use super::*;
use daruda_store::inbox::{ChannelKey, InboxId, InboxLease};
use gpui::TestAppContext;
use std::cell::Cell;

fn configure(cx: &mut App) {
    SettingsStore::init(cx);
    let mut config = daruda_config::Config::default();
    config.telegram.enabled = true;
    config.telegram.authorized_chat_id = Some(42);
    cx.global_mut::<SettingsStore>()
        .set_user_for_testing(config);
    let _queue = super::super::install_for_test(true, Some(42), cx);
}

fn ready(lock_root: &std::path::Path, profile: &std::path::Path) -> Arc<ReadyBot> {
    let Ownership::Ready(bot) = bot::open(
        lock_root,
        profile,
        keychain::TokenSnapshot {
            token: "shared-telegram".into(),
            revision: keychain::revision(),
        },
    )
    .unwrap() else {
        panic!("the previous bot lease must have drained");
    };
    bot
}

fn wait_release(root: &std::path::Path) {
    let lease = crate::test_support::once_released(
        || {
            InboxLease::try_acquire(
                root,
                &crate::remote_channel::lock::digest_of("shared-telegram"),
            )
        },
        |lease| matches!(lease, Ok(None)),
    )
    .unwrap()
    .unwrap();
    drop(lease);
}

fn update(id: i64) -> client::Update {
    client::Update {
        update_id: id,
        kind: client::UpdateKind::Message {
            chat_id: 42,
            text: "/list".into(),
            reply_to_message_id: None,
        },
    }
}

#[gpui::test]
async fn telegram_redelivery_and_claimed_before_crash_have_no_second_effect_or_reply(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let _workspace = crate::test_support::workspace_for_control(cx);
    cx.update(configure);
    let locks = tempfile::tempdir().unwrap();
    let first_profile = tempfile::tempdir().unwrap();
    let second_profile = tempfile::tempdir().unwrap();
    let first = ready(locks.path(), first_profile.path());
    cx.update(|cx| cx.global_mut::<TelegramBridge>().bot = Ownership::Ready(first.clone()));
    let mut app = cx.to_async();
    let mut replies = 0;
    for _ in 0..2 {
        if claim_update(&first, 42, Some(42), &mut app, keychain::revision).await
            == ClaimOutcome::Committed
        {
            let effect = app.update(|cx| dispatch_update(&first, Some(42), update(42), cx));
            assert!(matches!(effect, Some((dispatch::Effect::Reply(_), None))));
            replies += 1;
        }
    }
    assert_eq!(
        claim_update(&first, 43, Some(42), &mut app, keychain::revision).await,
        ClaimOutcome::Committed
    );
    cx.update(|cx| cx.global_mut::<TelegramBridge>().release_claim());
    drop(first);
    wait_release(locks.path());
    let second = ready(locks.path(), second_profile.path());
    cx.update(|cx| cx.global_mut::<TelegramBridge>().bot = Ownership::Ready(second.clone()));
    for id in [42, 43] {
        assert_eq!(
            claim_update(&second, id, Some(42), &mut app, keychain::revision).await,
            ClaimOutcome::Ignored
        );
    }
    assert_eq!(replies, 1);
    cx.update(|cx| cx.global_mut::<TelegramBridge>().release_claim());
    drop(second);
    wait_release(locks.path());
}

#[gpui::test]
async fn a_credential_aba_during_the_real_claim_and_a_recipient_change_before_dispatch_drop_effects(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    cx.update(configure);
    let bot = cx.update(|cx| cx.global::<TelegramBridge>().bot.ready().unwrap().clone());
    let mut app = cx.to_async();
    let reads = Cell::new(0);
    assert_eq!(
        claim_update(&bot, 42, Some(42), &mut app, || {
            reads.set(reads.get() + 1);
            bot.token.revision + u64::from(reads.get() > 1)
        })
        .await,
        ClaimOutcome::Ignored
    );
    assert_eq!(
        bot.inbox.snapshot().maximum(&ChannelKey::new("telegram")),
        Some(42)
    );
    assert!(matches!(
        bot.inbox
            .claim(ChannelKey::new("telegram"), InboxId::Monotonic(42))
            .await,
        Err(daruda_store::inbox::CommitError::Rejected(_))
    ));
    assert_eq!(
        claim_update(&bot, 43, Some(42), &mut app, keychain::revision).await,
        ClaimOutcome::Committed
    );
    cx.update(|cx| {
        let mut config = SettingsStore::global(cx).user().clone();
        config.telegram.authorized_chat_id = Some(99);
        cx.global_mut::<SettingsStore>()
            .set_user_for_testing(config);
        assert!(dispatch_update(&bot, Some(42), update(43), cx).is_none());
    });
}

#[gpui::test]
async fn a_failed_pair_cannot_authorize_later_commands_in_the_same_unpaired_batch(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let _workspace = crate::test_support::workspace_for_control(cx);
    let invalid_config = b"unpaired = [";
    let (bot, pair_code, config_path) = cx.update(|cx| {
        let _queue = super::super::install_for_test(true, None, cx);
        let pair_code = TelegramBridge::generate_pair_code(cx);
        let config_path = SettingsStore::global(cx)
            .writer_path_for_testing()
            .to_owned();
        std::fs::write(&config_path, invalid_config).unwrap();
        let bot = cx.global::<TelegramBridge>().bot.ready().unwrap().clone();
        (bot, pair_code, config_path)
    });
    let mut app = cx.to_async();
    for (id, text) in [
        (42, format!("/pair {pair_code}")),
        (43, "/list".into()),
        (44, "untrusted prompt".into()),
    ] {
        assert_eq!(
            claim_update(&bot, id, None, &mut app, keychain::revision).await,
            ClaimOutcome::Committed
        );
        let incoming = client::Update {
            update_id: id,
            kind: client::UpdateKind::Message {
                chat_id: 42,
                text,
                reply_to_message_id: None,
            },
        };
        let effect = app.update(|cx| {
            let effect = dispatch_update(&bot, None, incoming, cx);
            assert_eq!(
                SettingsStore::global(cx).user().telegram.authorized_chat_id,
                None
            );
            effect
        });
        assert!(
            matches!(effect, Some((dispatch::Effect::None, None))),
            "update {id} must not gain authority from a failed pairing"
        );
    }
    assert_eq!(
        bot.inbox.snapshot().maximum(&ChannelKey::new("telegram")),
        Some(44)
    );
    assert_eq!(std::fs::read(config_path).unwrap(), invalid_config);
}

#[gpui::test]
async fn failed_claims_release_the_owner_for_backed_off_reopen_and_preserve_history(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    cx.update(configure);
    for error in [
        daruda_store::inbox::CommitError::Io(std::io::Error::other("temporary storage failure")),
        daruda_store::inbox::CommitError::Closed,
        daruda_store::inbox::CommitError::Poisoned,
    ] {
        let locks = tempfile::tempdir().unwrap();
        let profile = tempfile::tempdir().unwrap();
        let bot = ready(locks.path(), profile.path());
        cx.update(|cx| cx.global_mut::<TelegramBridge>().bot = Ownership::Ready(bot.clone()));
        let mut app = cx.to_async();
        assert_eq!(
            claim_update(&bot, 41, Some(42), &mut app, keychain::revision).await,
            ClaimOutcome::Committed
        );
        assert_eq!(
            finish_claim_update(
                &bot,
                Some(42),
                std::future::ready(Err(error)),
                &mut app,
                keychain::revision
            )
            .await,
            ClaimOutcome::Retry
        );
        cx.update(|cx| assert!(matches!(cx.global::<TelegramBridge>().bot, Ownership::Idle)));
        assert!(
            InboxLease::try_acquire(
                locks.path(),
                &crate::remote_channel::lock::digest_of("shared-telegram")
            )
            .unwrap()
            .is_none()
        );
        drop(bot);
        wait_release(locks.path());
        let reopened = ready(locks.path(), profile.path());
        cx.update(|cx| cx.global_mut::<TelegramBridge>().bot = Ownership::Ready(reopened.clone()));
        assert_eq!(
            claim_update(&reopened, 41, Some(42), &mut app, keychain::revision).await,
            ClaimOutcome::Ignored
        );
        assert_eq!(
            claim_update(&reopened, 42, Some(42), &mut app, keychain::revision).await,
            ClaimOutcome::Committed
        );
        cx.update(|cx| cx.global_mut::<TelegramBridge>().release_claim());
        drop(reopened);
        wait_release(locks.path());
    }
}

#[gpui::test]
async fn an_old_failed_claim_does_not_release_a_new_healthy_owner(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    cx.update(configure);
    let old = cx.update(|cx| cx.global::<TelegramBridge>().bot.ready().unwrap().clone());
    let locks = tempfile::tempdir().unwrap();
    let profile = tempfile::tempdir().unwrap();
    let current = ready(locks.path(), profile.path());
    cx.update(|cx| cx.global_mut::<TelegramBridge>().bot = Ownership::Ready(current.clone()));
    let mut app = cx.to_async();
    assert_eq!(
        finish_claim_update(
            &old,
            Some(42),
            std::future::ready(Err(daruda_store::inbox::CommitError::Closed)),
            &mut app,
            keychain::revision
        )
        .await,
        ClaimOutcome::Retry
    );
    cx.update(|cx| assert!(live_bot(&current, Some(42), keychain::revision(), cx)));
    cx.update(|cx| cx.global_mut::<TelegramBridge>().release_claim());
    drop(current);
    wait_release(locks.path());
}

fn tap(message_id: i64, data: &str) -> client::Update {
    client::Update {
        update_id: 900 + message_id,
        kind: client::UpdateKind::Callback {
            chat_id: 42,
            callback_id: format!("cb-{message_id}"),
            data: data.into(),
            message_id,
            message_text: "daruda wants to create a worktree. Allow?".into(),
        },
    }
}

/// A tap on an approval card this process no longer tracks (an earlier
/// run's card) is answered and loses its buttons; a tap racing a settlement
/// edit is only answered, so the settled result is not overwritten.
#[gpui::test]
fn a_stale_tap_strips_only_a_card_no_settlement_owns(cx: &mut TestAppContext) {
    cx.update(|cx| {
        configure(cx);
        let bot = cx.global::<TelegramBridge>().bot.ready().unwrap().clone();
        let stale = format!(
            "{}:{}",
            crate::remote_channel::bridge::core::APPROVAL_TOKEN_PREFIX,
            "0".repeat(32)
        );
        let (effect, tapped) = dispatch_update(&bot, Some(42), tap(7, &stale), cx).unwrap();
        let Some(Answer::AckAndEdit {
            callback_id,
            message,
            label,
        }) = answer_for(effect, tapped)
        else {
            panic!("an untracked dead card loses its buttons");
        };
        assert_eq!(callback_id, "cb-7");
        assert_eq!((message.chat_id, message.message_id), (42, 7));
        assert_eq!(label, s::control::approval_already_answered());

        cx.global::<TelegramBridge>()
            .cards
            .borrow_mut()
            .note_settled(8);
        let (effect, tapped) = dispatch_update(&bot, Some(42), tap(8, &stale), cx).unwrap();
        assert!(matches!(
            answer_for(effect, tapped),
            Some(Answer::Ack { callback_id, .. }) if callback_id == "cb-8"
        ));

        let (effect, tapped) = dispatch_update(&bot, Some(42), tap(9, "unknown"), cx).unwrap();
        assert!(matches!(
            answer_for(effect, tapped),
            Some(Answer::AckAndEdit { label, .. })
                if label == s::notification::telegram_permission_stale()
        ));
    });
}

#[gpui::test]
fn taps_before_the_send_response_never_compete_with_the_settlement_edit(cx: &mut TestAppContext) {
    use crate::control::approval::{self, ApprovalOutcome};
    use crate::remote_channel::cards::tests::EditCapture;
    cx.update(|cx| {
        let mut outgoing = super::super::install_for_test(true, Some(42), cx);
        cx.set_global(EditCapture::default());
        let (_id, answer) = approval::request(
            "In flight".into(),
            crate::remote_channel::global::Delivery::Explicit,
            cx,
        );
        let bot = cx.global::<TelegramBridge>().bot.ready().unwrap().clone();
        let queued = outgoing.receiver.try_recv().unwrap();
        let crate::remote_channel::bridge::Outbound::Approval(prompt) = &queued.outbound else {
            panic!("approval");
        };
        let (approve, refuse) = (prompt.buttons[0].1.clone(), prompt.buttons[1].1.clone());
        cx.global::<TelegramBridge>()
            .outbound_tx
            .unbounded_send(queued)
            .unwrap();
        let finish = outgoing.begin_send(72, cx).unwrap();
        let (effect, tapped) = dispatch_update(&bot, Some(42), tap(72, &approve), cx).unwrap();
        assert!(matches!(
            answer_for(effect, tapped),
            Some(Answer::Ack { .. })
        ));
        assert_eq!(answer.try_recv(), Ok(ApprovalOutcome::Approved));
        let (effect, tapped) = dispatch_update(&bot, Some(42), tap(72, &refuse), cx).unwrap();
        assert!(matches!(
            answer_for(effect, tapped),
            Some(Answer::Ack { .. })
        ));
        assert!(cx.global::<EditCapture>().edits.is_empty());
        finish(cx);
        assert_eq!(cx.global::<EditCapture>().edits.len(), 1);
        assert_eq!(
            cx.global::<EditCapture>().edits[0].label,
            s::control::approval_approved_telegram()
        );
    });
}

#[gpui::test]
fn a_desktop_answer_before_the_send_response_protects_the_first_phone_tap(cx: &mut TestAppContext) {
    use crate::control::approval;
    cx.update(|cx| {
        let mut outgoing = super::super::install_for_test(true, Some(42), cx);
        cx.set_global(crate::remote_channel::cards::tests::EditCapture::default());
        let (id, _answer) = approval::request(
            "In flight".into(),
            crate::remote_channel::global::Delivery::Explicit,
            cx,
        );
        let bot = cx.global::<TelegramBridge>().bot.ready().unwrap().clone();
        let queued = outgoing.receiver.try_recv().unwrap();
        let crate::remote_channel::bridge::Outbound::Approval(prompt) = &queued.outbound else {
            panic!("approval");
        };
        let approve = prompt.buttons[0].1.clone();
        cx.global::<TelegramBridge>()
            .outbound_tx
            .unbounded_send(queued)
            .unwrap();
        let finish = outgoing.begin_send(73, cx).unwrap();
        assert!(approval::resolve_desktop(id.0, "approve", cx));
        let (effect, tapped) = dispatch_update(&bot, Some(42), tap(73, &approve), cx).unwrap();
        assert!(matches!(
            answer_for(effect, tapped),
            Some(Answer::Ack { .. })
        ));
        finish(cx);
    });
}
