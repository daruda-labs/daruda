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
    BotPermissionOutcome, BridgeCore, BridgePing, CallbackEdit, InboundAction, Outbound,
    OutboundMsg, PermissionDecision, RouteResult, TelegramTail,
};
use super::client;
use super::keychain;
use super::trace;
use crate::control::result::ControlError;
use crate::platform::attention::is_app_active;
use crate::settings_store::SettingsStore;
use crate::surface::strings as s;
use crate::window_registry::WindowRegistry;
use crate::workspace::Workspace;

mod control;

use control::{
    answer_only, log_unauthorized_inbound, persist_offset, render_outcome, report_target_gone,
    run_command, select_target, send_command_reply,
};
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
}

impl Global for TelegramBridge {}

impl TelegramBridge {
    /// Queue a pane-attributed ping for the outbound send loop.
    pub(crate) fn send(&self, ping: BridgePing) {
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
        let deliverable = {
            let cfg = SettingsStore::global(cx).user_arc();
            cfg.telegram.enabled && cfg.telegram.authorized_chat_id.is_some()
        };
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
    });

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
            trace::delivery("poll", || {
                format!(
                    "offset={offset} count={} app_active={}",
                    updates.len(),
                    trace::opt(app_active)
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
                        trace::delivery("permission", || {
                            format!(
                                "pane={} perm_id={perm_id} decision={decision:?} \
                                 outcome={outcome:?}",
                                trace::pane(pane)
                            )
                        });
                        let label = permission_feedback(&decision, outcome);
                        answer_and_edit(cx, &token, callback_id, callback_edit, &label).await;
                    }
                    // A listing button tap: point the target at that pane and
                    // toast which one. The message keeps its buttons — tapping
                    // another row is ordinary use, not a second decision.
                    (Some(callback_id), InboundAction::SelectTarget { pane }) => {
                        let label = cx.update(|cx| select_target(pane, cx));
                        answer_only(cx, &token, callback_id, &label).await;
                    }
                    // An approval button tap. The card keeps its buttons: the
                    // tool call may still be waiting, and a second tap is
                    // ordinary use rather than a second decision.
                    (Some(callback_id), InboundAction::ResolveApproval { id, choice }) => {
                        // The label follows what *happened*, not what was
                        // tapped: the card keeps its buttons while it is live,
                        // so a second, contradictory tap must not be told it
                        // took effect. Same rule as `permission_feedback`.
                        let settled =
                            cx.update(|cx| crate::control::approval::resolve(id, choice, cx));
                        let label = match (settled, choice) {
                            (false, _) => s::control_approval_already_answered(),
                            (true, crate::control::approval::ApprovalChoice::Approved) => {
                                s::control_approval_allowed()
                            }
                            (true, crate::control::approval::ApprovalChoice::Refused) => {
                                s::control_approval_refused()
                            }
                        };
                        trace::delivery("approval.tap", || {
                            format!("id={id:?} choice={choice:?} settled={settled}")
                        });
                        answer_only(cx, &token, callback_id, &label).await;
                    }
                    // A tap on a superseded listing. Answered with its own
                    // wording and *not* edited — `answer_and_edit` drops the
                    // message's keyboard, which would strip the rows off a
                    // listing the user is still reading.
                    (Some(callback_id), InboundAction::StaleListing) => {
                        answer_only(cx, &token, callback_id, &s::control_listing_stale()).await;
                    }
                    // A callback whose token is unknown / already consumed: still
                    // tell the user it was already handled (never leave the tap
                    // silent).
                    (Some(callback_id), _) => {
                        let label = s::telegram_permission_stale();
                        answer_and_edit(cx, &token, callback_id, callback_edit, &label).await;
                    }
                    // A command: resolve, run, fold the outcome back into the
                    // adapter's ordinal table, answer.
                    (None, InboundAction::RunCommand { command }) => {
                        let reply = run_command(command, cx);
                        send_command_reply(cx, &token, reply).await;
                    }
                    // Never swallowed: a mistyped slash command that produced
                    // silence is the defect this replaces.
                    (None, InboundAction::ReportParseError { error }) => {
                        let reply = super::command::render_parse_error(&error);
                        send_command_reply(cx, &token, reply).await;
                    }
                    (None, InboundAction::NoTarget) => {
                        let reply = cx
                            .update(|cx| render_outcome(&Err(ControlError::NoTargetSelected), cx));
                        send_command_reply(cx, &token, reply).await;
                    }
                    // A plain message routed to a remembered pane. The pane
                    // can be gone — a selection and a last-pinged target both
                    // outlive the pane they name — so the delivery is checked
                    // rather than assumed.
                    (None, InboundAction::InjectPrompt { pane, text }) => {
                        deliver_prompt(cx, &token, pane, text).await;
                    }
                    // A slash daruda does not own. Which vocabulary it belongs
                    // to is the *pane's* to answer — it advertises its own
                    // commands — and `route` is GPUI-free, so the question is
                    // settled here and then rejoins the two paths that already
                    // handle each outcome.
                    (
                        None,
                        InboundAction::UnknownSlash {
                            pane,
                            name,
                            text,
                            suggestion,
                        },
                    ) => {
                        let mut agents = false;
                        dispatch_to_workspace(cx, pane.workspace, |ws, cx| {
                            agents = ws.agent_takes_slash_command(pane.pane, &name, cx);
                        });
                        trace::delivery("slash.unowned", || {
                            format!("pane={} name={name} to_agent={agents}", trace::pane(pane))
                        });
                        if agents {
                            deliver_prompt(cx, &token, pane, text).await;
                        } else {
                            let reply = super::command::render_parse_error(
                                &crate::control::spec::ParseError::Unknown {
                                    input: name,
                                    suggestion,
                                },
                            );
                            send_command_reply(cx, &token, reply).await;
                        }
                    }
                    // A message-origin action (pairing, ignore, unsupported).
                    (None, action) => dispatch_action(action, cx),
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

/// How long one "unauthorized inbound" line suppresses the next.
///
/// A bot's username is publicly discoverable, so anyone can send it messages —
/// and `ErrorReport::dedup` does *not* help here: `LogWriter` writes every
/// report it is given, and `dedup_key` only merges toasts (see
/// `workspace::error::toast`). Without a real window, a probing sender writes
/// one NDJSON line per message and drowns genuine diagnostics./// Put `text` on `pane`, answering the sender when the pane is gone.
///
/// The pane can be: a selection and a last-pinged target both outlive the pane
/// they name, so the delivery is checked rather than assumed. Shared by the
/// plain-message path and by a slash the agent claimed — the two differ in how
/// they were routed, not in how they are delivered.
async fn deliver_prompt(
    cx: &mut gpui::AsyncApp,
    token: &str,
    pane: crate::telegram::bridge::PaneRef,
    text: String,
) {
    let mut delivered = false;
    dispatch_to_workspace(cx, pane.workspace, |ws, cx| {
        delivered = ws.inject_bot_reply(pane.pane, text.clone(), cx);
    });
    trace::delivery("inject", || {
        format!(
            "pane={} delivered={delivered} text={}",
            trace::pane(pane),
            trace::preview(&text)
        )
    });
    if !delivered {
        let reply = cx.update(|cx| report_target_gone(pane, cx));
        send_command_reply(cx, token, reply).await;
    }
}

/// Apply one routed [`InboundAction`]'s side effect. Pure dispatch —
/// no routing policy here, `bridge.rs` already decided what to do.
fn dispatch_action(action: InboundAction, cx: &mut gpui::AsyncApp) {
    match action {
        InboundAction::Ignore => log_unauthorized_inbound(),
        InboundAction::Paired { chat_id } => {
            trace::state("paired", || format!("chat_id={chat_id}"));
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
        InboundAction::InjectPrompt { .. } => {
            // Handled inline in the poll loop, which is the only place that
            // can await the "that chat is gone" answer.
        }
        InboundAction::UnknownSlash { .. } => {
            // Same: settled in the poll loop, which can both read the pane's
            // command list and await whichever answer that settles on.
        }
        InboundAction::RespondPermission { .. }
        | InboundAction::SelectTarget { .. }
        | InboundAction::ResolveApproval { .. }
        | InboundAction::StaleListing => {
            // All three always arrive as callbacks and are handled inline in
            // the poll loop, where their feedback can be accurate; they never
            // reach this message-origin dispatch path.
        }
        InboundAction::Unsupported => {
            // Nothing to do by construction — it reached `route` only so its
            // id could advance the `getUpdates` offset.
        }
        InboundAction::RunCommand { .. }
        | InboundAction::ReportParseError { .. }
        | InboundAction::NoTarget => {
            // Answered inline in the poll loop, which is the only place that
            // can await the reply's send.
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
/// here — the workspace-side handlers report a stale/gone pane id back to the
/// caller (`inject_bot_reply` returns `false`, `respond_bot_permission`
/// returns `Gone`), which then answers the phone rather than going quiet.
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
            let (enabled, chat_id) = cx.update(|cx| {
                let cfg = SettingsStore::global(cx).user_arc();
                let bridge = cx.global_mut::<TelegramBridge>();
                bridge.core.set_enabled(cfg.telegram.enabled);
                bridge
                    .core
                    .set_authorized_chat_id(cfg.telegram.authorized_chat_id);
                (cfg.telegram.enabled, cfg.telegram.authorized_chat_id)
            });
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
