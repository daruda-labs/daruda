//! GPUI wiring for the process-wide Telegram bridge.
//!
//! Owns the long-poll loop, outbound send loop, and routed re-entry into
//! workspaces. Bot API details stay in `client.rs`; routing policy stays in
//! `bridge.rs`.
//!
//! `BridgeCore` is a plain field on the GPUI Global, not `Arc<Mutex<_>>`:
//! globals are mutated on the foreground executor via atomic `cx.update(...)`
//! closures, matching the `WindowRegistry` confinement model.

use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use gpui::{App, Global};

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;

use super::bridge::{
    BotPermissionOutcome, BridgeCore, BridgePing, CallbackEdit, InboundAction, OutboundMsg,
    PermissionDecision, RouteResult, TelegramTail,
};
use super::client;
use super::keychain;
use crate::settings_store::SettingsStore;
use crate::surface::strings as s;
use crate::window_registry::WindowRegistry;
use crate::workspace::Workspace;

/// Telegram long-poll duration per `getUpdates` call. Telegram
/// recommends keeping this well under typical proxy/firewall idle
/// timeouts.
const POLL_TIMEOUT_SECS: u64 = 25;

/// Sleep between poll attempts while disabled, unpaired, or on a
/// transient fetch error — mirrors `workspace/sync/limits.rs`'s
/// `IDLE_RECHECK` idle-backoff idiom.
const IDLE_RECHECK: std::time::Duration = std::time::Duration::from_secs(30);

/// Process-wide Telegram bridge state.
pub struct TelegramBridge {
    core: BridgeCore,
    // `Workspace::relay_to_telegram` sends into this via
    // `cx.try_global::<TelegramBridge>()`.
    outbound_tx: UnboundedSender<BridgePing>,
}

impl Global for TelegramBridge {}

impl TelegramBridge {
    /// Queue a ping for the outbound send loop.
    pub(crate) fn send(&self, ping: BridgePing) {
        let _ = self.outbound_tx.unbounded_send(ping);
    }

    /// Generate a fresh Settings pairing code; only one pairing flow is active.
    pub(crate) fn generate_pair_code(cx: &mut App) -> String {
        cx.global_mut::<TelegramBridge>().core.new_pair_code()
    }
}

#[cfg(test)]
pub(crate) fn install_for_test(
    enabled: bool,
    authorized_chat_id: Option<i64>,
    cx: &mut App,
) -> futures::channel::mpsc::UnboundedReceiver<BridgePing> {
    assert!(
        !cx.has_global::<TelegramBridge>(),
        "TelegramBridge test global must be installed once per test app"
    );
    let core = BridgeCore::new(enabled, authorized_chat_id);
    let (outbound_tx, outbound_rx) = unbounded();
    cx.set_global(TelegramBridge { core, outbound_tx });
    outbound_rx
}

/// Register the Telegram bridge global and spawn its poll + send
/// loops. Call once from `main.rs`, after `SettingsStore::init`.
/// Idempotent (mirrors `agent::skills::global::init`'s `has_global`
/// guard) so a defensive second call — or a test fixture that
/// bootstraps the same App twice — never double-spawns a poll loop
/// against the same `getUpdates` offset.
pub fn install(cx: &mut App) {
    if cx.has_global::<TelegramBridge>() {
        return;
    }

    let cfg = SettingsStore::global(cx).user_arc();
    let core = BridgeCore::new(cfg.telegram.enabled, cfg.telegram.authorized_chat_id);
    let (outbound_tx, outbound_rx) = unbounded();

    cx.set_global(TelegramBridge { core, outbound_tx });

    spawn_poll_task(cx);
    spawn_send_task(outbound_rx, cx);
}

/// The inbound long-poll loop. Every iteration re-syncs `enabled` /
/// `authorized_chat_id` from the live `SettingsStore` so a
/// Settings-window toggle takes effect without any extra plumbing —
/// the same pattern `workspace/sync/limits.rs` uses for its poll
/// cadence.
fn spawn_poll_task(cx: &mut App) {
    cx.spawn(async move |cx| {
        loop {
            let (enabled, token, offset) = cx.update(|cx| {
                let cfg = SettingsStore::global(cx).user_arc();
                let bridge = cx.global_mut::<TelegramBridge>();
                bridge.core.set_enabled(cfg.telegram.enabled);
                bridge
                    .core
                    .set_authorized_chat_id(cfg.telegram.authorized_chat_id);
                (
                    bridge.core.is_enabled(),
                    keychain::read_token(),
                    bridge.core.current_offset(),
                )
            });

            if !enabled {
                cx.background_executor().timer(IDLE_RECHECK).await;
                continue;
            }
            let Some(token) = token else {
                cx.background_executor().timer(IDLE_RECHECK).await;
                continue;
            };

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

            // Route every update while holding the global once, then
            // release it before running any side effect (HTTP calls,
            // cross-workspace dispatch) — routing is the only step that
            // touches `BridgeCore`.
            let dispatch: Vec<(InboundAction, Option<String>, Option<CallbackEdit>)> =
                cx.update(|cx| {
                    let bridge = cx.global_mut::<TelegramBridge>();
                    updates
                        .into_iter()
                        .map(|update| {
                            let RouteResult {
                                action,
                                answer_callback_id,
                                callback_edit,
                            } = bridge.core.route(update);
                            (action, answer_callback_id, callback_edit)
                        })
                        .collect()
                });

            for (action, answer_callback_id, callback_edit) in dispatch {
                match (answer_callback_id, action) {
                    // A permission button tap: apply the decision FIRST so the
                    // feedback is accurate, then answer the callback (toast) and
                    // rewrite the message (drop the buttons + append the outcome).
                    (
                        Some(callback_id),
                        InboundAction::RespondPermission {
                            pane,
                            perm_id,
                            decision,
                        },
                    ) => {
                        let mut outcome = BotPermissionOutcome::Gone;
                        dispatch_to_workspace(cx, pane.workspace, |ws, cx| {
                            outcome =
                                ws.respond_bot_permission(pane.pane, perm_id, decision.clone(), cx);
                        });
                        let label = permission_feedback(&decision, outcome);
                        answer_and_edit(cx, &token, callback_id, callback_edit, &label).await;
                    }
                    // A callback whose token is unknown / already consumed: still
                    // tell the user it was already handled (never leave the tap
                    // silent).
                    (Some(callback_id), _) => {
                        let label = s::telegram_permission_stale();
                        answer_and_edit(cx, &token, callback_id, callback_edit, &label).await;
                    }
                    // A message-origin action (injected reply, pairing, ignore).
                    (None, action) => dispatch_action(action, cx),
                }
            }

            // No extra sleep on success — the long-poll `timeout_s` itself
            // paces the loop (Telegram returns immediately on data, or
            // after ~timeout_s seconds when idle).
        }
    })
    .detach();
}

/// Apply one routed [`InboundAction`]'s side effect. Pure dispatch —
/// no routing policy here, `bridge.rs` already decided what to do.
fn dispatch_action(action: InboundAction, cx: &mut gpui::AsyncApp) {
    match action {
        InboundAction::Ignore => {}
        InboundAction::Paired { chat_id } => {
            // `BridgeCore::route`'s `Paired` branch already updated the
            // in-memory `authorized_chat_id` — this persists it so pairing
            // survives a restart.
            cx.update(|cx| {
                let result = cx.global_mut::<SettingsStore>().apply_patch(
                    daruda_config::SettingsPatch::TelegramAuthorizedChatId(Some(chat_id)),
                );
                if let Err(e) = result {
                    LogWriter::log(
                        ErrorReport::new("Telegram pairing failed to persist")
                            .severity(ErrorSeverity::Warning)
                            .message(e)
                            .at(file!(), line!())
                            .dedup("telegram.pair.persist")
                            .build(),
                    );
                }
            });
        }
        InboundAction::InjectPrompt { pane, text } => {
            dispatch_to_workspace(cx, pane.workspace, |ws, cx| {
                ws.inject_bot_reply(pane.pane, text.clone(), cx)
            });
        }
        InboundAction::RespondPermission { .. } => {
            // Permission taps always arrive as callbacks and are handled inline
            // in the poll loop (apply → answer → edit) so their feedback can be
            // accurate; they never reach this message-origin dispatch path.
        }
    }
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
    let body = compose_edit_body(&edit.original_text, label);
    let edit_token = token.to_string();
    let edited = cx
        .background_executor()
        .spawn(async move {
            client::edit_message_text(&edit_token, edit.chat_id, edit.message_id, &body)
        })
        .await;
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

/// The localized outcome label for a phone-tapped permission decision, shown
/// both as the callback toast and appended to the rewritten message. Applied →
/// the tapped direction (Allow/Reject); otherwise the reason it didn't apply.
fn permission_feedback(decision: &PermissionDecision, outcome: BotPermissionOutcome) -> String {
    match outcome {
        BotPermissionOutcome::Applied => match decision {
            PermissionDecision::Allow(_) => s::telegram_permission_allowed(),
            PermissionDecision::Reject(_) => s::telegram_permission_rejected(),
        },
        BotPermissionOutcome::Stale => s::telegram_permission_stale(),
        BotPermissionOutcome::Gone => s::telegram_permission_gone(),
    }
}

/// Compose the rewritten message body: the original prompt text with the outcome
/// appended on its own line, so the phone keeps the context of what was asked.
/// Falls back to just the label when the original text is unavailable.
fn compose_edit_body(original_text: &str, label: &str) -> String {
    if original_text.is_empty() {
        label.to_string()
    } else {
        format!("{original_text}\n\n— {label}")
    }
}

/// Enter every open `Workspace` window and run `on_match` against the one
/// whose persisted `WorkspaceUuid` equals `workspace` — the shared
/// `WindowRegistry::for_each_workspace` + `ws.uuid() == workspace` scaffolding
/// both [`InboundAction`] dispatch arms above need (a phone-relayed reply /
/// permission decision names its target pane by `PaneRef { workspace, pane }`,
/// but `PaneId` alone is only unique within one open window, so every window
/// must be checked). `pane.pane` itself is *not* checked against anything
/// here — the workspace-side handlers (`inject_bot_reply` /
/// `respond_bot_permission`) already no-op on a stale/gone pane id.
fn dispatch_to_workspace(
    cx: &mut gpui::AsyncApp,
    workspace: daruda_store::project::WorkspaceUuid,
    mut on_match: impl FnMut(&mut Workspace, &mut gpui::Context<Workspace>),
) {
    cx.update(|cx| {
        WindowRegistry::for_each_workspace(cx, |ws, _window, cx| {
            if ws.uuid() == workspace {
                on_match(ws, cx);
            }
        });
    });
}

/// The plain-text fallback body for `header`/`tail`: verbatim, no escaping
/// or markdown parsing. What [`spawn_send_task`] falls back to when the
/// HTML attempt fails. Split out so the header/tail composition is
/// unit-testable without a network mock.
fn plain_body(header: &str, tail: &TelegramTail) -> String {
    let tail_text = match tail {
        TelegramTail::Plain(t) | TelegramTail::Markdown(t) => t.as_str(),
    };
    format!("{header}\n{tail_text}")
}

/// The Telegram-HTML body for `header`/`tail`. `header` and a
/// `TelegramTail::Plain` tail are HTML-escaped only, never markdown-parsed;
/// a `TelegramTail::Markdown` tail is run through the full converter — see
/// [`TelegramTail`]'s doc comment (`bridge.rs`) for why running the
/// markdown parser over plain administrative text (a pane title, a tool
/// name, a raw command) is wrong, not just unnecessary.
fn html_body(header: &str, tail: &TelegramTail) -> String {
    let html_header = super::markdown::escape_text(header);
    let html_tail = match tail {
        TelegramTail::Plain(t) => super::markdown::escape_text(t),
        TelegramTail::Markdown(t) => super::markdown::to_telegram_html(t),
    };
    format!("{html_header}\n{html_tail}")
}

/// The outbound send loop. Drains `outbound_rx` (moved in at spawn
/// time — never stored on the struct) and posts each ping via
/// `client::send_message`, off the foreground thread.
fn spawn_send_task(
    mut outbound_rx: futures::channel::mpsc::UnboundedReceiver<BridgePing>,
    cx: &mut App,
) {
    cx.spawn(async move |cx| {
        while let Some(ping) = outbound_rx.next().await {
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
                continue;
            };

            // Capture `pane` before `build_ping` consumes `ping` by value —
            // `OutboundMsg` does not carry the pane back.
            let pane = ping.pane;

            let msg = cx.update(|cx| cx.global_mut::<TelegramBridge>().core.build_ping(ping));

            let OutboundMsg {
                chat_id,
                header,
                tail,
                keyboard,
            } = msg;
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
            // `TELEGRAM_PREVIEW_THRESHOLD` chars is real work that
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
                        Err(_) => {
                            client::send_message(&token, chat_id, &plain_text, None, keyboard)
                        }
                    }
                })
                .await;

            match sent {
                Ok(message_id) => {
                    cx.update(|cx| {
                        cx.global_mut::<TelegramBridge>()
                            .core
                            .record_sent(message_id, pane);
                    });
                }
                Err(e) => {
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

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;

    use super::*;

    #[test]
    fn compose_edit_body_appends_outcome_and_falls_back_when_empty() {
        assert_eq!(
            compose_edit_body("Allow write to /tmp/x?", "OK"),
            "Allow write to /tmp/x?\n\n— OK"
        );
        // No original text available → just the label, no stray separator.
        assert_eq!(compose_edit_body("", "OK"), "OK");
    }

    #[test]
    fn permission_feedback_distinguishes_direction_and_reason() {
        let allow = PermissionDecision::Allow("opt".into());
        let reject = PermissionDecision::Reject("opt".into());
        let applied_allow = permission_feedback(&allow, BotPermissionOutcome::Applied);
        let applied_reject = permission_feedback(&reject, BotPermissionOutcome::Applied);
        let stale = permission_feedback(&allow, BotPermissionOutcome::Stale);
        let gone = permission_feedback(&allow, BotPermissionOutcome::Gone);

        // Every outcome yields a distinct, non-empty label so the toast/edit is
        // never blank and Allow reads differently from Reject.
        for label in [&applied_allow, &applied_reject, &stale, &gone] {
            assert!(!label.is_empty());
        }
        assert_ne!(applied_allow, applied_reject);
        assert_ne!(applied_allow, stale);
        assert_ne!(stale, gone);
    }

    #[test]
    fn plain_body_uses_the_tail_text_verbatim_for_either_variant() {
        assert_eq!(
            plain_body("title", &TelegramTail::Plain("file: a_b.txt".to_string())),
            "title\nfile: a_b.txt"
        );
        assert_eq!(
            plain_body("title", &TelegramTail::Markdown("**bold**".to_string())),
            "title\n**bold**"
        );
    }

    #[test]
    fn html_body_escapes_a_plain_tail_but_never_markdown_parses_it() {
        // The regression this guards: a raw_input summary or pane title
        // containing incidental markdown-special characters (a file path
        // with underscores, a shell glob) must render as literal text, not
        // get reformatted as CommonMark emphasis.
        let out = html_body(
            "Deploy: rm -rf *.log",
            &TelegramTail::Plain("file: my_file_name.txt".to_string()),
        );
        assert_eq!(out, "Deploy: rm -rf *.log\nfile: my_file_name.txt");
        assert!(
            !out.contains("<i>"),
            "plain tail must not gain emphasis tags"
        );
    }

    #[test]
    fn html_body_markdown_parses_only_the_markdown_tail() {
        let out = html_body(
            "project\nagent",
            &TelegramTail::Markdown("**bold** and `code`".to_string()),
        );
        assert_eq!(out, "project\nagent\n<b>bold</b> and <code>code</code>");
    }

    #[test]
    fn html_body_escapes_html_special_chars_in_the_header() {
        let out = html_body("A & B <panel>", &TelegramTail::Plain("ok".to_string()));
        assert_eq!(out, "A &amp; B &lt;panel&gt;\nok");
    }

    /// `install(cx)` must not double-spawn a poll loop against the same
    /// `getUpdates` offset — a second call is a no-op that leaves the
    /// existing `TelegramBridge` (and its already-running loops) intact.
    #[gpui::test]
    fn install_is_idempotent(cx: &mut TestAppContext) {
        cx.update(|cx| {
            SettingsStore::init(cx);
            install(cx);
            assert!(cx.has_global::<TelegramBridge>());

            // Stamp a sentinel so a clobbering second install is
            // detectable (mirrors `SettingsStore::init_is_idempotent`).
            cx.global_mut::<TelegramBridge>().core.set_enabled(true);

            install(cx);

            assert!(
                cx.global::<TelegramBridge>().core.is_enabled(),
                "second install() must not replace the existing global"
            );
        });
    }
}
