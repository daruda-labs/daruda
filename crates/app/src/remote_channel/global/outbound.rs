use super::RemoteChannels;
use crate::remote_channel::{
    bridge::{InlineKeyboard, Outbound, PaneRef},
    transport::{self, Credentials, Message},
};
use daruda_config::remote::{ChannelKind, RemoteRecipient};
use futures::{
    StreamExt,
    channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded},
};
use gpui::App;
use std::{collections::HashMap, sync::Arc};

pub(crate) struct Pending {
    pub id: String,
    pub generation: uuid::Uuid,
    pub outbound: Outbound,
}

pub(super) struct Prepared {
    kind: ChannelKind,
    credentials: Arc<Credentials>,
    recipient: RemoteRecipient,
    message: Message,
    pane: Option<PaneRef>,
}

pub(super) fn prepare(pending: &Pending, cx: &mut App) -> Option<Prepared> {
    let connection = cx.global::<RemoteChannels>().live(&pending.id, cx)?;
    if connection.generation != pending.generation || !connection.can_send() {
        return None;
    }
    let recipient = connection.config.recipient.clone()?;
    let kind = connection.config.kind;
    let credentials = connection.credentials.clone()?;
    let connection = cx
        .global_mut::<RemoteChannels>()
        .connections
        .get_mut(&pending.id)?;
    let (message, pane) = match &pending.outbound {
        Outbound::Ping(ping) => {
            let prepared = connection.core.build_ping(ping.clone());
            (prepared, Some(ping.pane))
        }
        Outbound::Notice(text) => (Message::plain(text.clone(), None), None),
        Outbound::Approval(prompt) => (
            Message::plain(
                prompt.summary.clone(),
                Some(InlineKeyboard::single_row(prompt.buttons.to_vec())),
            ),
            None,
        ),
    };
    Some(Prepared {
        kind,
        credentials,
        recipient,
        message,
        pane,
    })
}

/// Each connection drains in order; a throttled channel does not block another.
pub(super) fn spawn(mut receiver: UnboundedReceiver<Pending>, cx: &mut App) {
    cx.spawn(async move |cx| {
        let mut queues: HashMap<String, UnboundedSender<Pending>> = HashMap::new();
        while let Some(pending) = receiver.next().await {
            let sender = queues.entry(pending.id.clone()).or_insert_with(|| {
                let (sender, receiver) = unbounded();
                cx.update(|cx| spawn_connection(receiver, cx));
                sender
            });
            if let Err(error) = sender.unbounded_send(pending) {
                crate::remote_channel::log_error(
                    "Remote connection queue closed",
                    &error,
                    "remote.queue.connection",
                );
            }
        }
    })
    .detach();
}

fn spawn_connection(mut receiver: UnboundedReceiver<Pending>, cx: &mut App) {
    cx.spawn(async move |cx| {
        while let Some(pending) = receiver.next().await {
            let Some(prepared) = cx.update(|cx| prepare(&pending, cx)) else {
                continue;
            };
            let pane = prepared.pane;
            let (ids, result) = cx
                .background_executor()
                .spawn(async move {
                    let mut sent = Vec::new();
                    let result = transport::send_recording(
                        prepared.kind,
                        &prepared.credentials,
                        &prepared.recipient,
                        prepared.message,
                        |id| sent.push(id.to_owned()),
                    );
                    (sent, result)
                })
                .await;
            // Keep IDs even when a later chunk fails: delivered chunks must remain replyable.
            cx.update(|cx| {
                if let Some(connection) = cx
                    .global_mut::<RemoteChannels>()
                    .connections
                    .get_mut(&pending.id)
                    && connection.generation == pending.generation
                    && let Some(pane) = pane
                {
                    for id in ids {
                        connection.core.record_sent(id, pane);
                    }
                }
            });
            if let Err(error) = result {
                crate::remote_channel::log_error(
                    "Remote message failed to send",
                    &error,
                    "remote.message.send",
                );
            }
        }
    })
    .detach();
}
