use super::*;
use crate::remote_channel::transport::IncomingKind;
use daruda_config::remote::RemoteRecipient;
use gpui::TestAppContext;

/// A stalled durable writer is the same await boundary as throttled feedback:
/// neither may stall another connection or the shared worker-status receiver.
#[gpui::test]
async fn independent_connections_keep_serving_while_one_claim_waits(cx: &mut TestAppContext) {
    use std::{cell::RefCell, rc::Rc};
    cx.executor().allow_parking();
    let _outbound = cx.update(crate::remote_channel::global::tests::setup);
    let (work, personal) = cx.update(|cx| {
        let channels = cx.global::<RemoteChannels>();
        (
            channels.connections["work"].generation,
            channels.connections["personal"].generation,
        )
    });
    let (release, resume) = smol::channel::bounded(1);
    let claimed = Rc::new(RefCell::new(Vec::new()));
    let recorded = claimed.clone();
    let (events, receiver) = futures::channel::mpsc::unbounded();
    cx.update(|cx| {
        serve_events(
            receiver,
            move |inbox: &InboxStore, channel, id| {
                let first = id == InboxId::Opaque("slow".into());
                let committed = inbox.claim(channel, id.clone());
                let resume = resume.clone();
                let recorded = recorded.clone();
                async move {
                    if first {
                        resume.recv().await.unwrap();
                    }
                    recorded.borrow_mut().push(id);
                    committed.await
                }
            },
            cx,
        )
    });
    for (id, generation, event) in [
        ("work", work, "slow"),
        ("work", work, "next"),
        ("personal", personal, "independent"),
    ] {
        events
            .unbounded_send(WorkerEvent {
                id: id.into(),
                generation,
                payload: WorkerPayload::Incoming(transport::Incoming {
                    event_id: event.into(),
                    sender: RemoteRecipient {
                        user_id: "stranger".into(),
                        conversation_id: "2".into(),
                        scope_id: "3".into(),
                    },
                    kind: IncomingKind::Message {
                        text: "!list".into(),
                        reply_to: None,
                    },
                }),
            })
            .unwrap();
    }
    events
        .unbounded_send(WorkerEvent {
            id: "personal".into(),
            generation: personal,
            payload: WorkerPayload::Status(crate::remote_channel::runtime::Status::Connected),
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        *claimed.borrow(),
        [InboxId::Opaque("independent".into())],
        "another connection must not wait for the slow claim"
    );
    cx.update(|cx| {
        assert_eq!(
            cx.global::<RemoteChannels>().connections["personal"].status,
            crate::remote_channel::runtime::Status::Connected
        )
    });
    release.send(()).await.unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while claimed.borrow().len() < 3 && std::time::Instant::now() < deadline {
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(
        *claimed.borrow(),
        [
            InboxId::Opaque("independent".into()),
            InboxId::Opaque("slow".into()),
            InboxId::Opaque("next".into())
        ]
    );
    cx.update(|cx| cx.global_mut::<RemoteChannels>().connections.clear());
    drop(events);
}

#[gpui::test]
async fn a_healthy_channels_approval_and_reconnect_do_not_wait_for_another_claim(
    cx: &mut TestAppContext,
) {
    use crate::control::approval::{self, ApprovalOutcome};
    use crate::remote_channel::{
        bridge::Outbound,
        global::{
            Delivery,
            tests::{ready_bot_as, setup},
        },
        runtime::Status,
    };
    cx.executor().allow_parking();
    let mut outbound = cx.update(setup);
    let (work, personal) = cx.update(|cx| {
        let channels = cx.global::<RemoteChannels>();
        (
            channels.connections["work"].generation,
            channels.connections["personal"].generation,
        )
    });
    let (release, resume) = smol::channel::bounded(1);
    let (events, receiver) = futures::channel::mpsc::unbounded();
    cx.update(|cx| {
        serve_events(
            receiver,
            move |inbox: &InboxStore, channel, id| {
                let slow = id == InboxId::Opaque("slow".into());
                let committed = inbox.claim(channel, id);
                let resume = resume.clone();
                async move {
                    if slow {
                        resume.recv().await.unwrap();
                    }
                    committed.await
                }
            },
            cx,
        )
    });
    events
        .unbounded_send(WorkerEvent {
            id: "work".into(),
            generation: work,
            payload: WorkerPayload::Incoming(transport::Incoming {
                event_id: "slow".into(),
                sender: RemoteRecipient {
                    user_id: "stranger".into(),
                    conversation_id: "2".into(),
                    scope_id: "3".into(),
                },
                kind: IncomingKind::Message {
                    text: "!list".into(),
                    reply_to: None,
                },
            }),
        })
        .unwrap();
    cx.run_until_parked();
    let replacement = ready_bot_as("reconnected");
    events
        .unbounded_send(WorkerEvent {
            id: "personal".into(),
            generation: personal,
            payload: WorkerPayload::Ready(replacement.clone()),
        })
        .unwrap();
    events
        .unbounded_send(WorkerEvent {
            id: "personal".into(),
            generation: personal,
            payload: WorkerPayload::Status(Status::Connected),
        })
        .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let connection = &cx.global::<RemoteChannels>().connections["personal"];
        assert!(std::sync::Arc::ptr_eq(
            connection.bot.as_ref().unwrap(),
            &replacement
        ));
        assert_eq!(connection.status, Status::Connected);
    });
    let (_, answer) =
        cx.update(|cx| approval::request("Create worktree".into(), Delivery::Explicit, cx));
    let pending = loop {
        let pending = outbound.try_recv().unwrap();
        if pending.id == "personal" {
            break pending;
        }
    };
    let Outbound::Approval(prompt) = pending.outbound else {
        panic!("approval card");
    };
    events
        .unbounded_send(WorkerEvent {
            id: "personal".into(),
            generation: personal,
            payload: WorkerPayload::Incoming(transport::Incoming {
                event_id: "approve".into(),
                sender: pending.recipient,
                kind: IncomingKind::Callback {
                    data: prompt.buttons[0].1.clone(),
                    message_id: "approval-message".into(),
                    original: prompt.summary,
                },
            }),
        })
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while answer.is_empty() && std::time::Instant::now() < deadline {
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(
        answer.try_recv(),
        Ok(ApprovalOutcome::Approved),
        "the healthy channel must settle the real approval while work is blocked"
    );
    release.send(()).await.unwrap();
    cx.run_until_parked();
    cx.update(|cx| cx.global_mut::<RemoteChannels>().connections.clear());
    drop(events);
}

mod recovery;
