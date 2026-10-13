//! The outbound send loop: drains queued pings and posts each one to the
//! Bot API off the foreground thread.

use futures::StreamExt as _;
use gpui::App;

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;

use crate::settings_store::SettingsStore;
use crate::telegram::bridge::{Outbound, OutboundMsg};
use crate::telegram::client;
use crate::telegram::keychain;
use crate::telegram::trace;

use super::{TelegramBridge, html_body, plain_body};

/// The outbound send loop. Drains `outbound_rx` (moved in at spawn
/// time — never stored on the struct) and posts each ping via
/// `client::send_message`, off the foreground thread.
pub(super) fn spawn_send_task(
    mut outbound_rx: futures::channel::mpsc::UnboundedReceiver<Outbound>,
    cx: &mut App,
) {
    cx.spawn(async move |cx| {
        while let Some(outbound) = outbound_rx.next().await {
            trace::delivery("queue.drained", || {
                format!(
                    "{} kind={}",
                    trace::drain_slot(),
                    trace::outbound_kind(&outbound)
                )
            });
            // Check for a token BEFORE touching `BridgeCore` state —
            // `build_ping` registers permission tokens as a side effect;
            // if the message that would let the phone redeem them is
            // never actually sent (token cleared mid-flight, e.g. the
            // user clicks "Clear" in Settings between a permission
            // request firing and this loop draining it), those tokens
            // would otherwise be permanently orphaned in
            // `pending_permissions`.
            let token = cx.update(|_cx| keychain::read_token());
            let Some(token) = token else {
                trace::delivery("send.dropped", || "reason=no_token".to_string());
                // The caller only builds a ping when the feature is on and a
                // chat is paired, so reaching here means the config says
                // "paired" while the Keychain has no token — the relay is dead
                // and nothing else on this path would say so. Every other
                // failure in this file is logged; without this one the symptom
                // is a bridge that looks configured and silently sends nothing.
                LogWriter::log(
                    ErrorReport::new("Telegram ping dropped: no bot token")
                        .severity(ErrorSeverity::Warning)
                        .message(
                            "Telegram is enabled and paired, but no bot token is stored in the \
                             Keychain. Re-pair from Settings to restore the relay.",
                        )
                        .at(file!(), line!())
                        .dedup("telegram.missing_token")
                        .build(),
                );
                continue;
            };

            // Resync from live config *before* addressing the message, not
            // just at the top of the poll loop. That resync only runs once the
            // previous blocking `get_updates` returns, so `BridgeCore`'s copy
            // can be up to `POLL_TIMEOUT_SECS` stale — long enough for an
            // agent response queued before an unpair to be addressed to the
            // chat the user just revoked. Config is the single source of
            // truth for where a ping may go; this is where it is asked.
            let (enabled, chat_id, serving) = cx.update(|cx| {
                let cfg = SettingsStore::global(cx).user_arc();
                let bridge = cx.global_mut::<TelegramBridge>();
                bridge.core.set_enabled(cfg.telegram.enabled);
                bridge
                    .core
                    .set_authorized_chat_id(cfg.telegram.authorized_chat_id);
                (
                    cfg.telegram.enabled,
                    cfg.telegram.authorized_chat_id,
                    bridge.claim.serves(),
                )
            });
            // The last gate, for the same reason config is re-read here: a
            // message queued while this daruda still held the bot must not go
            // out after another one took it over.
            if !serving {
                trace::delivery("send.dropped", || {
                    "reason=bot_held_elsewhere stage=drain".to_string()
                });
                continue;
            }
            // A token is in hand by here, so `has_token` is not in question —
            // shares the poll loop's dedup slot, which reports the real value.
            trace::gate_change(enabled, chat_id, true);
            let Some(live_chat_id) = enabled.then_some(chat_id).flatten() else {
                // Disabled or unpaired since this was queued. Dropped rather
                // than held: its context is gone, and the chat it was meant
                // for may no longer be the user's.
                trace::delivery("send.dropped", || {
                    format!(
                        "reason=gate enabled={enabled} chat_id={}",
                        trace::opt(chat_id)
                    )
                });
                continue;
            };

            // A notice belongs to no pane, so it skips `build_ping` (which
            // would register permission tokens it has none of) and skips
            // `record_sent` below (which would make some pane the next plain
            // message's destination).
            let ping = match outbound {
                Outbound::Ping(ping) => ping,
                // Like a notice, this belongs to no pane — so it skips
                // `build_ping` and `record_sent`. Unlike one, it carries
                // buttons, and a card the user cannot answer is worse than no
                // card: the tool call would wait out the whole timeout.
                Outbound::Approval(prompt) => {
                    let summary = trace::is_on().then(|| trace::digest(&prompt.summary));
                    let approval_token = token;
                    let keyboard = client::InlineKeyboard {
                        rows: vec![prompt.buttons.to_vec()],
                    };
                    let sent = cx
                        .background_executor()
                        .spawn(async move {
                            client::send_message(
                                &approval_token,
                                live_chat_id,
                                &prompt.summary,
                                None,
                                Some(keyboard),
                            )
                        })
                        .await;
                    trace::delivery("send.approval", || {
                        format!(
                            "chat_id={live_chat_id} ok={} text={}",
                            sent.is_ok(),
                            trace::opt(summary)
                        )
                    });
                    if let Err(e) = sent {
                        LogWriter::log(
                            ErrorReport::new("Telegram approval card failed to send")
                                .severity(ErrorSeverity::Warning)
                                .from_error(&e)
                                .at(file!(), line!())
                                .dedup("telegram.approval")
                                .build(),
                        );
                    }
                    continue;
                }
                Outbound::Notice(text) => {
                    let notice = trace::is_on().then(|| trace::digest(&text));
                    let notice_token = token;
                    let sent = cx
                        .background_executor()
                        .spawn(async move {
                            client::send_message(&notice_token, live_chat_id, &text, None, None)
                        })
                        .await;
                    trace::delivery("send.notice", || {
                        format!(
                            "chat_id={live_chat_id} ok={} text={}",
                            sent.is_ok(),
                            trace::opt(notice)
                        )
                    });
                    if let Err(e) = sent {
                        // Warning, not Info: a command reply is missed the
                        // instant it fails because the user is looking at the
                        // chat, but a flow outcome dropped an hour later is
                        // invisible outside this log — and something was
                        // promised it would arrive.
                        LogWriter::log(
                            ErrorReport::new("Telegram notice failed to send")
                                .severity(ErrorSeverity::Warning)
                                .from_error(&e)
                                .at(file!(), line!())
                                .dedup("telegram.notice")
                                .build(),
                        );
                    }
                    continue;
                }
            };

            // Captured before `build_ping` consumes `ping` by value —
            // `OutboundMsg` does not carry the pane back.
            let pane = ping.pane;

            let msg = cx.update(|cx| cx.global_mut::<TelegramBridge>().core.build_ping(ping));

            let OutboundMsg {
                chat_id,
                header,
                tail,
                keyboard,
            } = msg;
            // One token per button, so the keyboard's width *is* what
            // `build_ping` just registered in `pending_permissions`.
            let buttons: usize = keyboard
                .as_ref()
                .map_or(0, |k| k.rows.iter().map(Vec::len).sum());
            if buttons > 0 {
                trace::state("permission.tokens", || {
                    format!("pane={} registered={buttons}", trace::pane(pane))
                });
            }
            trace::delivery("send.ping", || {
                format!(
                    "chat_id={chat_id} pane={} buttons={buttons} text={}",
                    trace::pane(pane),
                    trace::tail_digest(&tail)
                )
            });
            // Computed up front (not inside the spawn below), since both
            // `header` and `tail` get moved into the closure next.
            let plain_text = plain_body(&header, &tail);
            // Try the HTML-formatted body first; on ANY failure (a
            // conversion edge case, or Telegram rejecting the tags) fall
            // back to sending `plain_text` verbatim with no `parse_mode`
            // rather than losing the notification outright — a failed send
            // here is only logged below, never surfaced to the user, so
            // this retry is the only thing standing between a formatting
            // bug and a silently-dropped ping. Formatting itself runs
            // inside the background-executor spawn below, alongside the
            // blocking HTTP call, rather than on this foreground async
            // loop — a full CommonMark parse of a response up to
            // `TELEGRAM_PREVIEW_HEAD_CHARS` + `_TAIL_CHARS` is real work that
            // shouldn't run on the GPUI thread (mirrors
            // `daruda_acp::node`'s "blocking work stays off the
            // foreground executor" convention).
            let sent = cx
                .background_executor()
                .spawn(async move {
                    let html = html_body(&header, &tail);
                    match client::send_message(
                        &token,
                        chat_id,
                        &html,
                        Some("HTML"),
                        keyboard.clone(),
                    ) {
                        Ok(id) => Ok(id),
                        Err(e) => {
                            // The only place this error is ever named: the
                            // fallback below discards it by design.
                            trace::delivery("send.html_rejected", || {
                                format!("chat_id={chat_id} error={e}")
                            });
                            client::send_message(&token, chat_id, &plain_text, None, keyboard)
                        }
                    }
                })
                .await;

            match sent {
                Ok(message_id) => {
                    trace::state("sent_pings", || {
                        format!(
                            "message_id={message_id} pane={} last_pinged=true",
                            trace::pane(pane)
                        )
                    });
                    cx.update(|cx| {
                        cx.global_mut::<TelegramBridge>()
                            .core
                            .record_sent(message_id, pane);
                    });
                }
                Err(e) => {
                    trace::delivery("send.failed", || format!("chat_id={chat_id} error={e}"));
                    LogWriter::log(
                        ErrorReport::new("Telegram sendMessage failed")
                            .severity(ErrorSeverity::Info)
                            .from_error(&e)
                            .at(file!(), line!())
                            .dedup("telegram.send_message")
                            .build(),
                    );
                }
            }
        }
    })
    .detach();
}
