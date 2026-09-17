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

use crate::remote_channel::dispatch::{
    self,
    target::{aim, compose_edit_body},
};
use gpui::App;

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;

use super::super::bridge::{CallbackEdit, InboundAction, RouteResult};
use super::super::client;
use super::super::keychain;
use super::super::trace;
use super::control::{answer_only, log_unauthorized_inbound, persist_offset, send_command_reply};
use super::{IDLE_RECHECK, POLL_TIMEOUT_SECS, TelegramBridge};
use crate::platform::attention::is_app_active;
use crate::settings_store::SettingsStore;
use crate::surface::strings as s;

/// The inbound long-poll loop. Every iteration re-syncs `enabled` /
/// `authorized_chat_id` from the live `SettingsStore` so a
/// Settings-window toggle takes effect without any extra plumbing —
/// the same pattern `workspace/sync/limits.rs` uses for its poll
/// cadence.
pub(super) fn spawn_poll_task(cx: &mut App) {
    cx.spawn(async move |cx| {
        let lock_root = daruda_store::persistence::remote_lock_root();
        loop {
            let (enabled, chat_id, token, offset) = cx.update(|cx| {
                let cfg = SettingsStore::global(cx).user_arc();
                let bridge = cx.global_mut::<TelegramBridge>();
                bridge.core.set_enabled(cfg.telegram.enabled);
                bridge
                    .core
                    .set_authorized_chat_id(cfg.telegram.authorized_chat_id);
                (
                    bridge.core.is_enabled(),
                    cfg.telegram.authorized_chat_id,
                    keychain::read_token(),
                    bridge.core.current_offset(),
                )
            });
            // Why an idle bridge writes no line of its own: the gate is
            // resynced every iteration, so only its transitions are traced.
            trace::gate_change(enabled, chat_id, token.is_some());

            let Some(token) = token.filter(|_| enabled) else {
                // Let go of the bot on the way out: a daruda that is switched
                // off, or has lost its token, must not keep the next one from
                // serving the phone.
                release_bot(cx);
                cx.background_executor().timer(IDLE_RECHECK).await;
                continue;
            };

            // One daruda per bot. Telegram serves `getUpdates` to whichever
            // poller asked last and answers the rest with a 409, so a second
            // instance both steals replies and has no routing table to place
            // them with — the reply lands on whatever chat it happens to be
            // showing. Asked every iteration rather than once, so the claim is
            // picked up when the holder quits.
            if !claim_bot(&lock_root, &token, cx) {
                cx.background_executor().timer(IDLE_RECHECK).await;
                continue;
            }

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
                    cx.background_executor().timer(IDLE_RECHECK).await;
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
            for update in updates {
                let update_id = update.update_id;
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
                let RouteResult {
                    action,
                    answer_callback_id,
                    callback_edit,
                } = cx.update(|cx| cx.global_mut::<TelegramBridge>().core.route(update));
                trace::delivery("routed", || {
                    format!(
                        "update_id={update_id} action={} answers_callback={}",
                        trace::action_name(&action),
                        answer_callback_id.is_some()
                    )
                });
                let aimed = aim(action, cx);

                if let InboundAction::Paired { chat_id } = aimed.action {
                    cx.update(|cx| {
                        if let Err(error) = cx.global_mut::<SettingsStore>().apply_patch(
                            daruda_config::SettingsPatch::TelegramAuthorizedChatId(Some(chat_id)),
                        ) {
                            crate::remote_channel::log_error(
                                "Telegram pairing failed to persist",
                                &error,
                                "telegram.pair.persist",
                            );
                        }
                    });
                } else {
                    if matches!(aimed.action, InboundAction::Ignore) && answer_callback_id.is_none()
                    {
                        log_unauthorized_inbound();
                    }
                    let effect = dispatch::handle(aimed, &dispatch::Target::Telegram, cx);
                    match (answer_callback_id, effect) {
                        (
                            Some(callback_id),
                            dispatch::Effect::Feedback {
                                label,
                                edit: dispatch::Edit::ConsumeButtons,
                            },
                        ) => {
                            answer_and_edit(cx, &token, callback_id, callback_edit, &label).await;
                        }
                        (Some(callback_id), dispatch::Effect::Feedback { label, .. }) => {
                            answer_only(cx, &token, callback_id, &label).await;
                        }
                        // An unknown or already-consumed token. Answered *and*
                        // edited: leaving the buttons on invites the user to keep
                        // tapping a decision that can no longer land.
                        (Some(callback_id), _) => {
                            let label = s::telegram_permission_stale();
                            answer_and_edit(cx, &token, callback_id, callback_edit, &label).await;
                        }
                        (None, dispatch::Effect::Reply(reply)) => {
                            send_command_reply(cx, &token, reply).await
                        }
                        _ => {}
                    }
                }

                // After acting, not before: a crash in between re-delivers
                // this one update rather than losing it, and writing first
                // would drop the command outright. Per update rather than per
                // batch so the replay is bounded to one.
                persist_offset(cx);
            }

            // No extra sleep on success — the long-poll `timeout_s` itself
            // paces the loop (Telegram returns immediately on data, or
            // after ~timeout_s seconds when idle).
        }
    })
    .detach();
}

/// Take or renew this daruda's claim on the bot, answering whether it may
/// serve. Only re-asks while the claim is not already ours, so a held lock
/// costs nothing per iteration.
fn claim_bot(lock_root: &std::path::Path, token: &str, cx: &mut gpui::AsyncApp) -> bool {
    use crate::remote_channel::lock::{self, Claim};

    cx.update(|cx| {
        let bridge = cx.global_mut::<TelegramBridge>();
        if matches!(bridge.claim, Claim::Ours { .. }) {
            return true;
        }
        bridge.claim = lock::claim(lock_root, token);
        let serving = bridge.claim.serves();
        trace::state("bot.claim", || format!("serving={serving}"));
        serving
    })
}

/// Drop the claim, so the bot is free for whichever daruda is still
/// configured for it.
fn release_bot(cx: &mut gpui::AsyncApp) {
    use crate::remote_channel::lock::Claim;

    cx.update(|cx| {
        let bridge = cx.global_mut::<TelegramBridge>();
        if matches!(bridge.claim, Claim::Ours { .. }) {
            trace::state("bot.claim", || "serving=released".to_string());
        }
        bridge.claim = Claim::Unavailable;
    });
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
