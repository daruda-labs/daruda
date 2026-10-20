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

/// Claim through the production inbox path, as `Some(bot)` when it may act.
async fn claim_incoming(
    id: &str,
    generation: uuid::Uuid,
    event_id: &str,
    cx: &mut gpui::AsyncApp,
) -> Option<Arc<BotSession>> {
    match super::claim_incoming(
        id,
        generation,
        event_id,
        &crate::runs::RunsHost::claim_inbound,
        cx,
    )
    .await
    {
        Claimed::Ready(bot) => Some(bot),
        Claimed::Skip | Claimed::Retry => None,
    }
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

fn reopen_requested(id: &str, cx: &mut TestAppContext) -> bool {
    cx.update(
        |cx| match &cx.global::<RemoteChannels>().connections[id].worker {
            super::super::WorkerState::Running { worker } => worker.reopen_requested(),
            _ => panic!("the connection's worker was replaced"),
        },
    )
}

/// The defect this guards: recovery used to start a second worker while the
/// first still held the bot's lease, and the newcomer reported this process's
/// own bot as held elsewhere. The same worker now reopens its own writer.
#[gpui::test]
async fn a_failed_claim_has_its_own_worker_reopen_the_inbox_without_losing_history(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let root = tempfile::tempdir().unwrap();
    let bot = open_bot(root.path(), || Ok(InboxSeed::default()));
    let generation = cx.update(|cx| install_connection("work", bot.clone(), cx));
    cx.update(|cx| run_worker("work", cx));
    let target = crate::remote_channel::bridge::PaneRef {
        workspace: Default::default(),
        pane: 7,
    };
    cx.update(|cx| {
        cx.global_mut::<RemoteChannels>()
            .connections
            .get_mut("work")
            .unwrap()
            .core
            .command_state_mut()
            .select(Some(target))
    });
    let mut app = cx.to_async();
    assert!(
        claim_incoming("work", generation, "committed", &mut app)
            .await
            .is_some()
    );
    let config = cx.update(|cx| {
        cx.global::<RemoteChannels>().connections["work"]
            .config
            .clone()
    });
    let result = finish_claim(
        "work",
        generation,
        bot.clone(),
        config,
        std::future::ready(Err(daruda_store::inbox::CommitError::Io(
            std::io::Error::other("temporary storage failure"),
        ))),
        &mut app,
    )
    .await;
    assert!(matches!(result, Claimed::Retry));
    cx.update(|cx| {
        let connection = &cx.global::<RemoteChannels>().connections["work"];
        assert_eq!(
            connection.status,
            crate::remote_channel::runtime::Status::Failed
        );
        assert_eq!(connection.generation, generation);
        assert!(connection.bot.is_none());
        assert!(!connection.can_send());
    });
    assert!(
        !reopen_requested("work", cx),
        "the reopen waits out the claim backoff"
    );
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    cx.run_until_parked();
    assert!(reopen_requested("work", cx));
    cx.update(|cx| {
        assert_eq!(
            cx.global_mut::<RemoteChannels>()
                .connections
                .get_mut("work")
                .unwrap()
                .core
                .command_state_mut()
                .selected(),
            Some(target)
        )
    });
    assert!(!bot.is_live());
    assert!(
        InboxLease::try_acquire(root.path(), &lock::digest_of("shared-bot"))
            .unwrap()
            .is_none()
    );
    drop(bot);
    let reopened = open_bot(root.path(), || panic!("recovery must not reimport"));
    let generation = cx.update(|cx| {
        let connection = cx
            .global_mut::<RemoteChannels>()
            .connections
            .get_mut("work")
            .unwrap();
        connection.bot = Some(reopened.clone());
        connection.status = crate::remote_channel::runtime::Status::Connected;
        connection.generation
    });
    assert!(
        claim_incoming("work", generation, "committed", &mut app)
            .await
            .is_none()
    );
    assert!(
        claim_incoming("work", generation, "new", &mut app)
            .await
            .is_some()
    );
    cx.update(|cx| cx.global_mut::<RemoteChannels>().connections.clear());
    drop(reopened);
    wait_release(root.path());
}

#[gpui::test]
fn an_old_inbox_failure_cannot_restart_a_replacement_or_finish_credential_updates(
    cx: &mut TestAppContext,
) {
    cx.update(|cx| {
        let first_root = tempfile::tempdir().unwrap();
        let old = open_bot(first_root.path(), || Ok(InboxSeed::default()));
        let generation = install_connection("work", old.clone(), cx);
        let config = cx.global::<RemoteChannels>().connections["work"]
            .config
            .clone();
        RemoteChannels::suspend(config.clone(), cx);
        RemoteChannels::suspend(config, cx);
        let current_generation = cx.global::<RemoteChannels>().connections["work"].generation;
        RemoteChannels::recover_inbox("work", generation, &old, cx);
        assert_eq!(
            cx.global::<RemoteChannels>().connections["work"].generation,
            current_generation
        );
        assert!(matches!(
            cx.global::<RemoteChannels>().connections["work"].worker,
            super::super::WorkerState::UpdatingCredential { pending: 2 }
        ));
        let current_root = tempfile::tempdir().unwrap();
        let current = open_bot(current_root.path(), || Ok(InboxSeed::default()));
        let current_generation = install_connection("work", current.clone(), cx);
        RemoteChannels::recover_inbox("work", generation, &old, cx);
        let connection = &cx.global::<RemoteChannels>().connections["work"];
        assert_eq!(connection.generation, current_generation);
        assert!(connection.can_send());
        assert!(current.is_live());
        cx.global_mut::<RemoteChannels>().connections.clear();
        drop(old);
        drop(current);
        wait_release(first_root.path());
        wait_release(current_root.path());
    });
}

/// Stand in a running worker with no thread behind it.
fn run_worker(id: &str, cx: &mut App) {
    cx.global_mut::<RemoteChannels>()
        .connections
        .get_mut(id)
        .unwrap()
        .worker = super::super::WorkerState::Running {
        worker: crate::remote_channel::runtime::Worker::detached(),
    };
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
    assert!(matches!(waiting.await, Claimed::Skip));
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
    }
}

#[test]
fn a_dead_tap_reports_staleness_without_overwriting_the_settled_result() {
    let Feedback::Send(message) = feedback_for(Effect::None, callback()) else {
        panic!("a stale tap must receive feedback");
    };
    assert_eq!(
        message,
        Message::plain(s::remote_channel::stale_callback(), None)
    );
}

#[test]
fn a_managed_card_leaves_editing_to_the_ledger_and_a_selection_keeps_its_buttons() {
    assert!(matches!(
        feedback_for(
            Effect::Feedback {
                label: "Allowed".into(),
                edit: Edit::ManagedCard,
            },
            callback(),
        ),
        Feedback::Nothing
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

fn from(user: &str, event_id: &str, generation: uuid::Uuid) -> WorkerEvent {
    WorkerEvent {
        id: "work".into(),
        generation,
        payload: WorkerPayload::Incoming(transport::Incoming {
            event_id: event_id.into(),
            sender: RemoteRecipient {
                user_id: user.into(),
                conversation_id: "room".into(),
                scope_id: "scope".into(),
            },
            kind: IncomingKind::Message {
                text: "hello".into(),
                reply_to: None,
            },
        }),
    }
}

/// Slack and Discord acknowledge an event before it reaches the claim, so a
/// command dropped on a failed claim is never sent again. It waits for the
/// reopened inbox instead, with the commands after it queued behind it.
/// The sender is a stranger so the claimed commands stop short of a reply.
#[gpui::test]
async fn a_command_whose_claim_failed_is_claimed_again_on_the_reopened_inbox(
    cx: &mut TestAppContext,
) {
    use std::{cell::RefCell, rc::Rc};

    cx.executor().allow_parking();
    let root = tempfile::tempdir().unwrap();
    let first = open_bot(root.path(), || Ok(InboxSeed::default()));
    let generation = cx.update(|cx| install_connection("work", first.clone(), cx));
    cx.update(|cx| run_worker("work", cx));
    let attempts = Rc::new(RefCell::new(Vec::<String>::new()));
    let recorded = attempts.clone();
    let (events, receiver) = futures::channel::mpsc::unbounded();
    cx.update(|cx| {
        serve_events(
            receiver,
            move |inbox: &InboxStore, channel, id| {
                let event = match &id {
                    InboxId::Opaque(event) => event.clone(),
                    InboxId::Monotonic(update) => update.to_string(),
                };
                let fail = recorded.borrow().is_empty();
                recorded.borrow_mut().push(event);
                if fail {
                    futures::future::Either::Left(std::future::ready(Err(CommitError::Io(
                        std::io::Error::other("temporary storage failure"),
                    ))))
                } else {
                    futures::future::Either::Right(inbox.claim(channel, id))
                }
            },
            cx,
        )
    });

    events
        .unbounded_send(from("stranger", "first", generation))
        .unwrap();
    events
        .unbounded_send(from("stranger", "second", generation))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        *attempts.borrow(),
        ["first"],
        "a later command must not be claimed ahead of a held one"
    );
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    cx.run_until_parked();
    assert!(reopen_requested("work", cx));

    drop(first);
    let reopened = open_bot(root.path(), || panic!("recovery must not reimport"));
    events
        .unbounded_send(WorkerEvent {
            id: "work".into(),
            generation,
            payload: WorkerPayload::Ready(reopened.clone()),
        })
        .unwrap();
    // A real claim completes on the inbox's writer thread, outside the
    // executor, so parking alone does not mean the backlog has drained.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while attempts.borrow().len() < 3 && std::time::Instant::now() < deadline {
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(*attempts.borrow(), ["first", "first", "second"]);
    for event in ["first", "second"] {
        assert!(matches!(
            smol::block_on(reopened.inbox.claim(
                channel_key(ChannelKind::Slack),
                InboxId::Opaque(event.into())
            )),
            Err(CommitError::Rejected(InboxRejected::Duplicate { .. }))
        ));
    }
    cx.update(|cx| cx.global_mut::<RemoteChannels>().connections.clear());
    drop(events);
    cx.run_until_parked();
    drop(reopened);
    wait_release(root.path());
}
