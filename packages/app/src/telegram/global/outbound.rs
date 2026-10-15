//! Queue ownership and the off-foreground Telegram send loop.

use std::sync::Arc;

use futures::StreamExt as _;
use gpui::App;

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;

use crate::settings_store::SettingsStore;
use crate::telegram::bridge::{Outbound, OutboundMsg, PaneRef};
use crate::telegram::{client, keychain, trace};

use super::bot::ReadyBot;
use super::{TelegramBridge, html_body, plain_body};

pub(super) struct QueuedOutbound {
    pub bot: Arc<ReadyBot>,
    pub chat_id: i64,
    pub outbound: Outbound,
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
}

fn live(queued: &QueuedOutbound, cx: &App) -> bool {
    super::inbound::live_bot(&queued.bot, Some(queued.chat_id), keychain::revision(), cx)
}

fn prepare(queued: QueuedOutbound, cx: &mut App) -> Option<Prepared> {
    if !live(&queued, cx) {
        return None;
    }
    let cfg = SettingsStore::global(cx).user_arc();
    let bridge = cx.global_mut::<TelegramBridge>();
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
    Some(Prepared {
        bot: queued.bot,
        chat_id: queued.chat_id,
        pane,
        body,
    })
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
            match cx
                .background_executor()
                .spawn(async move { send(prepared) })
                .await
            {
                Ok(Some(message_id)) => {
                    if let Some(pane) = pane {
                        cx.update(|cx| {
                            if super::inbound::live_bot(
                                &bot,
                                Some(chat_id),
                                keychain::revision(),
                                cx,
                            ) {
                                trace::state("sent_pings", || {
                                    format!(
                                        "message_id={message_id} pane={} last_pinged=true",
                                        trace::pane(pane)
                                    )
                                });
                                let orchestrator = crate::orchestrator::pane(cx);
                                cx.global_mut::<TelegramBridge>().core.record_delivered(
                                    message_id,
                                    pane,
                                    orchestrator,
                                );
                            }
                        });
                    }
                }
                Ok(None) => {}
                Err(error) => LogWriter::log(
                    ErrorReport::new("Telegram message failed to send")
                        .severity(ErrorSeverity::Warning)
                        .from_error(&error)
                        .at(file!(), line!())
                        .dedup("telegram.send_message")
                        .build(),
                ),
            }
        }
    })
    .detach();
}

#[cfg(test)]
mod tests;
