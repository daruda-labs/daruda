use super::RemoteChannels;
use crate::remote_channel::{
    bridge::{InlineKeyboard, MessageTail, Outbound, PaneRef},
    cards::{CardId, SendClaim},
    runtime::BotSession,
    transport::{self, Message},
};
use daruda_config::remote::{ChannelKind, RemoteRecipient};
use futures::{
    StreamExt,
    channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded},
};
use gpui::App;
use std::{
    collections::HashMap,
    sync::{Arc, Weak},
};

pub(crate) struct Pending {
    pub id: String,
    pub generation: uuid::Uuid,
    pub bot: Arc<BotSession>,
    pub recipient: RemoteRecipient,
    pub outbound: Outbound,
    pub card: Option<SendClaim>,
}

pub(super) struct Receipt {
    id: String,
    generation: uuid::Uuid,
    bot: Weak<BotSession>,
    recipient: RemoteRecipient,
    ids: Vec<String>,
    original: String,
}

pub(super) struct Prepared {
    kind: ChannelKind,
    bot: Arc<BotSession>,
    recipient: RemoteRecipient,
    message: Message,
    pane: Option<PaneRef>,
    original: String,
}

pub(super) fn prepare(pending: &Pending, cx: &mut App) -> Option<Prepared> {
    let valid = cx
        .global::<RemoteChannels>()
        .live(&pending.id, cx)
        .is_some_and(|connection| {
            connection.generation == pending.generation
                && connection.can_send()
                && connection.config.recipient.as_ref() == Some(&pending.recipient)
                && connection
                    .bot
                    .as_ref()
                    .is_some_and(|bot| Arc::ptr_eq(bot, &pending.bot))
        });
    if !valid {
        discard(pending, cx);
        return None;
    }
    let connection = cx
        .global_mut::<RemoteChannels>()
        .connections
        .get_mut(&pending.id)?;
    if let Some(card) = &pending.card
        && !connection.cards.borrow_mut().begin(card)
    {
        return None;
    }
    let kind = connection.config.kind;
    let (message, pane) = match &pending.outbound {
        Outbound::Ping(ping) => (connection.core.build_ping(ping.clone()), Some(ping.pane)),
        Outbound::Notice(text) => (Message::plain(text.clone(), None), None),
        Outbound::FlowAsk(prompt) => (connection.core.build_flow_ask(prompt.clone()), None),
        Outbound::Approval(prompt) => (
            Message::plain(
                prompt.summary.clone(),
                Some(InlineKeyboard::single_row(prompt.buttons.to_vec())),
            ),
            None,
        ),
    };
    let tail = match &message.tail {
        MessageTail::Plain(text) | MessageTail::Markdown(text) => text,
    };
    let original = if message.header.is_empty() {
        tail.clone()
    } else {
        format!("{}\n{tail}", message.header)
    };
    Some(Prepared {
        kind,
        bot: pending.bot.clone(),
        recipient: pending.recipient.clone(),
        message,
        pane,
        original,
    })
}

fn discard(pending: &Pending, cx: &mut App) {
    if let Some(card) = &pending.card
        && let Some(connection) = cx
            .global_mut::<RemoteChannels>()
            .connections
            .get_mut(&pending.id)
        && connection.cards.borrow_mut().release(card)
    {
        connection.core.forget_card(&card.card);
    }
}

fn current(receipt: &Receipt, bot: &Arc<BotSession>, cx: &App) -> bool {
    cx.global::<RemoteChannels>()
        .live(&receipt.id, cx)
        .is_some_and(|connection| {
            connection.generation == receipt.generation
                && connection.can_send()
                && connection.config.recipient.as_ref() == Some(&receipt.recipient)
                && connection
                    .bot
                    .as_ref()
                    .is_some_and(|current| Arc::ptr_eq(current, bot))
        })
}

pub(super) fn edit(receipt: Receipt, label: String, cx: &mut App) {
    let Some(bot) = receipt.bot.upgrade() else {
        return;
    };
    if !current(&receipt, &bot, cx) {
        return;
    }
    #[cfg(test)]
    if let Some(result) = crate::remote_channel::cards::tests::capture(
        &receipt.id,
        receipt.ids.clone(),
        &receipt.original,
        &label,
        cx,
    ) {
        if let Err(error) = result {
            crate::remote_channel::log_error(
                "Remote settled card failed to edit",
                &error,
                "remote.card.edit",
            );
        }
        return;
    }
    cx.spawn(async move |cx| {
        for message_id in &receipt.ids {
            let Some(bot) = receipt.bot.upgrade() else {
                break;
            };
            let Some(kind) = cx.update(|cx| {
                current(&receipt, &bot, cx).then(|| {
                    cx.global::<RemoteChannels>().connections[&receipt.id]
                        .config
                        .kind
                })
            }) else {
                break;
            };
            let recipient = receipt.recipient.clone();
            let original = receipt.original.clone();
            let label = label.clone();
            let message_id = message_id.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    if !bot.is_live() {
                        return Ok(());
                    }
                    transport::edit(
                        kind,
                        &bot.credentials,
                        &recipient,
                        &message_id,
                        &original,
                        &label,
                    )
                })
                .await;
            if let Err(error) = result {
                crate::remote_channel::log_error(
                    "Remote settled card failed to edit",
                    &error,
                    "remote.card.edit",
                );
            }
        }
    })
    .detach();
}

impl RemoteChannels {
    pub(crate) fn settle_card(card: &CardId, label: &str, cx: &mut App) {
        if !cx.has_global::<Self>() {
            return;
        }
        let mut edits = Vec::new();
        for connection in cx.global_mut::<Self>().connections.values_mut() {
            connection.core.forget_card(card);
            if let Some(edit) = connection.cards.borrow_mut().settle(card, label.to_owned()) {
                edits.push(edit);
            }
        }
        for (receipt, label) in edits {
            edit(receipt, label, cx);
        }
    }

    pub(crate) fn settle_permissions(pane: PaneRef, session: u64, label: &str, cx: &mut App) {
        let cards: Vec<_> = cx
            .try_global::<Self>()
            .into_iter()
            .flat_map(|bridge| bridge.connections.values())
            .flat_map(|connection| connection.cards.borrow().permissions(pane, session))
            .collect();
        for card in cards {
            Self::settle_card(&card, label, cx);
        }
    }
}

pub(super) fn finish(
    pending: &Pending,
    pane: Option<PaneRef>,
    ids: Vec<String>,
    original: String,
    cx: &mut App,
) {
    let receipt = Receipt {
        id: pending.id.clone(),
        generation: pending.generation,
        bot: Arc::downgrade(&pending.bot),
        recipient: pending.recipient.clone(),
        ids,
        original,
    };
    if !current(&receipt, &pending.bot, cx) {
        discard(pending, cx);
        return;
    }
    let orchestrator = crate::orchestrator::pane(cx);
    let connection = cx
        .global_mut::<RemoteChannels>()
        .connections
        .get_mut(&pending.id)
        .unwrap();
    if let Some(pane) = pane {
        for id in &receipt.ids {
            connection
                .core
                .record_delivered(id.clone(), pane, orchestrator);
        }
    }
    if let Some(card) = &pending.card {
        let receipt = (!receipt.ids.is_empty()).then_some(receipt);
        if receipt.is_none() && connection.cards.borrow().owns(card) {
            connection.core.forget_card(&card.card);
        }
        let delivered = receipt.is_some() && connection.cards.borrow().is_sending(card);
        let edit_action = connection.cards.borrow_mut().complete(card, receipt);
        if delivered && let CardId::Approval(id) = &card.card {
            crate::control::approval::remote_card_delivered(*id, cx);
        }
        if let Some((receipt, label)) = edit_action {
            edit(receipt, label, cx);
        }
    }
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
                cx.update(|cx| discard(&error.into_inner(), cx));
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
            let original = prepared.original.clone();
            let (ids, result) = cx
                .background_executor()
                .spawn(async move {
                    let mut sent = Vec::new();
                    if !prepared.bot.is_live() {
                        return (sent, Ok(Vec::new()));
                    }
                    let result = transport::send_recording(
                        prepared.kind,
                        &prepared.bot.credentials,
                        &prepared.recipient,
                        prepared.message,
                        |id| sent.push(id.to_owned()),
                    );
                    (sent, result)
                })
                .await;
            cx.update(|cx| finish(&pending, pane, ids, original, cx));
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

#[cfg(test)]
mod tests;
