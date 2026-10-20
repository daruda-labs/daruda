use super::RemoteChannels;
use crate::remote_channel::{
    bridge::{InlineKeyboard, MessageTail, Outbound, PaneRef},
    cards::{CardCompletion, CardId, SendClaim},
    runtime::BotSession,
    transport::{self, Message},
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
    pub bot: Arc<BotSession>,
    pub recipient: RemoteRecipient,
    pub outbound: Outbound,
    pub card: Option<SendClaim>,
}

/// A sent card's messages, editable by whichever session serves the identity
/// that sent them.
#[derive(Clone)]
pub(super) struct Receipt {
    id: String,
    owner: String,
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
        && !connection.cards.get_mut().ledger_mut().begin(card)
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
        && connection.cards.get_mut().ledger_mut().release(card)
    {
        connection.core.forget_card(&card.card);
    }
}

/// Whether `pending` was sent by the session still serving its connection.
fn current(pending: &Pending, cx: &App) -> bool {
    cx.global::<RemoteChannels>()
        .live(&pending.id, cx)
        .is_some_and(|connection| {
            connection.generation == pending.generation
                && connection.can_send()
                && connection.config.recipient.as_ref() == Some(&pending.recipient)
                && connection
                    .bot
                    .as_ref()
                    .is_some_and(|bot| Arc::ptr_eq(bot, &pending.bot))
        })
}

/// The session that may edit `receipt`'s messages: any live one serving the
/// identity and recipient that sent them.
fn editor(receipt: &Receipt, cx: &App) -> Option<(ChannelKind, Arc<BotSession>)> {
    let connection = cx.global::<RemoteChannels>().live(&receipt.id, cx)?;
    let serves = connection.can_send()
        && connection.config.recipient.as_ref() == Some(&receipt.recipient)
        && connection.cards.borrow().owner() == Some(&receipt.owner);
    if !serves {
        return None;
    }
    Some((connection.config.kind, connection.bot.clone()?))
}

pub(super) fn edit(receipt: Receipt, label: String, cx: &mut App) {
    let Some(connection) = cx
        .global_mut::<RemoteChannels>()
        .connections
        .get_mut(&receipt.id)
    else {
        return;
    };
    let cards = connection.cards.get_mut();
    if cards.owner() != Some(&receipt.owner) {
        return;
    }
    for id in &receipt.ids {
        cards.note_settled(id.clone());
    }
    let id = receipt.id.clone();
    cards.edits.push(receipt, label);
    flush_connection(&id, cx);
}

pub(super) fn flush_edits(cx: &mut App) {
    let ids: Vec<_> = cx
        .global::<RemoteChannels>()
        .connections
        .keys()
        .cloned()
        .collect();
    for id in ids {
        flush_connection(&id, cx);
    }
}

fn flush_connection(id: &str, cx: &mut App) {
    let Some(connection) = cx.global_mut::<RemoteChannels>().connections.get_mut(id) else {
        return;
    };
    if !connection.can_send() {
        return;
    }
    let pending = connection.cards.get_mut().edits.claim();
    for (attempt, receipt, label) in pending {
        #[cfg(test)]
        if let Some(result) = crate::remote_channel::cards::tests::capture(
            &receipt.id,
            receipt.ids.clone(),
            &receipt.original,
            &label,
            cx,
        ) {
            let edited = result.is_ok();
            if let Err(error) = result {
                crate::remote_channel::log_error(
                    "Remote settled card failed to edit",
                    &error,
                    "remote.card.edit",
                );
            }
            finish_edit(&receipt, attempt, edited, cx);
            continue;
        }
        cx.spawn(async move |cx| {
            let mut edited = true;
            for message_id in &receipt.ids {
                let serving = cx.update(|cx| {
                    let connection = cx.global::<RemoteChannels>().connections.get(&receipt.id)?;
                    if !connection.cards.borrow().edits.contains(attempt) {
                        return None;
                    }
                    editor(&receipt, cx)
                });
                let Some((kind, bot)) = serving else {
                    edited = false;
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
                            return Ok(false);
                        }
                        transport::edit(
                            kind,
                            &bot.credentials,
                            &recipient,
                            &message_id,
                            &original,
                            &label,
                        )
                        .map(|()| true)
                    })
                    .await;
                match result {
                    Ok(true) => {}
                    Ok(false) => {
                        edited = false;
                        break;
                    }
                    Err(error) => {
                        crate::remote_channel::log_error(
                            "Remote settled card failed to edit",
                            &error,
                            "remote.card.edit",
                        );
                        edited = false;
                        break;
                    }
                }
            }
            cx.update(|cx| finish_edit(&receipt, attempt, edited, cx));
        })
        .detach();
    }
}

fn finish_edit(receipt: &Receipt, attempt: uuid::Uuid, edited: bool, cx: &mut App) {
    if let Some(connection) = cx
        .global_mut::<RemoteChannels>()
        .connections
        .get_mut(&receipt.id)
    {
        connection.cards.get_mut().edits.finish(attempt, edited);
    }
}

impl RemoteChannels {
    pub(crate) fn settle_card(card: &CardId, label: &str, cx: &mut App) {
        if !cx.has_global::<Self>() {
            return;
        }
        let mut edits = Vec::new();
        for connection in cx.global_mut::<Self>().connections.values_mut() {
            connection.core.settle_card(card);
            if let Some(edit) = connection
                .cards
                .get_mut()
                .ledger_mut()
                .settle(card, label.to_owned())
            {
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
            .flat_map(|connection| {
                connection
                    .cards
                    .borrow()
                    .ledger()
                    .permissions(pane, session)
            })
            .collect();
        for card in cards {
            Self::settle_card(&card, label, cx);
        }
    }
}

/// Record what a send came to. A card that reached the person is kept as sent
/// even if its session was replaced meanwhile: the ledger's claim decides
/// whether it still belongs to this identity, and a same-bot replacement can
/// still edit it, so the pump must not offer it again.
pub(super) fn finish(
    pending: &Pending,
    pane: Option<PaneRef>,
    ids: Vec<String>,
    original: String,
    cx: &mut App,
) {
    let live = current(pending, cx);
    if ids.is_empty() && !live {
        // Nothing went out: the session was gone before the send.
        discard(pending, cx);
        return;
    }
    let orchestrator = crate::orchestrator::pane(cx);
    let Some(connection) = cx
        .global_mut::<RemoteChannels>()
        .connections
        .get_mut(&pending.id)
    else {
        return;
    };
    if live && let Some(pane) = pane {
        for id in &ids {
            connection
                .core
                .record_delivered(id.clone(), pane, orchestrator);
        }
    }
    if let Some(card) = &pending.card {
        let cards = connection.cards.get_mut();
        let receipt = cards
            .owner()
            .filter(|_| !ids.is_empty())
            .map(|owner| Receipt {
                id: pending.id.clone(),
                owner: owner.clone(),
                recipient: pending.recipient.clone(),
                ids,
                original,
            });
        let completion = cards.ledger_mut().complete(card, receipt);
        if matches!(completion, CardCompletion::Unconfirmed) {
            connection.core.forget_card(&card.card);
        }
        crate::remote_channel::apply_card_completion(&card.card, completion, edit, cx);
    }
}

/// Each connection drains in order; a throttled channel does not block another.
pub(super) fn spawn(mut receiver: UnboundedReceiver<Pending>, cx: &mut App) {
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor()
                .timer(crate::remote_channel::cards::RETRY_INTERVAL)
                .await;
            cx.update(flush_edits);
        }
    })
    .detach();
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
