use super::*;
use std::{cell::RefCell, rc::Rc};

#[gpui::test]
async fn a_ready_queued_before_claim_failure_cannot_release_the_failed_command(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let _outbound = cx.update(crate::remote_channel::global::tests::setup);
    let (generation, old_bot) = cx.update(|cx| {
        let connection = &cx.global::<RemoteChannels>().connections["work"];
        (connection.generation, connection.bot.clone().unwrap())
    });
    let (release, resume) = smol::channel::bounded(1);
    let attempted = Rc::new(RefCell::new(Vec::new()));
    let recorded = attempted.clone();
    let (events, receiver) = futures::channel::mpsc::unbounded();
    cx.update(|cx| {
        serve_events(
            receiver,
            move |inbox: &InboxStore, channel, id| {
                let fail = recorded.borrow().is_empty();
                recorded.borrow_mut().push(id.clone());
                let committed = (!fail).then(|| inbox.claim(channel, id));
                let resume = resume.clone();
                async move {
                    match committed {
                        Some(committed) => committed.await,
                        None => {
                            resume.recv().await.unwrap();
                            Err(daruda_store::inbox::CommitError::Io(std::io::Error::other(
                                "temporary failure",
                            )))
                        }
                    }
                }
            },
            cx,
        )
    });
    events
        .unbounded_send(WorkerEvent {
            id: "work".into(),
            generation,
            payload: WorkerPayload::Incoming(transport::Incoming {
                event_id: "retry-me".into(),
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
    assert_eq!(attempted.borrow().len(), 1);
    // A reconnect publishes the still-current session before its writer fails.
    events
        .unbounded_send(WorkerEvent {
            id: "work".into(),
            generation,
            payload: WorkerPayload::Ready(old_bot.clone()),
        })
        .unwrap();
    cx.run_until_parked();
    release.send(()).await.unwrap();
    cx.run_until_parked();
    assert!(!old_bot.is_live());
    assert_eq!(
        attempted.borrow().len(),
        1,
        "the failed writer has not reopened yet"
    );
    events
        .unbounded_send(WorkerEvent {
            id: "work".into(),
            generation,
            payload: WorkerPayload::Ready(crate::remote_channel::global::tests::ready_bot()),
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        *attempted.borrow(),
        [
            InboxId::Opaque("retry-me".into()),
            InboxId::Opaque("retry-me".into())
        ],
        "only a fresh live Ready may resume the held command"
    );
    cx.update(|cx| cx.global_mut::<RemoteChannels>().connections.clear());
    drop(events);
}
