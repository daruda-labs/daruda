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

use super::bridge::{BridgeCore, BridgePing, Outbound, OutboundMsg, PaneRef, TelegramTail};
use super::client;
use super::keychain;
use super::trace;
use crate::remote_channel::lock::Claim;
use crate::settings_store::SettingsStore;
use crate::surface::strings as s;

mod control;
mod inbound;

use inbound::spawn_poll_task;

use daruda_store::persistence;

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
    /// Last `update_offset` written to disk. Kept beside the core rather than
    /// read back from the file so the guard against a redundant write costs
    /// nothing on an idle poll.
    persisted_offset: i64,
    // `Workspace::relay_to_telegram` sends into this via
    // `cx.try_global::<TelegramBridge>()`.
    outbound_tx: UnboundedSender<Outbound>,
    /// Whether this daruda is the one serving the bot. Taken and refreshed by
    /// the poll loop, read here by everything that would send: Telegram hands
    /// `getUpdates` to one poller per token, so a second instance that kept
    /// sending would put its pings in a conversation it cannot hear the
    /// answers to. Starts [`Claim::Unavailable`] — serving — because until the
    /// loop has asked, no other instance is known to be.
    claim: Claim,
}

impl Global for TelegramBridge {}

impl TelegramBridge {
    pub(crate) fn command_state_mut(
        &mut self,
    ) -> &mut crate::remote_channel::command::CommandState {
        self.core.command_state_mut()
    }

    /// Whether a message can actually go out: the feature is on, someone has
    /// paired, and this daruda is the one serving the bot. The first two are
    /// the conditions `Workspace::telegram_bridge` asks, kept in one place so
    /// the two answers cannot drift apart; the third is enforced again inside
    /// [`Self::send`] and [`Self::send_notice`], which every relay reaches
    /// without passing here.
    fn deliverable(cx: &App) -> bool {
        let cfg = SettingsStore::global(cx).user_arc();
        cfg.telegram.enabled
            && cfg.telegram.authorized_chat_id.is_some()
            && cx
                .try_global::<TelegramBridge>()
                .is_some_and(|bridge| bridge.claim.serves())
    }

    /// Introduce a chat the phone approved opening, and make it the phone's
    /// target. A selection and not just a ping: the orchestrator's own
    /// completion ping follows and would take `last_pinged` straight back.
    /// `false` when nothing can be sent, so the caller can record the drop.
    pub(crate) fn announce_chat(pane: PaneRef, header: String, tail: String, cx: &mut App) -> bool {
        if !Self::deliverable(cx) || cx.try_global::<TelegramBridge>().is_none() {
            return false;
        }
        let bridge = cx.global_mut::<TelegramBridge>();
        bridge.command_state_mut().select(Some(pane));
        bridge.send(BridgePing {
            pane,
            header,
            tail: TelegramTail::Plain(tail),
            permission: None,
        });
        true
    }
    /// Queue a pane-attributed ping for the outbound send loop.
    pub(crate) fn send(&self, ping: BridgePing) {
        if !self.claim.serves() {
            trace::delivery("send.dropped", || {
                format!("reason=bot_held_elsewhere pane={}", trace::pane(ping.pane))
            });
            return;
        }
        let (pane, permission) = (ping.pane, ping.permission.is_some());
        let text = trace::is_on().then(|| trace::tail_digest(&ping.tail));
        match self.outbound_tx.unbounded_send(Outbound::Ping(ping)) {
            Ok(()) => trace::delivery("queue.ping", || {
                format!(
                    "{} pane={} permission={permission} text={}",
                    trace::enqueue_slot(),
                    trace::pane(pane),
                    trace::opt(text)
                )
            }),
            // Only reachable once the send loop is gone, which is to say once
            // nothing will ever be delivered again — worth a line of its own
            // rather than the silent drop this used to be.
            Err(_) => trace::delivery("queue.closed", || {
                format!("kind=ping pane={}", trace::pane(pane))
            }),
        }
    }

    /// Queue a standalone notice — text owed to a command the phone sent,
    /// belonging to no pane. See [`Outbound`] for why the distinction matters.
    pub(crate) fn send_notice(&self, text: String) {
        if !self.claim.serves() {
            trace::delivery("send.dropped", || {
                "reason=bot_held_elsewhere kind=notice".to_string()
            });
            return;
        }
        let digest = trace::is_on().then(|| trace::digest(&text));
        match self.outbound_tx.unbounded_send(Outbound::Notice(text)) {
            Ok(()) => trace::delivery("queue.notice", || {
                format!("{} text={}", trace::enqueue_slot(), trace::opt(digest))
            }),
            Err(_) => trace::delivery("queue.closed", || "kind=notice".to_string()),
        }
    }

    /// Mint an approval card's callback tokens, build it, and queue it.
    ///
    /// All three together so the tokens are registered before the message that
    /// redeems them can be sent, and so nothing outside this module has to
    /// know `BridgeCore` holds the table. `false` when there is no bridge to
    /// ask through.
    pub(crate) fn send_approval_card(
        id: crate::control::approval::ApprovalId,
        summary: String,
        cx: &mut App,
    ) -> bool {
        // All three conditions, before minting anything. `install` runs
        // unconditionally at startup, so the global existing says nothing
        // about whether a message can actually go out — and the send task
        // drops an unsendable card silently, which would leave the caller
        // waiting out the whole approval timeout for a question the user
        // never saw. Same three conditions `Workspace::telegram_bridge`
        // asks, for the same reason: they must not disagree.
        let deliverable = Self::deliverable(cx);
        if !deliverable || cx.try_global::<TelegramBridge>().is_none() {
            trace::delivery("queue.approval.refused", || {
                format!("id={id:?} deliverable={deliverable}")
            });
            return false;
        }
        let bridge = cx.global_mut::<TelegramBridge>();
        let (approve, refuse) = bridge.core.record_pending_approval(id);
        let prompt = crate::telegram::bridge::ApprovalPrompt {
            summary,
            buttons: [
                (s::control_approval_allow(), approve),
                (s::control_approval_refuse(), refuse),
            ],
        };
        trace::state("approval.tokens", || format!("id={id:?} minted=2"));
        let digest = trace::is_on().then(|| trace::digest(&prompt.summary));
        match bridge
            .outbound_tx
            .unbounded_send(Outbound::Approval(prompt))
        {
            Ok(()) => trace::delivery("queue.approval", || {
                format!(
                    "{} id={id:?} text={}",
                    trace::enqueue_slot(),
                    trace::opt(digest)
                )
            }),
            Err(_) => trace::delivery("queue.closed", || format!("kind=approval id={id:?}")),
        }
        true
    }

    /// Drop an approval's callback tokens once it has been decided.
    ///
    /// Settle-time cleanup, not tap-time: while a card is live, tapping twice
    /// is ordinary use, so the tokens have to survive a tap. Once the request
    /// is answered they are dead weight — and the table is bounded, so leaving
    /// them would eventually evict a *live* card's tokens and leave the user
    /// tapping a button that resolves nothing.
    pub(crate) fn forget_approval(id: crate::control::approval::ApprovalId, cx: &mut App) {
        if cx.try_global::<TelegramBridge>().is_some() {
            trace::state("approval.forgotten", || format!("id={id:?}"));
            cx.global_mut::<TelegramBridge>()
                .core
                .forget_pending_approval(id);
        }
    }

    /// Generate a fresh Settings pairing code; only one pairing flow is active.
    pub(crate) fn generate_pair_code(cx: &mut App) -> String {
        // The value is deliberately absent: it authorizes a chat, and this
        // file is plain text that outlives the pairing window.
        trace::state("pair_code.minted", || "replaced=pending".to_string());
        cx.global_mut::<TelegramBridge>().core.new_pair_code()
    }
}

/// Seed the bridge with a claim another daruda holds, so a test can drive the
/// only state the poll loop reaches by asking the filesystem.
#[cfg(test)]
pub(crate) fn hold_bot_elsewhere_for_test(cx: &mut App) {
    cx.global_mut::<TelegramBridge>().claim = crate::remote_channel::lock::Claim::Theirs;
}

#[cfg(test)]
pub(crate) fn install_for_test(
    enabled: bool,
    authorized_chat_id: Option<i64>,
    cx: &mut App,
) -> futures::channel::mpsc::UnboundedReceiver<Outbound> {
    assert!(
        !cx.has_global::<TelegramBridge>(),
        "TelegramBridge test global must be installed once per test app"
    );
    let core = BridgeCore::new(enabled, authorized_chat_id, 0);
    let (outbound_tx, outbound_rx) = unbounded();
    cx.set_global(TelegramBridge {
        core,
        persisted_offset: 0,
        outbound_tx,
        claim: Claim::Unavailable,
    });
    outbound_rx
}

/// How many approval tokens the bridge is still holding. Lets a test assert
/// that a settled card leaves none — the property that keeps a bounded table
/// from evicting a live card's buttons.
#[cfg(test)]
pub(crate) fn pending_approval_tokens_for_test(cx: &App) -> usize {
    cx.try_global::<TelegramBridge>()
        .map_or(0, |b| b.core.pending_approval_token_count())
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
    let core = BridgeCore::new(
        cfg.telegram.enabled,
        cfg.telegram.authorized_chat_id,
        daruda_store::telegram::load_telegram_state_in(&persistence::default_data_dir())
            .update_offset,
    );
    let (outbound_tx, outbound_rx) = unbounded();

    cx.set_global(TelegramBridge {
        persisted_offset: core.current_offset(),
        core,
        outbound_tx,
        claim: Claim::Unavailable,
    });

    spawn_poll_task(cx);
    spawn_send_task(outbound_rx, cx);
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

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;

    use super::*;

    /// A bot another daruda is serving is not this one's to speak into: the phone
    /// already has a daruda answering it, and a second sender doubles every ping
    /// while remembering only its own as the chat a plain reply belongs to.
    #[gpui::test]
    async fn a_bot_another_daruda_holds_queues_nothing(cx: &mut gpui::TestAppContext) {
        use futures::{FutureExt as _, StreamExt as _};

        cx.update(|cx| {
            let mut outbound = install_for_test(true, Some(42), cx);
            hold_bot_elsewhere_for_test(cx);
            let pane = PaneRef {
                workspace: Default::default(),
                pane: 7,
            };

            cx.global::<TelegramBridge>().send(BridgePing {
                pane,
                header: "daruda/main".into(),
                tail: TelegramTail::Plain("done".into()),
                permission: None,
            });
            cx.global::<TelegramBridge>().send_notice("hello".into());

            assert!(
                outbound.next().now_or_never().is_none(),
                "nothing goes out on a bot this daruda is not serving"
            );
        });
    }

    /// The announcement both points the phone at the new chat and tells it
    /// so, as a ping attributed to that chat — a reply to it reaches the chat.
    #[gpui::test]
    fn announcing_a_chat_selects_it_and_pings_it(cx: &mut TestAppContext) {
        use futures::{FutureExt as _, StreamExt as _};
        cx.update(|cx| {
            crate::settings_store::SettingsStore::init(cx);
            cx.global_mut::<crate::settings_store::SettingsStore>()
                .set_user_for_testing(daruda_config::Config {
                    telegram: daruda_config::TelegramConfig {
                        enabled: true,
                        authorized_chat_id: Some(42),
                        ..Default::default()
                    },
                    ..daruda_config::Config::default()
                });
            let mut outbound = install_for_test(true, Some(42), cx);
            let pane = PaneRef {
                workspace: Default::default(),
                pane: 7,
            };
            assert!(TelegramBridge::announce_chat(
                pane,
                "daruda/main".into(),
                "new tab".into(),
                cx
            ));
            assert_eq!(
                cx.global_mut::<TelegramBridge>()
                    .command_state_mut()
                    .selected(),
                Some(pane)
            );
            match outbound.next().now_or_never().flatten() {
                Some(Outbound::Ping(ping)) => assert_eq!(ping.pane, pane),
                _ => panic!("the announcement is a pane-attributed ping"),
            }
        });
    }

    /// Unpaired, nothing goes out and nothing is selected — the caller hears
    /// `false` and records the drop instead of the user hearing nothing.
    #[gpui::test]
    fn an_unpaired_bridge_refuses_to_announce(cx: &mut TestAppContext) {
        use futures::{FutureExt as _, StreamExt as _};
        cx.update(|cx| {
            crate::settings_store::SettingsStore::init(cx);
            let mut outbound = install_for_test(true, None, cx);
            let pane = PaneRef {
                workspace: Default::default(),
                pane: 7,
            };
            assert!(!TelegramBridge::announce_chat(
                pane,
                "h".into(),
                "t".into(),
                cx
            ));
            assert_eq!(
                cx.global_mut::<TelegramBridge>()
                    .command_state_mut()
                    .selected(),
                None
            );
            assert!(outbound.next().now_or_never().is_none());
        });
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
