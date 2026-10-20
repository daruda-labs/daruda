//! Queue ownership and the off-foreground Telegram send loop.

use std::sync::Arc;

use futures::StreamExt as _;
use gpui::App;

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;

use crate::remote_channel::cards::{CardCompletion, CardId, SendClaim};
use crate::settings_store::SettingsStore;
use crate::telegram::bridge::{Outbound, OutboundMsg, PaneRef};
use crate::telegram::{client, keychain, trace};

use super::bot::ReadyBot;
use super::{TelegramBridge, html_body, plain_body};

pub(super) struct QueuedOutbound {
    pub bot: Arc<ReadyBot>,
    pub chat_id: i64,
    pub outbound: Outbound,
    pub card: Option<SendClaim>,
}

/// A sent card's message, editable by whichever bot serves its identity.
#[derive(Clone)]
pub(super) struct Receipt {
    owner: CardOwner,
    message_id: i64,
    original: String,
}

/// The identity whose messages Telegram cards are: the bot, by its token, and
/// the paired chat. Not the `ReadyBot` instance — a token revision or inbox
/// reopen replaces that while the same bot keeps serving the same chat.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CardOwner {
    bot: String,
    chat_id: i64,
}

impl CardOwner {
    pub(super) fn of(bot: &ReadyBot, chat_id: i64) -> Self {
        Self {
            bot: crate::remote_channel::lock::digest_of(&bot.token.token),
            chat_id,
        }
    }
}

pub(super) type OwnedCards = crate::remote_channel::cards::OwnedCards<CardOwner, Receipt, i64>;

#[cfg(test)]
pub(crate) struct TestOutbox {
    pub(super) receiver: futures::channel::mpsc::UnboundedReceiver<QueuedOutbound>,
}

#[cfg(test)]
impl futures::Stream for TestOutbox {
    type Item = Outbound;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Outbound>> {
        self.get_mut()
            .receiver
            .poll_next_unpin(cx)
            .map(|queued| queued.map(|queued| queued.outbound))
    }
}

#[cfg(test)]
impl TestOutbox {
    pub(crate) fn try_recv(&mut self) -> Result<Outbound, futures::channel::mpsc::TryRecvError> {
        self.receiver.try_recv().map(|queued| queued.outbound)
    }

    pub(crate) fn begin_send(
        &mut self,
        message_id: i64,
        cx: &mut App,
    ) -> Option<impl FnOnce(&mut App) + use<>> {
        let queued = self.receiver.try_recv().ok()?;
        let prepared = prepare(queued, cx)?;
        Some(move |cx: &mut App| {
            finish(
                &prepared.bot,
                prepared.chat_id,
                prepared.pane,
                prepared.card,
                Some(message_id),
                prepared.original,
                cx,
            )
        })
    }
}

enum Body {
    Ping(OutboundMsg),
    Plain {
        text: String,
        keyboard: Option<client::InlineKeyboard>,
    },
}

struct Prepared {
    bot: Arc<ReadyBot>,
    chat_id: i64,
    pane: Option<PaneRef>,
    body: Body,
    card: Option<SendClaim>,
    original: String,
}

fn live(queued: &QueuedOutbound, cx: &App) -> bool {
    super::inbound::live_bot(&queued.bot, Some(queued.chat_id), keychain::revision(), cx)
}

fn prepare(queued: QueuedOutbound, cx: &mut App) -> Option<Prepared> {
    if !live(&queued, cx) {
        discard(&queued, cx);
        return None;
    }
    let cfg = SettingsStore::global(cx).user_arc();
    let bridge = cx.global_mut::<TelegramBridge>();
    if let Some(card) = &queued.card
        && !bridge.cards.get_mut().ledger_mut().begin(card)
    {
        return None;
    }
    bridge.core.set_enabled(cfg.telegram.enabled);
    bridge
        .core
        .set_authorized_chat_id(cfg.telegram.authorized_chat_id);
    let (pane, body) = match queued.outbound {
        Outbound::Ping(ping) => {
            let pane = ping.pane;
            let message = bridge.core.build_ping(ping);
            let buttons: usize = message
                .keyboard
                .as_ref()
                .map_or(0, |keyboard| keyboard.rows.iter().map(Vec::len).sum());
            if buttons > 0 {
                trace::state("permission.tokens", || {
                    format!("pane={} registered={buttons}", trace::pane(pane))
                });
            }
            trace::delivery("send.ping", || {
                format!(
                    "chat_id={} pane={} buttons={buttons} text={}",
                    queued.chat_id,
                    trace::pane(pane),
                    trace::tail_digest(&message.tail)
                )
            });
            (Some(pane), Body::Ping(message))
        }
        Outbound::Notice(text) => (
            None,
            Body::Plain {
                text,
                keyboard: None,
            },
        ),
        Outbound::FlowAsk(prompt) => {
            let message = bridge.core.routing.build_flow_ask(prompt);
            (
                None,
                Body::Plain {
                    text: match message.tail {
                        crate::remote_channel::bridge::MessageTail::Plain(text)
                        | crate::remote_channel::bridge::MessageTail::Markdown(text) => text,
                    },
                    keyboard: message.keyboard,
                },
            )
        }
        Outbound::Approval(prompt) => (
            None,
            Body::Plain {
                text: prompt.summary,
                keyboard: Some(client::InlineKeyboard {
                    rows: vec![prompt.buttons.to_vec()],
                }),
            },
        ),
    };
    let original = match &body {
        Body::Ping(message) => plain_body(&message.header, &message.tail),
        Body::Plain { text, .. } => text.clone(),
    };
    Some(Prepared {
        bot: queued.bot,
        chat_id: queued.chat_id,
        pane,
        body,
        card: queued.card,
        original,
    })
}

fn discard(queued: &QueuedOutbound, cx: &mut App) {
    if let Some(card) = &queued.card {
        release(card, cx);
    }
}

/// Give up a claim nothing was sent for, with the tokens minted for it.
fn release(card: &SendClaim, cx: &mut App) {
    let bridge = cx.global_mut::<TelegramBridge>();
    if bridge.cards.get_mut().ledger_mut().release(card) {
        bridge.core.routing.forget_card(&card.card);
    }
}

/// The bot that may edit `receipt`'s message: the one serving its identity now.
fn editor(receipt: &Receipt, cx: &App) -> Option<Arc<ReadyBot>> {
    let bridge = cx.global::<TelegramBridge>();
    let bot = bridge.bot.ready().filter(|_| bridge.bot.serves())?;
    let config = &SettingsStore::global(cx).user().telegram;
    let serves = config.enabled
        && config.authorized_chat_id == Some(receipt.owner.chat_id)
        && CardOwner::of(bot, receipt.owner.chat_id) == receipt.owner
        && bridge.cards.borrow().owner() == Some(&receipt.owner);
    serves.then(|| bot.clone())
}

pub(super) fn edit(receipt: Receipt, label: String, cx: &mut App) {
    let cards = cx.global_mut::<TelegramBridge>().cards.get_mut();
    if cards.owner() != Some(&receipt.owner) {
        return;
    }
    cards.note_settled(receipt.message_id);
    cards.edits.push(receipt, label);
    flush_edits(cx);
}

pub(super) fn flush_edits(cx: &mut App) {
    let bridge = cx.global_mut::<TelegramBridge>();
    if !bridge.bot.serves() {
        return;
    }
    let pending = bridge.cards.get_mut().edits.claim();
    for (attempt, receipt, label) in pending {
        if editor(&receipt, cx).is_none() {
            finish_edit(attempt, false, cx);
            continue;
        }
        #[cfg(test)]
        if let Some(result) = crate::remote_channel::cards::tests::capture(
            "telegram",
            vec![receipt.message_id.to_string()],
            &receipt.original,
            &label,
            cx,
        ) {
            let edited = result.is_ok();
            if let Err(error) = result {
                crate::remote_channel::log_error(
                    "Telegram settled card failed to edit",
                    &error,
                    "telegram.card.edit",
                );
            }
            finish_edit(attempt, edited, cx);
            continue;
        }
        cx.spawn(async move |cx| {
            let bot = cx.update(|cx| {
                if !cx
                    .global::<TelegramBridge>()
                    .cards
                    .borrow()
                    .edits
                    .contains(attempt)
                {
                    return None;
                }
                editor(&receipt, cx)
            });
            let Some(bot) = bot else {
                cx.update(|cx| finish_edit(attempt, false, cx));
                return;
            };
            let body = crate::remote_channel::dispatch::target::compose_edit_body(
                &receipt.original,
                &label,
            );
            let result = cx
                .background_executor()
                .spawn(async move {
                    if bot.token.revision != keychain::revision() {
                        return Ok(false);
                    }
                    client::edit_message_text(
                        &bot.token.token,
                        receipt.owner.chat_id,
                        receipt.message_id,
                        &body,
                    )
                    .map(|()| true)
                })
                .await;
            let edited = match result {
                Ok(edited) => edited,
                Err(error) => {
                    crate::remote_channel::log_error(
                        "Telegram settled card failed to edit",
                        &error,
                        "telegram.card.edit",
                    );
                    false
                }
            };
            cx.update(|cx| finish_edit(attempt, edited, cx));
        })
        .detach();
    }
}

fn finish_edit(attempt: uuid::Uuid, edited: bool, cx: &mut App) {
    cx.global_mut::<TelegramBridge>()
        .cards
        .get_mut()
        .edits
        .finish(attempt, edited);
}

impl TelegramBridge {
    /// Bind the card ledger to the identity now serving. The same bot and chat
    /// keep every record; another identity can neither edit the old messages
    /// nor receive their taps, so their callback tokens are forgotten with
    /// them. Switching Telegram off or unpairing ends the identity the same
    /// way. A bot that is only between claims keeps its records for its
    /// return.
    pub(super) fn sync_card_owner(cx: &mut App) {
        let Some(bridge) = cx.try_global::<Self>() else {
            return;
        };
        let config = &SettingsStore::global(cx).user().telegram;
        let paired = config.authorized_chat_id.filter(|_| config.enabled);
        let owner = paired
            .zip(bridge.bot.ready())
            .map(|(chat_id, bot)| CardOwner::of(bot, chat_id));
        let bridge = cx.global_mut::<Self>();
        if owner
            .as_ref()
            .is_some_and(|owner| bridge.cards.get_mut().owner() != Some(owner))
            || paired.is_none()
        {
            bridge.core.routing.clear_settled_callbacks();
        }
        let forgotten = match (paired, owner) {
            (None, _) => bridge.cards.get_mut().retire(),
            (Some(_), Some(owner)) => bridge.cards.get_mut().bind(owner),
            (Some(_), None) => return,
        };
        for card in forgotten {
            bridge.core.routing.forget_card(&card);
        }
    }

    pub(crate) fn settle_card(card: &CardId, label: &str, cx: &mut App) {
        if !cx.has_global::<Self>() {
            return;
        }
        let bridge = cx.global_mut::<Self>();
        bridge.core.routing.settle_card(card);
        let edit_action = bridge
            .cards
            .get_mut()
            .ledger_mut()
            .settle(card, label.to_owned());
        if let Some((receipt, label)) = edit_action {
            edit(receipt, label, cx);
        }
    }

    pub(crate) fn settle_permissions(pane: PaneRef, session: u64, label: &str, cx: &mut App) {
        let cards = cx
            .try_global::<Self>()
            .map(|bridge| bridge.cards.borrow().ledger().permissions(pane, session))
            .unwrap_or_default();
        for card in cards {
            Self::settle_card(&card, label, cx);
        }
    }
}

/// Record what a send came to. A card that reached the chat is kept as sent
/// even if its `ReadyBot` was replaced meanwhile: the ledger's claim decides
/// whether it still belongs to this identity, and a same-bot replacement can
/// still edit it, so it must not be offered again.
fn finish(
    bot: &Arc<ReadyBot>,
    chat_id: i64,
    pane: Option<PaneRef>,
    card: Option<SendClaim>,
    message_id: Option<i64>,
    original: String,
    cx: &mut App,
) {
    let live = super::inbound::live_bot(bot, Some(chat_id), keychain::revision(), cx);
    if !live && message_id.is_none() {
        // Nothing went out: the bot was retired before the send.
        if let Some(card) = &card {
            release(card, cx);
        }
        return;
    }
    let orchestrator = crate::orchestrator::pane(cx);
    let bridge = cx.global_mut::<TelegramBridge>();
    if live && let Some((pane, id)) = pane.zip(message_id) {
        bridge.core.record_delivered(id, pane, orchestrator);
    }
    if let Some(card) = card {
        let receipt = message_id.map(|message_id| Receipt {
            owner: CardOwner::of(bot, chat_id),
            message_id,
            original,
        });
        let completion = bridge.cards.get_mut().ledger_mut().complete(&card, receipt);
        if matches!(completion, CardCompletion::Unconfirmed) {
            bridge.core.routing.forget_card(&card.card);
        }
        crate::remote_channel::apply_card_completion(&card.card, completion, edit, cx);
    }
}

fn send(prepared: Prepared) -> Result<Option<i64>, client::ClientError> {
    if prepared.bot.token.revision != keychain::revision() {
        return Ok(None);
    }
    let token = &prepared.bot.token.token;
    let sent = match prepared.body {
        Body::Plain { text, keyboard } => {
            let event = if keyboard.is_some() {
                "send.approval"
            } else {
                "send.notice"
            };
            let sent = client::send_message(token, prepared.chat_id, &text, None, keyboard);
            trace::delivery(event, || {
                format!(
                    "chat_id={} ok={} text={}",
                    prepared.chat_id,
                    sent.is_ok(),
                    trace::digest(&text)
                )
            });
            sent
        }
        Body::Ping(OutboundMsg {
            chat_id,
            header,
            tail,
            keyboard,
        }) => {
            let html = html_body(&header, &tail);
            match client::send_message(token, chat_id, &html, Some("HTML"), keyboard.clone()) {
                Ok(id) => Ok(id),
                Err(error) => {
                    trace::delivery("send.html_rejected", || {
                        format!("chat_id={chat_id} error={error}")
                    });
                    client::send_message(
                        token,
                        chat_id,
                        &plain_body(&header, &tail),
                        None,
                        keyboard,
                    )
                }
            }
        }
    };
    sent.map(Some)
}

pub(super) fn spawn_send_task(
    mut outbound_rx: futures::channel::mpsc::UnboundedReceiver<QueuedOutbound>,
    cx: &mut App,
) {
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
        while let Some(queued) = outbound_rx.next().await {
            trace::delivery("queue.drained", || {
                format!(
                    "{} kind={}",
                    trace::drain_slot(),
                    trace::outbound_kind(&queued.outbound)
                )
            });
            let Some(prepared) = cx.update(|cx| prepare(queued, cx)) else {
                trace::delivery("send.dropped", || {
                    "reason=owner_or_recipient_changed".into()
                });
                continue;
            };
            let bot = prepared.bot.clone();
            let chat_id = prepared.chat_id;
            let pane = prepared.pane;
            let card = prepared.card.clone();
            let original = prepared.original.clone();
            let result = cx
                .background_executor()
                .spawn(async move { send(prepared) })
                .await;
            cx.update(|cx| {
                finish(
                    &bot,
                    chat_id,
                    pane,
                    card,
                    result.as_ref().ok().copied().flatten(),
                    original,
                    cx,
                )
            });
            if let Err(error) = result {
                LogWriter::log(
                    ErrorReport::new("Telegram message failed to send")
                        .severity(ErrorSeverity::Warning)
                        .from_error(&error)
                        .at(file!(), line!())
                        .dedup("telegram.send_message")
                        .build(),
                );
            }
        }
    })
    .detach();
}

#[cfg(test)]
mod tests;
