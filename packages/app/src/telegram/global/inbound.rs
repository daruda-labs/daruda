//! What to do with an update Telegram sent us.
//!
//! The long-poll loop and everything it dispatches into: routing an inbound
//! update through `BridgeCore`, giving a targetless one the app's own active
//! lane, and carrying out whichever action came back. The outbound half —
//! composing and posting pings — stays in [`super`].
//!
//! Split from `mod.rs` because the two halves share only the `TelegramBridge`
//! global and the bot token: they read different state, fail differently, and
//! neither one's changes should conflict with the other's.

use crate::remote_channel::dispatch::{self, target::compose_edit_body};
use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use gpui::App;
use std::sync::Arc;

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;

use super::super::bridge::{CallbackEdit, InboundAction, RouteResult};
use super::super::client;
use super::super::keychain;
use super::super::trace;
use super::backoff::ErrorBackoff;
use super::bot::{self, Ownership, ReadyBot};
use super::control::{answer_only, log_unauthorized_inbound, send_command_reply};
use super::{IDLE_RECHECK, POLL_TIMEOUT_SECS, TelegramBridge};
use crate::platform::attention::is_app_active;
use crate::settings_store::SettingsStore;
use crate::surface::strings as s;

#[derive(Debug, PartialEq, Eq)]
enum ClaimOutcome {
    Committed,
    Ignored,
    Retry,
}

/// The inbound long-poll loop. Every iteration re-syncs `enabled` /
/// `authorized_chat_id` from the live `SettingsStore` so a
/// Settings-window toggle takes effect without any extra plumbing —
/// the same pattern `workspace/sync/limits.rs` uses for its poll
/// cadence.
pub(super) fn spawn_poll_task(cx: &mut App) {
    let answers = spawn_answer_task(cx);
    cx.spawn(async move |cx| {
        let lock_root = daruda_store::persistence::remote_lock_root();
        let mut backoff = ErrorBackoff::new();
        loop {
            let (enabled, chat_id) = cx.update(|cx| {
                let cfg = SettingsStore::global(cx).user_arc();
                let bridge = cx.global_mut::<TelegramBridge>();
                bridge.core.set_enabled(cfg.telegram.enabled);
                bridge
                    .core
                    .set_authorized_chat_id(cfg.telegram.authorized_chat_id);
                (bridge.core.is_enabled(), cfg.telegram.authorized_chat_id)
            });
            let token = cx
                .background_executor()
                .spawn(async { keychain::snapshot() })
                .await;
            // Why an idle bridge writes no line of its own: the gate is
            // resynced every iteration, so only its transitions are traced.
            trace::gate_change(enabled, chat_id, token.is_some());

            let Some(token) = token.filter(|_| enabled) else {
                // Let go of the bot on the way out: a daruda that is switched
                // off, or has lost its token, must not keep the next one from
                // serving the phone.
                release_bot(cx);
                backoff.reset();
                cx.background_executor().timer(IDLE_RECHECK).await;
                continue;
            };

            // One daruda per bot. Telegram serves `getUpdates` to whichever
            // poller asked last and answers the rest with a 409, so a second
            // instance both steals replies and has no routing table to place
            // them with — the reply lands on whatever chat it happens to be
            // showing. Asked every iteration rather than once, so the claim is
            // picked up when the holder quits.
            let Some(bot) = claim_bot(&lock_root, token, chat_id, cx).await else {
                backoff.reset();
                cx.background_executor().timer(IDLE_RECHECK).await;
                continue;
            };
            let offset = bot
                .inbox
                .snapshot()
                .maximum(&daruda_store::inbox::ChannelKey::new("telegram"))
                .unwrap_or(-1)
                .saturating_add(1);
            let offset = cx.update(|cx| {
                let bridge = cx.global_mut::<TelegramBridge>();
                bridge.core.set_offset(offset);
                bridge.core.current_offset()
            });
            let token = bot.token.token.clone();

            // Run the blocking `ureq` long-poll off the foreground thread —
            // it can hang for up to POLL_TIMEOUT_SECS and must never stall
            // the GPUI event loop.
            let fetch_token = token.clone();
            let fetched = cx
                .background_executor()
                .spawn(async move { client::get_updates(&fetch_token, offset, POLL_TIMEOUT_SECS) })
                .await;

            let updates = match fetched {
                Ok(updates) => updates,
                Err(e) => {
                    trace::delivery("poll.failed", || format!("offset={offset} error={e}"));
                    LogWriter::log(
                        ErrorReport::new("Telegram getUpdates failed")
                            .severity(ErrorSeverity::Info)
                            .from_error(&e)
                            .at(file!(), line!())
                            .dedup("telegram.get_updates")
                            .build(),
                    );
                    // Not `IDLE_RECHECK`: that is the cadence for a bridge with
                    // nothing to do. Every second waited here is a second a
                    // message the phone already sent sits undelivered.
                    cx.background_executor().timer(backoff.delay()).await;
                    continue;
                }
            };

            // Read after the fetch returns, not before it: the long-poll can
            // block for POLL_TIMEOUT_SECS, so a presence reading taken at the
            // top of the iteration would describe the wrong moment entirely.
            let app_active = trace::is_on().then(|| cx.update(|_cx| is_app_active()));
            // `pid` because a debug build and an installed build can both hold
            // a live bridge; without it a line cannot be attributed.
            trace::delivery("poll", || {
                format!(
                    "offset={offset} count={} app_active={} pid={}",
                    updates.len(),
                    trace::opt(app_active),
                    std::process::id(),
                )
            });

            // One update at a time, routed and then acted on before the next
            // is routed. A batch can hold `/use 2` and a plain message that
            // means it: routing them together would resolve the second
            // against the target the first had not yet moved.
            //
            // The global is held only for the routing step and released
            // before any side effect (HTTP calls, cross-workspace dispatch)
            // — routing is the only step that touches `BridgeCore` directly.
            let mut retry_inbox = false;
            for update in updates {
                let update_id = update.update_id;
                match claim_update(&bot, update_id, chat_id, cx, keychain::revision).await {
                    ClaimOutcome::Committed => {}
                    ClaimOutcome::Ignored => continue,
                    ClaimOutcome::Retry => {
                        retry_inbox = true;
                        break;
                    }
                }
                trace::delivery("inbound", || {
                    format!(
                        "update_id={update_id} kind={} chat_id={} app_active={} \
                         payload={}",
                        trace::update_kind_name(&update.kind),
                        trace::opt(trace::update_chat_id(&update.kind)),
                        trace::opt(app_active),
                        trace::update_payload(&update.kind)
                    )
                });
                let dispatched = cx.update(|cx| dispatch_update(&bot, chat_id, update, cx));
                let Some((effect, answer_callback_id, callback_edit)) = dispatched else {
                    continue;
                };
                let effect = effect.settled().await;
                // Queued, not awaited: this loop is the only thing listening
                // for the next message, and one queue keeps the answers in
                // the order the messages came.
                let answer = match (answer_callback_id, effect) {
                    (
                        Some(callback_id),
                        dispatch::Effect::Feedback {
                            label,
                            edit: dispatch::Edit::ConsumeButtons,
                        },
                    ) => Some(Answer::AckAndEdit {
                        callback_id,
                        edit: callback_edit,
                        label,
                    }),
                    (Some(callback_id), dispatch::Effect::Feedback { label, .. }) => {
                        Some(Answer::Ack { callback_id, label })
                    }
                    // An unknown or already-consumed token. Answered *and*
                    // edited: leaving the buttons on invites the user to keep
                    // tapping a decision that can no longer land.
                    (Some(callback_id), _) => Some(Answer::AckAndEdit {
                        callback_id,
                        edit: callback_edit,
                        label: s::notification::telegram_permission_stale(),
                    }),
                    (None, dispatch::Effect::Reply(reply)) => Some(Answer::Reply(reply)),
                    _ => None,
                };
                if let Some(answer) = answer
                    && answers
                        .unbounded_send((bot.clone(), chat_id, answer))
                        .is_err()
                {
                    trace::delivery("answer.dropped", || "reason=queue_closed".to_string());
                }
            }

            if retry_inbox {
                drop(bot);
                cx.background_executor().timer(backoff.delay()).await;
            } else {
                backoff.reset();
            }

            // No extra sleep on success — the long-poll `timeout_s` itself
            // paces the loop (Telegram returns immediately on data, or
            // after ~timeout_s seconds when idle).
        }
    })
    .detach();
}

/// What the phone is owed for one routed update, sent by [`spawn_answer_task`].
enum Answer {
    Reply(crate::remote_channel::command::RenderedReply),
    Ack {
        callback_id: String,
        label: String,
    },
    AckAndEdit {
        callback_id: String,
        edit: Option<CallbackEdit>,
        label: String,
    },
}

/// One task sends every answer in arrival order, so a reply to an earlier
/// message never lands after a later one and a backlog cannot fan out.
fn spawn_answer_task(cx: &mut App) -> UnboundedSender<(Arc<ReadyBot>, Option<i64>, Answer)> {
    let (sender, mut queue) = unbounded::<(Arc<ReadyBot>, Option<i64>, Answer)>();
    cx.spawn(async move |cx| {
        while let Some((bot, chat_id, answer)) = queue.next().await {
            if !cx.update(|cx| live_bot(&bot, chat_id, keychain::revision(), cx)) {
                continue;
            }
            let token = &bot.token.token;
            match answer {
                Answer::Reply(reply) => {
                    if let Some(chat_id) = chat_id {
                        send_command_reply(cx, token, chat_id, reply).await;
                    }
                }
                Answer::Ack { callback_id, label } => {
                    answer_only(cx, token, callback_id, &label).await
                }
                Answer::AckAndEdit {
                    callback_id,
                    edit,
                    label,
                } => answer_and_edit(cx, token, callback_id, edit, &label).await,
            }
        }
    })
    .detach();
    sender
}

/// Reuse the original writer for the same token; a changed bot must acquire
/// its own lease before opening a writable inbox.
async fn claim_bot(
    lock_root: &std::path::Path,
    token: keychain::TokenSnapshot,
    chat_id: Option<i64>,
    cx: &mut gpui::AsyncApp,
) -> Option<Arc<ReadyBot>> {
    let existing = cx.update(|cx| {
        cx.global::<TelegramBridge>()
            .bot
            .ready()
            .filter(|bot| bot.token.token == token.token)
            .cloned()
    });
    let ownership = if let Some(existing) = existing {
        if existing.token.revision == token.revision {
            return Some(existing);
        }
        Ownership::Ready(Arc::new(ReadyBot {
            token,
            inbox: existing.inbox.clone(),
            generation: uuid::Uuid::new_v4(),
        }))
    } else {
        release_bot(cx);
        let lock_root = lock_root.to_owned();
        let opened = cx
            .background_executor()
            .spawn(async move {
                bot::open(
                    &lock_root,
                    &daruda_store::persistence::default_data_dir(),
                    token,
                )
            })
            .await;
        match opened {
            Ok(ownership) => ownership,
            Err(error) => {
                crate::remote_channel::log_error(
                    "Telegram inbox could not open",
                    &error,
                    "telegram.inbox.open",
                );
                return None;
            }
        }
    };
    cx.update(|cx| {
        let cfg = SettingsStore::global(cx).user_arc();
        if !cfg.telegram.enabled || cfg.telegram.authorized_chat_id != chat_id {
            return None;
        }
        if let Ownership::Ready(bot) = &ownership
            && bot.token.revision != keychain::revision()
        {
            return None;
        }
        let result = ownership.ready().cloned();
        if matches!(ownership, Ownership::HeldElsewhere) {
            LogWriter::log(
                ErrorReport::new("Telegram bot held by another daruda")
                    .severity(ErrorSeverity::Warning)
                    .at(file!(), line!())
                    .dedup("telegram.bot_held_elsewhere")
                    .build(),
            );
        }
        cx.global_mut::<TelegramBridge>().bot = ownership;
        result
    })
}

pub(super) fn live_bot(bot: &Arc<ReadyBot>, chat_id: Option<i64>, revision: u64, cx: &App) -> bool {
    let cfg = SettingsStore::global(cx).user_arc();
    bot.token.revision == revision
        && cfg.telegram.enabled
        && cfg.telegram.authorized_chat_id == chat_id
        && cx
            .global::<TelegramBridge>()
            .bot
            .ready()
            .is_some_and(|current| {
                current.generation == bot.generation && Arc::ptr_eq(current, bot)
            })
}

fn dispatch_update(
    bot: &Arc<ReadyBot>,
    chat_id: Option<i64>,
    update: client::Update,
    cx: &mut App,
) -> Option<(dispatch::Effect, Option<String>, Option<CallbackEdit>)> {
    if !live_bot(bot, chat_id, keychain::revision(), cx) {
        return None;
    }
    let config = SettingsStore::global(cx).user_arc();
    let core = &mut cx.global_mut::<TelegramBridge>().core;
    core.set_enabled(config.telegram.enabled);
    core.set_authorized_chat_id(config.telegram.authorized_chat_id);
    let update_id = update.update_id;
    let RouteResult {
        action,
        answer_callback_id,
        callback_edit,
    } = core.route(update);
    trace::delivery("routed", || {
        format!(
            "update_id={update_id} action={} target={} answers_callback={}",
            trace::action_name(&action),
            trace::opt(trace::action_target(&action).map(trace::pane)),
            answer_callback_id.is_some()
        )
    });
    let aimed = dispatch::target::aim_in_app(action, cx);
    let effect = if let InboundAction::Paired { chat_id } = aimed.action {
        if let Err(error) = cx.global_mut::<SettingsStore>().apply_patch(
            daruda_config::SettingsPatch::TelegramAuthorizedChatId(Some(chat_id)),
        ) {
            crate::remote_channel::log_error(
                "Telegram pairing failed to persist",
                &error,
                "telegram.pair.persist",
            );
        }
        dispatch::Effect::None
    } else {
        if matches!(aimed.action, InboundAction::Ignore) && answer_callback_id.is_none() {
            log_unauthorized_inbound();
        }
        dispatch::handle_in_app(aimed, &dispatch::Target::Telegram, cx)
    };
    Some((effect, answer_callback_id, callback_edit))
}

async fn claim_update(
    bot: &Arc<ReadyBot>,
    update_id: i64,
    chat_id: Option<i64>,
    cx: &mut gpui::AsyncApp,
    revision: impl Fn() -> u64,
) -> ClaimOutcome {
    if !cx.update(|cx| live_bot(bot, chat_id, revision(), cx)) {
        return ClaimOutcome::Ignored;
    }
    let claimed = crate::runs::RunsHost::claim_inbound(
        &bot.inbox,
        daruda_store::inbox::ChannelKey::new("telegram"),
        daruda_store::inbox::InboxId::Monotonic(update_id),
    );
    finish_claim_update(bot, chat_id, claimed, cx, revision).await
}

async fn finish_claim_update(
    bot: &Arc<ReadyBot>,
    chat_id: Option<i64>,
    claimed: impl Future<
        Output = Result<
            daruda_store::inbox::Committed<daruda_store::inbox::InboxMachine>,
            daruda_store::inbox::CommitError<daruda_store::inbox::InboxRejected>,
        >,
    >,
    cx: &mut gpui::AsyncApp,
    revision: impl Fn() -> u64,
) -> ClaimOutcome {
    let claimed = claimed.await;
    match claimed {
        Ok(_) => cx.update(|cx| {
            if live_bot(bot, chat_id, revision(), cx) {
                ClaimOutcome::Committed
            } else {
                ClaimOutcome::Ignored
            }
        }),
        Err(daruda_store::inbox::CommitError::Rejected(
            daruda_store::inbox::InboxRejected::Duplicate { .. },
        )) => ClaimOutcome::Ignored,
        Err(error) => {
            crate::remote_channel::log_error(
                "Telegram command claim failed",
                &error,
                "telegram.inbox.claim",
            );
            cx.update(|cx| {
                let owns_writer = cx
                    .global::<TelegramBridge>()
                    .bot
                    .ready()
                    .is_some_and(|current| Arc::ptr_eq(&current.inbox, &bot.inbox));
                if owns_writer {
                    cx.global_mut::<TelegramBridge>().release_claim();
                }
            });
            ClaimOutcome::Retry
        }
    }
}

fn release_bot(cx: &mut gpui::AsyncApp) {
    cx.update(|cx| cx.global_mut::<TelegramBridge>().release_claim());
}

/// Answer a tapped callback with a toast, then rewrite the tapped message to
/// drop its now-consumed buttons and append the outcome. Both are best-effort
/// (logged on failure, never surfaced) — the decision itself was already applied
/// by the caller. `label` is the localized outcome; the message keeps its
/// original prompt text with the outcome appended.
async fn answer_and_edit(
    cx: &mut gpui::AsyncApp,
    token: &str,
    callback_id: String,
    callback_edit: Option<CallbackEdit>,
    label: &str,
) {
    let ack_token = token.to_string();
    let toast = label.to_string();
    let answered = cx
        .background_executor()
        .spawn(async move { client::answer_callback(&ack_token, &callback_id, Some(&toast)) })
        .await;
    trace::delivery("answer_callback", || {
        format!(
            "ok={} edits={} label={}",
            answered.is_ok(),
            callback_edit.is_some(),
            trace::preview(label)
        )
    });
    if let Err(e) = answered {
        LogWriter::log(
            ErrorReport::new("Telegram answerCallbackQuery failed")
                .severity(ErrorSeverity::Info)
                .from_error(&e)
                .at(file!(), line!())
                .dedup("telegram.answer_callback")
                .build(),
        );
    }

    let Some(edit) = callback_edit else {
        return;
    };
    let (edit_chat_id, edit_message_id) = (edit.chat_id, edit.message_id);
    let body = compose_edit_body(&edit.original_text, label);
    let edit_token = token.to_string();
    let edited = cx
        .background_executor()
        .spawn(async move {
            client::edit_message_text(&edit_token, edit.chat_id, edit.message_id, &body)
        })
        .await;
    trace::delivery("edit_message", || {
        format!(
            "chat_id={edit_chat_id} message_id={edit_message_id} ok={}",
            edited.is_ok()
        )
    });
    if let Err(e) = edited {
        LogWriter::log(
            ErrorReport::new("Telegram editMessageText failed")
                .severity(ErrorSeverity::Info)
                .from_error(&e)
                .at(file!(), line!())
                .dedup("telegram.edit_message")
                .build(),
        );
    }
}

#[cfg(test)]
mod tests;
