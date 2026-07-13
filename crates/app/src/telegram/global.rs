//! GPUI wiring for the Telegram bridge — a process-wide background
//! service that owns the `getUpdates` long-poll loop, the outbound
//! send loop, and re-entry into whichever `Workspace` a routed
//! `Update` targets.
//!
//! This is the only file in `crate::telegram` that touches GPUI. All
//! Bot API wire knowledge stays in `client.rs` and all routing policy
//! (auth gate, pairing, reply-to resolution, token bookkeeping) stays
//! in `bridge.rs`; this file only calls `core.route()` /
//! `core.build_ping()` / `core.record_sent()` and dispatches on the
//! returned [`InboundAction`].
//!
//! `BridgeCore` lives as a plain owned field on the `TelegramBridge`
//! Global rather than behind `Arc<Mutex<..>>`: GPUI Globals are only
//! ever mutated from the single foreground executor thread via
//! `cx.global_mut::<T>()`, and every `cx.update(...)` closure runs
//! atomically to completion before the next one starts — the same
//! reasoning `WindowRegistry` already relies on. See
//! `crate::watcher_pumps` for the detached-pump idiom this module
//! follows.

use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use gpui::{App, Global};

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;

use super::bridge::{
    BridgeCore, BridgePing, InboundAction, OutboundMsg, RouteResult, TelegramTail,
};
use super::client;
use super::keychain;
use crate::settings_store::SettingsStore;
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

/// Process-wide Telegram bridge state. `core` owns the pure routing
/// state machine; `outbound_tx` is the send side of the channel the
/// poll/send loops share — a later task (`Workspace::relay_to_telegram`)
/// reaches it via `cx.global_mut::<TelegramBridge>()`.
pub struct TelegramBridge {
    core: BridgeCore,
    // `Workspace::relay_to_telegram` sends into this via
    // `cx.try_global::<TelegramBridge>()`.
    outbound_tx: UnboundedSender<BridgePing>,
}

impl Global for TelegramBridge {}

impl TelegramBridge {
    /// Queue a ping for the outbound send loop. Fire-and-forget — a send
    /// failure means the send task has already ended (app shutting down),
    /// which the caller learns about some other way; mirrors
    /// `AcpSessionHandle::send_prompt`'s `let _ = ...unbounded_send(...)`
    /// idiom used throughout this codebase.
    pub(crate) fn send(&self, ping: BridgePing) {
        let _ = self.outbound_tx.unbounded_send(ping);
    }

    /// Generate a fresh pairing code for display in Settings. Overwrites
    /// any previously-generated code that wasn't used — only one pairing
    /// flow is active at a time (`BridgeCore::new_pair_code`'s own doc).
    pub(crate) fn generate_pair_code(cx: &mut App) -> String {
        cx.global_mut::<TelegramBridge>().core.new_pair_code()
    }
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
            let dispatch: Vec<(InboundAction, Option<String>)> = cx.update(|cx| {
                let bridge = cx.global_mut::<TelegramBridge>();
                updates
                    .into_iter()
                    .map(|update| {
                        let RouteResult {
                            action,
                            answer_callback_id,
                        } = bridge.core.route(update);
                        (action, answer_callback_id)
                    })
                    .collect()
            });

            for (action, answer_callback_id) in dispatch {
                if let Some(callback_id) = answer_callback_id {
                    let ack_token = token.clone();
                    let result = cx
                        .background_executor()
                        .spawn(async move { client::answer_callback(&ack_token, &callback_id) })
                        .await;
                    if let Err(e) = result {
                        LogWriter::log(
                            ErrorReport::new("Telegram answerCallbackQuery failed")
                                .severity(ErrorSeverity::Info)
                                .from_error(&e)
                                .at(file!(), line!())
                                .dedup("telegram.answer_callback")
                                .build(),
                        );
                    }
                }

                dispatch_action(action, cx);
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
                let result = cx
                    .global_mut::<SettingsStore>()
                    .patch_user(|c| c.telegram.authorized_chat_id = Some(chat_id));
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
        InboundAction::RespondPermission { pane, decision, .. } => {
            dispatch_to_workspace(cx, pane.workspace, |ws, cx| {
                ws.respond_bot_permission(pane.pane, decision.clone(), cx)
            });
        }
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
/// or markdown parsing — exactly what used to be sent unconditionally
/// before HTML formatting was added, and what [`spawn_send_task`] falls
/// back to when the HTML attempt fails. Split out (mirrors
/// `client.rs::parse_send_message_response`'s "split so tests can exercise
/// it directly" reasoning) so the header/tail composition is unit-testable
/// without a network mock.
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
