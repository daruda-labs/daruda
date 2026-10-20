use super::*;
use daruda_config::remote::{ChannelKind, RemoteRecipient};
use futures::{FutureExt, StreamExt};

pub(super) fn ready_bot() -> Arc<BotSession> {
    let root = tempfile::tempdir().unwrap().keep();
    let lease = daruda_store::inbox::InboxLease::try_acquire(
        &root,
        &crate::remote_channel::lock::digest_of("fixture-bot"),
    )
    .unwrap()
    .unwrap();
    let inbox =
        daruda_store::inbox::InboxStore::open(lease, "test", || Ok(Default::default())).unwrap();
    Arc::new(BotSession::new(
        Arc::new(crate::remote_channel::transport::Credentials {
            bot: "fake".into(),
            app: Some("fake".into()),
        }),
        Arc::new(inbox),
        Default::default(),
    ))
}

pub(crate) fn setup(cx: &mut App) -> futures::channel::mpsc::UnboundedReceiver<Pending> {
    SettingsStore::init(cx);
    let mut config = daruda_config::Config::default();
    for (id, kind, only_when_away) in [
        ("work", ChannelKind::Slack, true),
        ("personal", ChannelKind::Discord, false),
    ] {
        let mut channel = ChannelConfig::new(id.into(), kind);
        channel.enabled = true;
        channel.only_when_away = only_when_away;
        channel.recipient = Some(RemoteRecipient {
            user_id: "1".into(),
            conversation_id: "2".into(),
            scope_id: "3".into(),
        });
        config.remote.channels.push(channel);
    }
    cx.global_mut::<SettingsStore>()
        .set_user_for_testing(config);
    let (events, _) = unbounded();
    let (outbound, receiver) = unbounded();
    cx.set_global(RemoteChannels {
        connections: HashMap::new(),
        events,
        outbound,
        applied: None,
    });
    reconcile(cx);
    for connection in cx.global_mut::<RemoteChannels>().connections.values_mut() {
        connection.bot = Some(ready_bot());
    }
    receiver
}

fn ping() -> BridgePing {
    BridgePing {
        pane: super::super::bridge::PaneRef {
            workspace: Default::default(),
            pane: 1,
        },
        header: "project".into(),
        tail: super::super::bridge::MessageTail::Plain("done".into()),
        permission: None,
    }
}

#[gpui::test]
fn credential_suspension_survives_removal_invalid_config_and_absent_rows(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(|cx| {
        let _outgoing = setup(cx);
        let config = cx.global::<RemoteChannels>().connections["work"]
            .config
            .clone();
        RemoteChannels::suspend(config.clone(), cx);
        let mut user = SettingsStore::global(cx).user().clone();
        user.remote.channels.clear();
        cx.global_mut::<SettingsStore>()
            .set_user_for_testing(user.clone());
        reconcile(cx);
        assert!(matches!(
            cx.global::<RemoteChannels>().connections["work"].worker,
            WorkerState::UpdatingCredential { .. }
        ));
        user.remote.channels = vec![config.clone(), config.clone()];
        cx.global_mut::<SettingsStore>()
            .set_user_for_testing(user.clone());
        reconcile(cx);
        assert!(matches!(
            cx.global::<RemoteChannels>().connections["work"].worker,
            WorkerState::UpdatingCredential { .. }
        ));
        let mut absent = ChannelConfig::new("new-row".into(), ChannelKind::Slack);
        RemoteChannels::suspend(absent.clone(), cx);
        absent.enabled = true;
        user.remote.channels = vec![absent];
        cx.global_mut::<SettingsStore>().set_user_for_testing(user);
        reconcile(cx);
        let connection = &cx.global::<RemoteChannels>().connections["new-row"];
        assert!(matches!(
            connection.worker,
            WorkerState::UpdatingCredential { .. }
        ));
        assert!(connection.bot.is_none());
        RemoteChannels::restart("new-row", cx);
        let connection = &cx.global::<RemoteChannels>().connections["new-row"];
        assert!(matches!(connection.worker, WorkerState::Idle));
        assert!(connection.config.enabled);
        assert!(connection.bot.is_none());
    });
}

#[gpui::test]
fn overlapping_credential_writes_resume_only_after_both_complete(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        let _outgoing = setup(cx);
        let channel = cx.global::<RemoteChannels>().connections["work"]
            .config
            .clone();
        RemoteChannels::suspend(channel.clone(), cx);
        RemoteChannels::suspend(channel.clone(), cx);
        RemoteChannels::restart("work", cx);
        let connection = &cx.global::<RemoteChannels>().connections["work"];
        assert!(matches!(
            connection.worker,
            WorkerState::UpdatingCredential { pending: 1 }
        ));
        assert!(connection.bot.is_none());
        let mut user = SettingsStore::global(cx).user().clone();
        user.remote.channels.clear();
        cx.global_mut::<SettingsStore>()
            .set_user_for_testing(user.clone());
        reconcile(cx);
        assert!(matches!(
            cx.global::<RemoteChannels>().connections["work"].worker,
            WorkerState::UpdatingCredential { pending: 1 }
        ));
        user.remote.channels.push(channel);
        cx.global_mut::<SettingsStore>().set_user_for_testing(user);
        reconcile(cx);
        RemoteChannels::restart("work", cx);
        let connection = &cx.global::<RemoteChannels>().connections["work"];
        assert!(matches!(connection.worker, WorkerState::Idle));
        assert!(connection.bot.is_none());
    });
}

/// A worker that gave up holds no claim either, and the bot it let go is the
/// next daruda's to take. Reading "not explicitly held elsewhere" as
/// permission to send would leave a dead connection speaking into a bot
/// someone else is now serving.
#[gpui::test]
fn a_channel_whose_worker_gave_up_sends_nothing(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        let mut receiver = setup(cx);
        cx.global_mut::<RemoteChannels>()
            .connections
            .get_mut("personal")
            .expect("the always-relaying channel")
            .status = Status::Failed;

        assert!(!RemoteChannels::send_ping(
            ping(),
            Delivery::Presence { away: false },
            cx
        ));
        assert!(receiver.next().now_or_never().is_none());
    });
}

/// A bot another daruda is serving is not this one's to speak into: two
/// instances both sending would double every ping, and each would remember
/// only its own as the chat a plain reply belongs to.
#[gpui::test]
fn a_channel_another_daruda_holds_sends_nothing(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        let mut receiver = setup(cx);
        cx.global_mut::<RemoteChannels>()
            .connections
            .get_mut("personal")
            .expect("the always-relaying channel")
            .status = Status::HeldElsewhere;

        assert!(
            !RemoteChannels::send_ping(ping(), Delivery::Presence { away: false }, cx),
            "nothing was queued, so nothing was sent"
        );
        assert!(receiver.next().now_or_never().is_none());
    });
}

#[gpui::test]
fn presence_is_per_connection_and_revocation_cancels_queued_work(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        let mut receiver = setup(cx);
        assert!(RemoteChannels::send_ping(
            ping(),
            Delivery::Presence { away: false },
            cx
        ));
        let pending = receiver.next().now_or_never().flatten().unwrap();
        assert_eq!(pending.id, "personal");
        assert!(receiver.next().now_or_never().is_none());
        let mut config = SettingsStore::global(cx).user().clone();
        config.remote.channels[1].recipient = None;
        cx.global_mut::<SettingsStore>()
            .set_user_for_testing(config);
        assert!(outbound::prepare(&pending, cx).is_none());
    });
}

#[gpui::test]
fn a_queued_message_cannot_move_to_a_replacement_session_with_the_same_config(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(|cx| {
        let mut receiver = setup(cx);
        assert!(RemoteChannels::send_ping(
            ping(),
            Delivery::Presence { away: false },
            cx
        ));
        let pending = receiver.next().now_or_never().flatten().unwrap();
        assert!(outbound::prepare(&pending, cx).is_some());
        let connection = cx
            .global_mut::<RemoteChannels>()
            .connections
            .get_mut("personal")
            .unwrap();
        connection.bot.as_ref().unwrap().revoke();
        connection.bot = Some(ready_bot());
        assert!(outbound::prepare(&pending, cx).is_none());
    });
}

#[gpui::test]
fn a_presence_preference_keeps_routing_but_a_new_recipient_invalidates_it(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(|cx| {
        let _receiver = setup(cx);
        let pane = ping().pane;
        let connection = cx
            .global_mut::<RemoteChannels>()
            .connections
            .get_mut("work")
            .unwrap();
        let generation = connection.generation;
        connection.core.command_state_mut().select(Some(pane));
        let mut config = SettingsStore::global(cx).user().clone();
        config.remote.channels[0].only_when_away = false;
        cx.global_mut::<SettingsStore>()
            .set_user_for_testing(config.clone());
        reconcile(cx);
        let connection = cx
            .global_mut::<RemoteChannels>()
            .connections
            .get_mut("work")
            .unwrap();
        assert_eq!(connection.generation, generation);
        assert!(matches!(
            connection.core.route_text("hello".into(), None),
            super::super::bridge::Routed::Ready(
                super::super::bridge::InboundAction::InjectPrompt { .. }
            )
        ));
        config.remote.channels[0]
            .recipient
            .as_mut()
            .unwrap()
            .user_id = "other".into();
        cx.global_mut::<SettingsStore>()
            .set_user_for_testing(config);
        reconcile(cx);
        let connection = cx
            .global_mut::<RemoteChannels>()
            .connections
            .get_mut("work")
            .unwrap();
        assert_ne!(connection.generation, generation);
        assert!(matches!(
            connection.core.route_text("hello".into(), None),
            super::super::bridge::Routed::NeedsTarget(_)
        ));
    });
}

#[gpui::test]
async fn a_shared_approval_settles_once_and_invalidates_every_channels_buttons(
    cx: &mut gpui::TestAppContext,
) {
    let mut receiver = cx.update(setup);
    let (id, answer) = cx.update(|cx| {
        crate::control::approval::request("create lane".into(), Delivery::Explicit, cx)
    });
    let first = receiver.next().await.unwrap();
    let second = receiver.next().await.unwrap();
    assert_ne!(first.id, second.id);
    cx.update(|cx| {
        for connection in cx.global::<RemoteChannels>().connections.values() {
            assert_eq!(connection.core.pending_approval_token_count(), 2);
        }
        assert!(crate::control::approval::resolve(
            id,
            crate::control::approval::ApprovalChoice::Approved,
            cx
        ));
        assert!(!crate::control::approval::resolve(
            id,
            crate::control::approval::ApprovalChoice::Refused,
            cx
        ));
        for connection in cx.global::<RemoteChannels>().connections.values() {
            assert_eq!(connection.core.pending_approval_token_count(), 0);
        }
    });
    assert_eq!(
        answer.recv().await.unwrap(),
        crate::control::approval::ApprovalOutcome::Approved
    );
}

#[gpui::test]
fn approval_cards_respect_each_connections_presence_policy(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        let mut receiver = setup(cx);
        for (delivery, expected) in [
            (Delivery::Presence { away: false }, vec!["personal"]),
            (Delivery::Presence { away: true }, vec!["personal", "work"]),
            (Delivery::Explicit, vec!["personal", "work"]),
        ] {
            let id = crate::control::approval::ApprovalId(7);
            assert!(RemoteChannels::send_approval(
                id,
                "Create a task".into(),
                delivery,
                cx
            ));
            let mut delivered = Vec::new();
            while let Some(Some(pending)) = receiver.next().now_or_never() {
                assert!(matches!(pending.outbound, Outbound::Approval(_)));
                delivered.push(pending.id);
            }
            delivered.sort();
            assert_eq!(delivered, expected);
            for (key, connection) in &cx.global::<RemoteChannels>().connections {
                assert_eq!(
                    connection.core.pending_approval_token_count(),
                    if expected.contains(&key.as_str()) {
                        2
                    } else {
                        0
                    }
                );
            }
            RemoteChannels::settle_card(
                &crate::remote_channel::cards::CardId::Approval(id),
                "Cancelled",
                cx,
            );
        }
        drop(receiver);
        assert!(!RemoteChannels::send_approval(
            crate::control::approval::ApprovalId(9),
            "Create a task".into(),
            Delivery::Explicit,
            cx
        ));
        assert!(
            cx.global::<RemoteChannels>()
                .connections
                .values()
                .all(|connection| connection.core.pending_approval_token_count() == 0)
        );
    });
}

#[gpui::test]
fn disabled_or_revoked_connections_cannot_deliver(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        SettingsStore::init(cx);
        let mut config = daruda_config::Config::default();
        let mut channel = ChannelConfig::new("work".into(), ChannelKind::Slack);
        channel.enabled = true;
        channel.recipient = Some(RemoteRecipient {
            user_id: "U1".into(),
            conversation_id: "D1".into(),
            scope_id: "T1".into(),
        });
        config.remote.channels.push(channel);
        cx.global_mut::<SettingsStore>()
            .set_user_for_testing(config.clone());
        let (events, _) = unbounded();
        let (outbound, _receiver) = unbounded();
        cx.set_global(RemoteChannels {
            connections: HashMap::new(),
            events,
            outbound,
            applied: None,
        });
        reconcile(cx);
        let connection = cx
            .global_mut::<RemoteChannels>()
            .connections
            .get_mut("work")
            .unwrap();
        connection.bot = Some(ready_bot());
        assert!(RemoteChannels::has_recipient(cx));
        config.remote.channels[0].enabled = false;
        cx.global_mut::<SettingsStore>()
            .set_user_for_testing(config);
        assert!(
            !RemoteChannels::has_recipient(cx),
            "live config revokes before the worker refreshes"
        );
    });
}

/// The announcement targets every live connection at once — `selected` on
/// each, and an explicit ping the `only_when_away` connection receives too,
/// because the user approved this a moment ago.
#[gpui::test]
fn announcing_a_chat_selects_it_on_every_connection_and_pings_all(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        let mut receiver = setup(cx);
        let pane = ping().pane;
        assert!(RemoteChannels::announce_chat(
            pane,
            "daruda/main".into(),
            "new tab".into(),
            Delivery::Explicit,
            cx
        ));
        let bridge = cx.global_mut::<RemoteChannels>();
        for id in ["work", "personal"] {
            assert_eq!(
                bridge
                    .connections
                    .get_mut(id)
                    .unwrap()
                    .core
                    .command_state_mut()
                    .selected(),
                Some(pane),
                "{id}"
            );
        }
        let mut pinged: Vec<String> = Vec::new();
        while let Some(pending) = receiver.next().now_or_never().flatten() {
            assert!(
                matches!(pending.outbound, Outbound::Ping(ref p) if p.pane == pane),
                "the announcement is a ping attributed to the new chat"
            );
            pinged.push(pending.id);
        }
        pinged.sort();
        assert_eq!(pinged, ["personal", "work"]);
    });
}

#[gpui::test]
fn presence_announcements_preserve_targets_and_use_each_connections_presence_setting(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(|cx| {
        let mut receiver = setup(cx);
        let selected = ping().pane;
        let announced = PaneRef {
            pane: 2,
            ..selected
        };
        for connection in cx.global_mut::<RemoteChannels>().connections.values_mut() {
            connection.core.command_state_mut().select(Some(selected));
        }
        assert!(RemoteChannels::announce_chat(
            announced,
            "header".into(),
            "announcement".into(),
            Delivery::Presence { away: false },
            cx,
        ));
        let pending = receiver
            .next()
            .now_or_never()
            .flatten()
            .expect("personal accepts while present");
        assert_eq!(pending.id, "personal");
        assert!(
            matches!(pending.outbound, Outbound::Notice(text) if text == "header\nannouncement")
        );
        assert!(receiver.next().now_or_never().is_none());
        for connection in cx.global_mut::<RemoteChannels>().connections.values_mut() {
            assert_eq!(
                connection.core.command_state_mut().selected(),
                Some(selected)
            );
        }
    });
}
