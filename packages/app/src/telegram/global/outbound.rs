//! Queue ownership and the off-foreground Telegram send loop.

use std::sync::{Arc, Weak};

use futures::StreamExt as _;
use gpui::App;

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;

use crate::remote_channel::cards::{CardId, CardLedger, SendClaim};
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

pub(super) struct Receipt {
    bot: Weak<ReadyBot>,
    chat_id: i64,
    message_id: i64,
    original: String,
}

pub(super) struct OwnedCards {
    generation: uuid::Uuid,
    chat_id: i64,
    pub(super) ledger: CardLedger<Receipt>,
}

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
        && !bridge
            .cards
            .borrow_mut()
            .as_mut()
            .is_some_and(|cards| cards.ledger.begin(card))
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
        let bridge = cx.global_mut::<TelegramBridge>();
        if bridge
            .cards
            .borrow_mut()
            .as_mut()
            .is_some_and(|cards| cards.ledger.release(card))
        {
            bridge.core.routing.forget_card(&card.card);
        }
    }
}

pub(super) fn edit(receipt: Receipt, label: String, cx: &mut App) {
    let Some(bot) = receipt.bot.upgrade() else {
        return;
    };
    if !super::inbound::live_bot(&bot, Some(receipt.chat_id), keychain::revision(), cx) {
        return;
    }
    #[cfg(test)]
    if let Some(result) = crate::remote_channel::cards::tests::capture(
        "telegram",
        vec![receipt.message_id.to_string()],
        &receipt.original,
        &label,
        cx,
    ) {
        if let Err(error) = result {
            crate::remote_channel::log_error(
                "Telegram settled card failed to edit",
                &error,
                "telegram.card.edit",
            );
        }
        return;
    }
    cx.spawn(async move |cx| {
        let Some(bot) = receipt.bot.upgrade() else {
            return;
        };
        if !cx.update(|cx| {
            super::inbound::live_bot(&bot, Some(receipt.chat_id), keychain::revision(), cx)
        }) {
            return;
        }
        let body =
            crate::remote_channel::dispatch::target::compose_edit_body(&receipt.original, &label);
        let result = cx
            .background_executor()
            .spawn(async move {
                if bot.token.revision != keychain::revision() {
                    return Ok(());
                }
                client::edit_message_text(
                    &bot.token.token,
                    receipt.chat_id,
                    receipt.message_id,
                    &body,
                )
            })
            .await;
        if let Err(error) = result {
            crate::remote_channel::log_error(
                "Telegram settled card failed to edit",
                &error,
                "telegram.card.edit",
            );
        }
    })
    .detach();
}

impl TelegramBridge {
    pub(super) fn cards_for(
        &self,
        bot: &ReadyBot,
        chat_id: i64,
    ) -> std::cell::RefMut<'_, CardLedger<Receipt>> {
        let mut slot = self.cards.borrow_mut();
        if slot
            .as_ref()
            .is_none_or(|cards| cards.generation != bot.generation || cards.chat_id != chat_id)
        {
            *slot = Some(OwnedCards {
                generation: bot.generation,
                chat_id,
                ledger: CardLedger::default(),
            });
        }
        std::cell::RefMut::map(slot, |slot| &mut slot.as_mut().unwrap().ledger)
    }

    pub(crate) fn sync_card_owner(cx: &App) {
        let Some(bridge) = cx.try_global::<Self>() else {
            return;
        };
        let config = &SettingsStore::global(cx).user().telegram;
        if config.enabled
            && let Some((bot, chat_id)) = bridge.bot.ready().zip(config.authorized_chat_id)
        {
            drop(bridge.cards_for(bot, chat_id));
        } else {
            *bridge.cards.borrow_mut() = None;
        }
    }

    pub(crate) fn settle_card(card: &CardId, label: &str, cx: &mut App) {
        if !cx.has_global::<Self>() {
            return;
        }
        let bridge = cx.global_mut::<Self>();
        bridge.core.routing.forget_card(card);
        let edit_action = bridge
            .cards
            .borrow_mut()
            .as_mut()
            .and_then(|cards| cards.ledger.settle(card, label.to_owned()));
        if let Some((receipt, label)) = edit_action {
            edit(receipt, label, cx);
        }
    }

    pub(crate) fn settle_permissions(pane: PaneRef, session: u64, label: &str, cx: &mut App) {
        let cards = cx
            .try_global::<Self>()
            .map(|bridge| {
                bridge
                    .cards
                    .borrow()
                    .as_ref()
                    .map(|cards| cards.ledger.permissions(pane, session))
                    .unwrap_or_default()
            })
            .unwrap_or_default();
        for card in cards {
            Self::settle_card(&card, label, cx);
        }
    }
}

fn finish(
    bot: &Arc<ReadyBot>,
    chat_id: i64,
    pane: Option<PaneRef>,
    card: Option<SendClaim>,
    message_id: Option<i64>,
    original: String,
    cx: &mut App,
) {
    if !super::inbound::live_bot(bot, Some(chat_id), keychain::revision(), cx) {
        if let Some(card) = card
            && let Some(cards) = cx.global::<TelegramBridge>().cards.borrow_mut().as_mut()
        {
            cards.ledger.release(&card);
        }
        return;
    }
    let orchestrator = crate::orchestrator::pane(cx);
    let bridge = cx.global_mut::<TelegramBridge>();
    if let Some((pane, id)) = pane.zip(message_id) {
        bridge.core.record_delivered(id, pane, orchestrator);
    }
    if let Some(card) = card {
        let receipt = message_id.map(|message_id| Receipt {
            bot: Arc::downgrade(bot),
            chat_id,
            message_id,
            original,
        });
        if receipt.is_none()
            && bridge
                .cards
                .borrow()
                .as_ref()
                .is_some_and(|cards| cards.ledger.owns(&card))
        {
            bridge.core.routing.forget_card(&card.card);
        }
        let delivered = receipt.is_some()
            && bridge
                .cards
                .borrow()
                .as_ref()
                .is_some_and(|cards| cards.ledger.is_sending(&card));
        let edit_action = bridge
            .cards
            .borrow_mut()
            .as_mut()
            .and_then(|cards| cards.ledger.complete(&card, receipt));
        if delivered && let CardId::Approval(id) = &card.card {
            crate::control::approval::remote_card_delivered(*id, cx);
        }
        if let Some((receipt, label)) = edit_action {
            edit(receipt, label, cx);
        }
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
