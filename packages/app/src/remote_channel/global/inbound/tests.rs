use super::*;
use crate::remote_channel::command::RenderedReply;
use crate::remote_channel::{lock, runtime::BotSession, transport::Credentials};
use daruda_config::remote::{ChannelConfig, ChannelKind, RemoteRecipient};
use daruda_store::inbox::{InboxLease, InboxSeed, InboxStore};
use gpui::TestAppContext;

fn open_bot(
    root: &std::path::Path,
    seed: impl FnOnce() -> std::io::Result<InboxSeed>,
) -> Arc<BotSession> {
    let lease = crate::test_support::once_released(
        || InboxLease::try_acquire(root, &lock::digest_of("shared-bot")),
        |lease| matches!(lease, Ok(None)),
    )
    .unwrap()
    .unwrap();
    let inbox = InboxStore::open(lease, "test", seed).unwrap();
    Arc::new(BotSession::new(
        Arc::new(Credentials {
            bot: "shared-bot".into(),
            app: Some("app".into()),
        }),
        Arc::new(inbox),
        Default::default(),
    ))
}

fn wait_release(root: &std::path::Path) {
    let lease = crate::test_support::once_released(
        || InboxLease::try_acquire(root, &lock::digest_of("shared-bot")),
        |lease| matches!(lease, Ok(None)),
    )
    .unwrap()
    .unwrap();
    drop(lease);
}

fn install_connection(id: &str, bot: Arc<BotSession>, cx: &mut App) -> uuid::Uuid {
    SettingsStore::init(cx);
    let mut config = daruda_config::Config::default();
    let mut channel = ChannelConfig::new(id.into(), ChannelKind::Slack);
    channel.enabled = true;
    channel.recipient = Some(RemoteRecipient {
        user_id: "user".into(),
        conversation_id: "room".into(),
        scope_id: "scope".into(),
    });
    config.remote.channels.push(channel);
    cx.global_mut::<SettingsStore>()
        .set_user_for_testing(config);
    let (events, _) = futures::channel::mpsc::unbounded();
    let (outbound, _) = futures::channel::mpsc::unbounded();
    cx.set_global(RemoteChannels {
        connections: Default::default(),
        events,
        outbound,
        applied: None,
    });
    super::super::reconcile(cx);
    let connection = cx
        .global_mut::<RemoteChannels>()
        .connections
        .get_mut(id)
        .unwrap();
    connection.bot = Some(bot);
    connection.status = crate::remote_channel::runtime::Status::Connected;
    connection.generation
}

#[gpui::test]
async fn slack_redelivery_after_a_profile_uuid_handoff_has_no_second_effect_or_reply(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let _workspace = crate::test_support::workspace_for_control(cx);
    let root = tempfile::tempdir().unwrap();
    let profile = tempfile::tempdir().unwrap();
    let first_id = uuid::Uuid::new_v4().to_string();
    let second_id = uuid::Uuid::new_v4().to_string();
    assert_ne!(first_id, second_id);
    std::fs::write(
        profile.path().join("remote_channels.json"),
        serde_json::to_vec(&serde_json::json!({"processed": {&first_id: ["legacy"]}})).unwrap(),
    )
    .unwrap();
    let first = open_bot(root.path(), || {
        daruda_store::inbox::migrate::remote(
            profile.path(),
            &first_id,
            channel_key(ChannelKind::Slack),
        )
    });
    let generation = cx.update(|cx| install_connection(&first_id, first.clone(), cx));
    let mut app = cx.to_async();
    let mut replies = 0;
    for _ in 0..2 {
        if let Some(bot) = claim_incoming(&first_id, generation, "event", &mut app).await {
            let incoming = transport::Incoming {
                event_id: "event".into(),
                sender: RemoteRecipient {
                    user_id: "user".into(),
                    conversation_id: "room".into(),
                    scope_id: "scope".into(),
                },
                kind: IncomingKind::Message {
                    text: "!list".into(),
                    reply_to: None,
                },
            };
            let effect =
                app.update(|cx| dispatch_claimed(&first_id, generation, &bot, &incoming, cx));
            assert!(matches!(effect, Some(Effect::Reply(_))));
            replies += 1;
        }
    }
    assert_eq!(replies, 1);
    cx.update(|cx| cx.global_mut::<RemoteChannels>().connections.clear());
    drop(first);
    let second = open_bot(root.path(), || {
        panic!("a new profile must not import its config id again")
    });
    let generation = cx.update(|cx| install_connection(&second_id, second.clone(), cx));
    assert!(
        claim_incoming(&second_id, generation, "event", &mut app)
            .await
            .is_none()
    );
    assert!(
        claim_incoming(&second_id, generation, "legacy", &mut app)
            .await
            .is_none()
    );
    assert_eq!(replies, 1);
    cx.update(|cx| cx.global_mut::<RemoteChannels>().connections.clear());
    drop(second);
    wait_release(root.path());
}

#[gpui::test]
async fn worker_revocation_while_a_claim_waits_prevents_effects_before_status_is_consumed(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let root = tempfile::tempdir().unwrap();
    let bot = open_bot(root.path(), || Ok(InboxSeed::default()));
    let generation = cx.update(|cx| install_connection("profile-local-id", bot.clone(), cx));
    let config = cx.update(|cx| {
        cx.global::<RemoteChannels>().connections["profile-local-id"]
            .config
            .clone()
    });
    let queued = crate::runs::RunsHost::claim_inbound(
        &bot.inbox,
        channel_key(ChannelKind::Slack),
        daruda_store::inbox::InboxId::Opaque("pending".into()),
    );
    let (entered, blocked) = smol::channel::bounded(1);
    let (release, resume) = smol::channel::bounded(1);
    let waiting_bot = bot.clone();
    let waiting = cx.update(|cx| {
        cx.spawn(async move |cx| {
            finish_claim(
                "profile-local-id",
                generation,
                waiting_bot,
                config,
                async move {
                    let committed = queued.await;
                    entered.send(()).await.unwrap();
                    resume.recv().await.unwrap();
                    committed
                },
                cx,
            )
            .await
        })
    });
    blocked.recv().await.unwrap();
    bot.revoke();
    cx.update(|cx| {
        let connection = &cx.global::<RemoteChannels>().connections["profile-local-id"];
        assert_eq!(
            connection.status,
            crate::remote_channel::runtime::Status::Connected
        );
        assert!(!connection.can_send());
    });
    assert!(
        InboxLease::try_acquire(root.path(), &lock::digest_of("shared-bot"))
            .unwrap()
            .is_none()
    );
    release.send(()).await.unwrap();
    assert!(waiting.await.is_none());
    assert!(matches!(
        smol::block_on(bot.inbox.claim(
            channel_key(ChannelKind::Slack),
            daruda_store::inbox::InboxId::Opaque("pending".into())
        )),
        Err(daruda_store::inbox::CommitError::Rejected(
            daruda_store::inbox::InboxRejected::Duplicate { .. }
        ))
    ));
    cx.update(|cx| cx.global_mut::<RemoteChannels>().connections.clear());
    drop(bot);
    wait_release(root.path());
}

fn callback() -> IncomingKind {
    IncomingKind::Callback {
        data: "token".into(),
        message_id: "12.34".into(),
        original: "Allow write?".into(),
    }
}

/// The defect this guards: a tap whose token is unknown or already consumed
/// used to get a fresh message, leaving the dead buttons on screen for the
/// user to keep tapping.
#[test]
fn a_dead_tap_is_answered_in_place_and_loses_its_buttons() {
    let Feedback::Edit {
        message_id,
        original,
        label,
    } = feedback_for(Effect::None, callback())
    else {
        panic!("a tapped button must be answered in place");
    };
    assert_eq!(message_id, "12.34");
    assert_eq!(original, "Allow write?");
    assert!(!label.is_empty());
}

#[test]
fn a_settled_decision_consumes_its_buttons_and_a_selection_keeps_them() {
    assert!(matches!(
        feedback_for(
            Effect::Feedback {
                label: "Allowed".into(),
                edit: Edit::ConsumeButtons,
            },
            callback(),
        ),
        Feedback::Edit { .. }
    ));
    // A listing row stays tappable — picking another row is ordinary use.
    assert_eq!(
        feedback_for(
            Effect::Feedback {
                label: "Now talking to lane 2".into(),
                edit: Edit::KeepButtons,
            },
            callback(),
        ),
        Feedback::Send(Message::plain("Now talking to lane 2".into(), None)),
    );
}

/// A plain message has no keyboard to strip, so nothing is owed unless the
/// effect produced an answer of its own.
#[test]
fn a_plain_message_is_never_edited() {
    let text = IncomingKind::Message {
        text: "ship it".into(),
        reply_to: None,
    };
    assert_eq!(feedback_for(Effect::None, text), Feedback::Nothing);
    assert_eq!(
        feedback_for(
            Effect::Reply(RenderedReply {
                text: "done".into(),
                keyboard: None,
            }),
            IncomingKind::Message {
                text: "ship it".into(),
                reply_to: None,
            },
        ),
        Feedback::Send(Message::plain("done".into(), None)),
    );
}
