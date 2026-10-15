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
        if claim_update(&first, 42, Some(42), &mut app, keychain::revision).await {
            let effect = app.update(|cx| dispatch_update(&first, Some(42), update(42), cx));
            assert!(matches!(
                effect,
                Some((dispatch::Effect::Reply(_), None, None))
            ));
            replies += 1;
        }
    }
    assert!(claim_update(&first, 43, Some(42), &mut app, keychain::revision).await);
    cx.update(|cx| cx.global_mut::<TelegramBridge>().release_claim());
    drop(first);
    wait_release(locks.path());
    let second = ready(locks.path(), second_profile.path());
    cx.update(|cx| cx.global_mut::<TelegramBridge>().bot = Ownership::Ready(second.clone()));
    for id in [42, 43] {
        assert!(!claim_update(&second, id, Some(42), &mut app, keychain::revision).await);
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
    assert!(
        !claim_update(&bot, 42, Some(42), &mut app, || {
            reads.set(reads.get() + 1);
            bot.token.revision + u64::from(reads.get() > 1)
        })
        .await
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
    assert!(claim_update(&bot, 43, Some(42), &mut app, keychain::revision).await);
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
        assert!(claim_update(&bot, id, None, &mut app, keychain::revision).await);
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
            matches!(effect, Some((dispatch::Effect::None, None, None))),
            "update {id} must not gain authority from a failed pairing"
        );
    }
    assert_eq!(
        bot.inbox.snapshot().maximum(&ChannelKey::new("telegram")),
        Some(44)
    );
    assert_eq!(std::fs::read(config_path).unwrap(), invalid_config);
}
