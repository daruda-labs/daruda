//! Bridge routing core — a pure, GPUI-free state machine that turns
//! parsed `Update`s into routing decisions and shapes outbound pings
//! into `OutboundMsg`s. All bridge policy (auth gate, pairing,
//! reply-to resolution, permission-token routing, offset bookkeeping)
//! lives here and is unit-tested with plain function calls.
//!
//! All I/O happens one layer up in `global.rs`'s poll loop, which owns
//! the `BridgeCore` instance. This file must never import `gpui`,
//! `daruda_acp`, or `ureq`.

use std::collections::{HashMap, VecDeque};

use uuid::Uuid;

use crate::telegram::client::{InlineKeyboard, Update, UpdateKind};
use daruda_store::project::WorkspaceUuid;

/// Upper bound on `(message_id -> PaneRef)` entries in `sent_pings`.
/// Oldest entries are evicted first; a reply-to lookup for an evicted
/// message_id falls back to `last_pinged`.
const SENT_PINGS_CAP: usize = 64;

/// Upper bound on outstanding permission-callback tokens in
/// `pending_permissions`. Tokens whose permission is resolved in-app
/// (never tapped on the phone) are never consumed and would otherwise
/// accumulate forever; oldest are evicted first, and an evicted token
/// routes to `Ignore` on a later tap like any unknown one.
const PENDING_PERMISSIONS_CAP: usize = 64;

/// How long a generated `/pair` code stays valid. After this, `/pair`
/// against it is rejected regardless of correctness and the pending
/// code is cleared — the user must generate a fresh code in Settings.
const PAIR_CODE_TTL: std::time::Duration = std::time::Duration::from_secs(600);

/// How many wrong `/pair` guesses a pending code tolerates before it's
/// invalidated. The bot's Telegram username is public/discoverable, so
/// without this an attacker could send unlimited `/pair <guess>`
/// attempts against an unresolved code at no cost.
const PAIR_CODE_MAX_ATTEMPTS: u32 = 5;

/// Identifies one agent-chat pane across the whole process: a
/// workspace (window) uuid plus that workspace's locally-scoped pane
/// id. `PaneId` is only unique within a workspace, so routing an
/// inbound reply needs both halves. Mirrors `LaneRef { project, lane }`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PaneRef {
    pub workspace: WorkspaceUuid,
    pub pane: u64,
}

/// The host's decision on a permission request — deliberately not
/// `daruda_acp::session::PermissionDecision` (this file must not depend
/// on `daruda_acp`). The poll-loop task converts it into the real ACP
/// type at `AcpSessionHandle::respond_permission`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionDecision {
    Allow(String),
    Reject(String),
}

/// Outcome of routing a phone-tapped permission decision into a pane,
/// so the poll loop can give accurate feedback (callback toast +
/// message rewrite). Set by `Workspace::respond_bot_permission`; this
/// pure layer only defines the shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BotPermissionOutcome {
    /// The decision was routed to the agent.
    Applied,
    /// The pane exists but the request was no longer outstanding (already
    /// answered in-app, or the turn was cancelled).
    Stale,
    /// The target pane/view is gone (closed since the prompt was sent).
    Gone,
}

/// What the caller (the poll loop) should do with a routed update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InboundAction {
    /// Nothing to do — unauthorized sender, unmatched /pair, unroutable
    /// text (no reply-to match and no last-pinged pane), or an unknown/
    /// already-consumed callback token.
    Ignore,
    /// A pairing code matched; the caller should persist this chat_id
    /// as `TelegramConfig::authorized_chat_id`. `BridgeCore` already
    /// updated its own in-memory copy, so the caller's persistence is
    /// only for surviving a restart.
    Paired {
        chat_id: i64,
    },
    InjectPrompt {
        pane: PaneRef,
        text: String,
    },
    RespondPermission {
        pane: PaneRef,
        perm_id: u64,
        decision: PermissionDecision,
    },
}

/// `BridgeCore::route`'s full result: the action, plus whether
/// `answerCallbackQuery` must be called (only for `Callback` updates,
/// so the phone's button stops spinning — regardless of the action,
/// including `Ignore`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteResult {
    pub action: InboundAction,
    pub answer_callback_id: Option<String>,
    /// For a `Callback` update, the coordinates + current text of the
    /// tapped message so the caller can rewrite it (drop buttons +
    /// append outcome). `None` for a `Message`. Independent of `action`
    /// because even an `Ignore` callback should still edit the message.
    pub callback_edit: Option<CallbackEdit>,
}

/// Where and what to rewrite after a callback button is tapped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallbackEdit {
    pub chat_id: i64,
    pub message_id: i64,
    /// The message's current display text, so the rewrite can preserve
    /// the original prompt and append an outcome line.
    pub original_text: String,
}

/// One row of inline buttons for a permission-wait ping: every option
/// the agent offered, as pre-formatted `(label, decision)` pairs in the
/// order the in-app card renders them — never collapsed to a single
/// Allow/Reject pair, so richer option sets stay fully choosable from
/// the phone. Labels arrive pre-localized (this file does not touch
/// i18n); `BridgeCore` uses them only to route a later tap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionPromptRef {
    pub perm_id: u64,
    pub buttons: Vec<(String, PermissionDecision)>,
}

/// The event-specific trailing content of a ping, kept separate from
/// the always-plain `header`. Only some pings carry genuine
/// agent-authored markdown safe to run through
/// `telegram::markdown::to_telegram_html`; plain administrative text (a
/// label, a tool title, a `raw_input` summary) must never be
/// markdown-parsed, or incidental punctuation in a path or command
/// (`file_name.txt`, `rm -rf *.log`) gets misread as emphasis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TelegramTail {
    /// Plain text — HTML-escaped for transport, never markdown-parsed.
    Plain(String),
    /// Genuine markdown (the agent's own response text) — markdown-parsed
    /// into Telegram's HTML subset.
    Markdown(String),
}

/// A ping to relay to the phone. `header` and `tail` are pre-formatted,
/// already-localized text whose content `BridgeCore` treats as opaque —
/// it only cares which `TelegramTail` variant `tail` is. `permission`
/// is `Some` only for a permission-wait ping (one button per option),
/// `None` for a plain completion ping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgePing {
    pub pane: PaneRef,
    pub header: String,
    pub tail: TelegramTail,
    pub permission: Option<PermissionPromptRef>,
}

/// The currently pending `/pair` code: text, mint time, and wrong-guess
/// count, so `route_message` can enforce `PAIR_CODE_TTL` expiry and
/// `PAIR_CODE_MAX_ATTEMPTS` throttling.
struct PendingPairCode {
    code: String,
    generated_at: std::time::Instant,
    attempts: u32,
}

/// What the caller should actually send to Telegram.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundMsg {
    pub chat_id: i64,
    pub header: String,
    pub tail: TelegramTail,
    pub keyboard: Option<InlineKeyboard>,
}

/// Has a pairing code minted at `generated_at` aged past `ttl` as of
/// `now`? A free function so it's unit-testable with real `Instant`
/// arithmetic (a past `Instant` via subtraction) — no sleep or clock mock.
fn pair_code_expired(
    generated_at: std::time::Instant,
    now: std::time::Instant,
    ttl: std::time::Duration,
) -> bool {
    now.saturating_duration_since(generated_at) >= ttl
}

/// Pure routing state machine for the Telegram bridge. Owns the
/// `getUpdates` offset, the pairing flow, the reply-to/last-pinged
/// resolution table, and the set of live permission-prompt callback
/// tokens. No network calls, no GPUI — see module docs.
pub struct BridgeCore {
    enabled: bool,
    authorized_chat_id: Option<i64>,
    pending_pair_code: Option<PendingPairCode>,
    update_offset: i64,
    last_pinged: Option<PaneRef>,
    sent_pings: HashMap<i64, PaneRef>,
    sent_pings_order: VecDeque<i64>,
    pending_permissions: HashMap<String, (PaneRef, u64, PermissionDecision)>,
    pending_permissions_order: VecDeque<String>,
}

impl BridgeCore {
    /// Constructs the routing core from `TelegramConfig`'s persisted
    /// settings — called at startup and again on config reload.
    /// `update_offset` always starts at `0` (a fresh `getUpdates` from
    /// zero is safe: Telegram just resends anything still queued).
    pub fn new(enabled: bool, authorized_chat_id: Option<i64>) -> Self {
        Self {
            enabled,
            authorized_chat_id,
            pending_pair_code: None,
            update_offset: 0,
            last_pinged: None,
            sent_pings: HashMap::new(),
            sent_pings_order: VecDeque::new(),
            pending_permissions: HashMap::new(),
            pending_permissions_order: VecDeque::new(),
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Syncs a Settings-window-initiated pairing change (including an
    /// "unpair" that clears it) back into the live `BridgeCore`.
    /// `route()`'s own `Paired` branch still directly mutates the field
    /// for the pairing-success path — this setter is for the poll
    /// loop's per-iteration config resync (`global.rs`).
    ///
    /// Known asymmetry: pairing (`route()`'s `Paired` branch) takes
    /// effect immediately, in-process, the instant a `/pair` message
    /// is routed. Revocation via this setter does not have the same
    /// immediacy — `global.rs`'s poll loop only calls this at the TOP
    /// of each loop iteration, before starting the next `get_updates`
    /// long-poll, so it only runs once the PREVIOUS iteration's
    /// blocking `get_updates` call returns. An unpair click in
    /// Settings can therefore lag by up to `POLL_TIMEOUT_SECS` (~25-30s)
    /// before it actually stops this `BridgeCore` from routing updates
    /// for the revoked chat. This is accepted as low-risk for this
    /// feature's scope (pairing/unpairing is a rare, deliberate action)
    /// — do not assume revocation is as immediate as pairing.
    pub fn set_authorized_chat_id(&mut self, chat_id: Option<i64>) {
        self.authorized_chat_id = chat_id;
    }

    /// The offset the poll loop should pass to the next
    /// `client::get_updates` call.
    pub fn current_offset(&self) -> i64 {
        self.update_offset
    }

    /// Generates a fresh 6-character uppercase hex pairing code,
    /// stores it as the pending code (overwriting any previous one —
    /// only one pairing flow is active at a time; the Settings UI
    /// always shows the *current* code), and returns it.
    pub fn new_pair_code(&mut self) -> String {
        let code = Uuid::new_v4().simple().to_string()[..6].to_uppercase();
        self.pending_pair_code = Some(PendingPairCode {
            code: code.clone(),
            generated_at: std::time::Instant::now(),
            attempts: 0,
        });
        code
    }

    /// Routes one inbound `Update` to a decision. See module docs and
    /// the task spec for the exact policy; the two invariants that
    /// must never regress:
    ///
    /// 1. `update_offset` advances to `max(update_id) + 1`
    ///    unconditionally, even for ignored/unauthorized updates —
    ///    otherwise Telegram resends the same update forever.
    /// 2. Every `Callback` update gets `answer_callback_id: Some(..)`
    ///    regardless of the resulting action — otherwise the phone
    ///    shows a stuck loading spinner on the tapped button.
    pub fn route(&mut self, update: Update) -> RouteResult {
        self.update_offset = self.update_offset.max(update.update_id + 1);

        if !self.enabled {
            let (answer_callback_id, callback_edit) = match &update.kind {
                UpdateKind::Callback {
                    callback_id,
                    chat_id,
                    message_id,
                    message_text,
                    ..
                } => (
                    Some(callback_id.clone()),
                    Some(CallbackEdit {
                        chat_id: *chat_id,
                        message_id: *message_id,
                        original_text: message_text.clone(),
                    }),
                ),
                UpdateKind::Message { .. } => (None, None),
            };
            return RouteResult {
                action: InboundAction::Ignore,
                answer_callback_id,
                callback_edit,
            };
        }

        match update.kind {
            UpdateKind::Message {
                chat_id,
                text,
                reply_to_message_id,
            } => {
                let action = self.route_message(chat_id, text, reply_to_message_id);
                RouteResult {
                    action,
                    answer_callback_id: None,
                    callback_edit: None,
                }
            }
            UpdateKind::Callback {
                chat_id,
                callback_id,
                data,
                message_id,
                message_text,
            } => {
                let action = self.route_callback(chat_id, data);
                RouteResult {
                    action,
                    answer_callback_id: Some(callback_id),
                    callback_edit: Some(CallbackEdit {
                        chat_id,
                        message_id,
                        original_text: message_text,
                    }),
                }
            }
        }
    }

    fn route_message(
        &mut self,
        chat_id: i64,
        text: String,
        reply_to_message_id: Option<i64>,
    ) -> InboundAction {
        if self.authorized_chat_id.is_none()
            && let Some(code) = text.strip_prefix("/pair ")
        {
            let code = code.trim();

            // A code that has aged past its TTL, or already absorbed
            // `PAIR_CODE_MAX_ATTEMPTS` wrong guesses, is dead — clear it
            // so the user must generate a fresh one, regardless of
            // whether THIS guess happens to be correct.
            let dead = self.pending_pair_code.as_ref().is_some_and(|pending| {
                pair_code_expired(
                    pending.generated_at,
                    std::time::Instant::now(),
                    PAIR_CODE_TTL,
                ) || pending.attempts >= PAIR_CODE_MAX_ATTEMPTS
            });
            if dead {
                self.pending_pair_code = None;
                return InboundAction::Ignore;
            }

            let matches = self
                .pending_pair_code
                .as_ref()
                .is_some_and(|pending| pending.code.eq_ignore_ascii_case(code));
            if matches {
                self.authorized_chat_id = Some(chat_id);
                self.pending_pair_code = None;
                return InboundAction::Paired { chat_id };
            }

            if let Some(pending) = self.pending_pair_code.as_mut() {
                pending.attempts += 1;
            }
            return InboundAction::Ignore;
        }

        if self.authorized_chat_id != Some(chat_id) {
            return InboundAction::Ignore;
        }

        let pane = reply_to_message_id
            .and_then(|id| self.sent_pings.get(&id))
            .copied()
            .or(self.last_pinged);

        match pane {
            Some(pane) => InboundAction::InjectPrompt { pane, text },
            None => InboundAction::Ignore,
        }
    }

    fn route_callback(&mut self, chat_id: i64, data: String) -> InboundAction {
        if self.authorized_chat_id != Some(chat_id) {
            return InboundAction::Ignore;
        }

        match self.pending_permissions.remove(&data) {
            Some((pane, perm_id, decision)) => InboundAction::RespondPermission {
                pane,
                perm_id,
                decision,
            },
            None => InboundAction::Ignore,
        }
    }

    /// Shapes a `BridgePing` into an `OutboundMsg` ready for
    /// `client::send_message`. Requires `self.authorized_chat_id` to
    /// be `Some` — the realistic caller already gates on
    /// `enabled && authorized_chat_id.is_some()` before ever building
    /// a ping; if called while unpaired anyway (should not happen in
    /// practice) this falls back to `chat_id: 0`, a request Telegram
    /// will reject, rather than panicking.
    pub fn build_ping(&mut self, ping: BridgePing) -> OutboundMsg {
        debug_assert!(
            self.authorized_chat_id.is_some(),
            "build_ping called before pairing"
        );

        let keyboard = ping.permission.map(|prompt| {
            let buttons: Vec<(String, String)> = prompt
                .buttons
                .into_iter()
                .map(|(label, decision)| {
                    let token = Uuid::new_v4().simple().to_string();
                    self.insert_pending_permission(
                        token.clone(),
                        (ping.pane, prompt.perm_id, decision),
                    );
                    (label, token)
                })
                .collect();

            InlineKeyboard { buttons }
        });

        OutboundMsg {
            chat_id: self.authorized_chat_id.unwrap_or_default(),
            header: ping.header,
            tail: ping.tail,
            keyboard,
        }
    }

    /// Registers one permission-callback token, enforcing the
    /// `PENDING_PERMISSIONS_CAP` bound by evicting the oldest token
    /// (from both the map and its companion order queue) once
    /// exceeded — mirrors `record_sent`'s eviction shape. `build_ping`
    /// calls this once per button in the permission-wait ping's
    /// `PermissionPromptRef::buttons`.
    fn insert_pending_permission(
        &mut self,
        token: String,
        value: (PaneRef, u64, PermissionDecision),
    ) {
        self.pending_permissions.insert(token.clone(), value);
        self.pending_permissions_order.push_back(token);

        if self.pending_permissions_order.len() > PENDING_PERMISSIONS_CAP
            && let Some(oldest) = self.pending_permissions_order.pop_front()
        {
            self.pending_permissions.remove(&oldest);
        }
    }

    /// Records a successfully-sent ping's real Telegram `message_id`
    /// (only known after `client::send_message` returns), so a later
    /// reply-to can resolve back to `pane`. Separate from
    /// `build_ping` because the message_id doesn't exist until
    /// Telegram responds. Enforces the `SENT_PINGS_CAP` bound by
    /// evicting the oldest entry once exceeded; `last_pinged` is left
    /// untouched by eviction — it's a separate fallback path, not
    /// derived from `sent_pings`.
    pub fn record_sent(&mut self, message_id: i64, pane: PaneRef) {
        self.last_pinged = Some(pane);
        self.sent_pings.insert(message_id, pane);
        self.sent_pings_order.push_back(message_id);

        if self.sent_pings_order.len() > SENT_PINGS_CAP
            && let Some(oldest) = self.sent_pings_order.pop_front()
        {
            self.sent_pings.remove(&oldest);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(workspace_seed: u128, id: u64) -> PaneRef {
        PaneRef {
            workspace: WorkspaceUuid(Uuid::from_u128(workspace_seed)),
            pane: id,
        }
    }

    fn message(update_id: i64, chat_id: i64, text: &str, reply_to: Option<i64>) -> Update {
        Update {
            update_id,
            kind: UpdateKind::Message {
                chat_id,
                text: text.to_string(),
                reply_to_message_id: reply_to,
            },
        }
    }

    fn callback(update_id: i64, chat_id: i64, callback_id: &str, data: &str) -> Update {
        Update {
            update_id,
            kind: UpdateKind::Callback {
                chat_id,
                callback_id: callback_id.to_string(),
                data: data.to_string(),
                message_id: update_id,
                message_text: "prompt".to_string(),
            },
        }
    }

    /// A fresh (not-yet-expired, zero-attempts) pending pair code, for
    /// tests that seed `pending_pair_code` directly rather than going
    /// through `new_pair_code()`.
    fn fresh_pending_code(code: &str) -> PendingPairCode {
        PendingPairCode {
            code: code.to_string(),
            generated_at: std::time::Instant::now(),
            attempts: 0,
        }
    }

    #[test]
    fn unauthorized_message_with_no_pending_pair_is_ignored() {
        let mut bridge = BridgeCore::new(true, None);
        let result = bridge.route(message(1, 999, "hello", None));
        assert_eq!(result.action, InboundAction::Ignore);
        assert_eq!(result.answer_callback_id, None);
    }

    #[test]
    fn pair_code_exact_match_pairs_and_authorizes_future_messages() {
        let mut bridge = BridgeCore::new(true, None);
        let code = bridge.new_pair_code();

        let result = bridge.route(message(1, 555, &format!("/pair {code}"), None));
        assert_eq!(result.action, InboundAction::Paired { chat_id: 555 });
        assert_eq!(bridge.authorized_chat_id, Some(555));

        // A follow-up message from the now-authorized chat routes as
        // authorized (falls through to Ignore here only because there's
        // no reply-to / last-pinged pane yet — proves the auth gate
        // passed).
        bridge.last_pinged = Some(pane(1, 1));
        let result = bridge.route(message(2, 555, "ping back", None));
        assert_eq!(
            result.action,
            InboundAction::InjectPrompt {
                pane: pane(1, 1),
                text: "ping back".to_string(),
            }
        );
    }

    #[test]
    fn pair_code_match_is_case_insensitive() {
        let mut bridge = BridgeCore::new(true, None);
        bridge.pending_pair_code = Some(fresh_pending_code("AB12CD"));

        let result = bridge.route(message(1, 42, "/pair ab12cd", None));
        assert_eq!(result.action, InboundAction::Paired { chat_id: 42 });
    }

    #[test]
    fn pair_code_mismatch_is_ignored_and_stays_unauthorized() {
        let mut bridge = BridgeCore::new(true, None);
        bridge.pending_pair_code = Some(fresh_pending_code("AB12CD"));

        let result = bridge.route(message(1, 42, "/pair WRONG1", None));
        assert_eq!(result.action, InboundAction::Ignore);
        assert_eq!(bridge.authorized_chat_id, None);
    }

    #[test]
    fn reply_to_found_in_sent_pings_wins_over_last_pinged() {
        let mut bridge = BridgeCore::new(true, Some(1));
        let reply_target = pane(1, 10);
        let different_last = pane(1, 99);

        bridge.record_sent(41, reply_target);
        bridge.record_sent(50, different_last);
        assert_eq!(bridge.last_pinged, Some(different_last));

        let result = bridge.route(message(1, 1, "answer", Some(41)));
        assert_eq!(
            result.action,
            InboundAction::InjectPrompt {
                pane: reply_target,
                text: "answer".to_string(),
            }
        );
    }

    #[test]
    fn reply_to_missing_from_sent_pings_falls_back_to_last_pinged() {
        let mut bridge = BridgeCore::new(true, Some(1));
        let fallback = pane(1, 7);
        bridge.record_sent(1, fallback);

        let result = bridge.route(message(2, 1, "answer", Some(999)));
        assert_eq!(
            result.action,
            InboundAction::InjectPrompt {
                pane: fallback,
                text: "answer".to_string(),
            }
        );
    }

    #[test]
    fn no_reply_to_and_no_last_pinged_is_ignored() {
        let mut bridge = BridgeCore::new(true, Some(1));
        let result = bridge.route(message(1, 1, "hello", None));
        assert_eq!(result.action, InboundAction::Ignore);
    }

    #[test]
    fn callback_with_known_token_responds_and_consumes_it() {
        let mut bridge = BridgeCore::new(true, Some(1));
        let target = pane(1, 3);
        bridge.pending_permissions.insert(
            "tok-a".to_string(),
            (target, 55, PermissionDecision::Allow("opt_yes".to_string())),
        );

        let result = bridge.route(callback(1, 1, "cbq-1", "tok-a"));
        assert_eq!(
            result.action,
            InboundAction::RespondPermission {
                pane: target,
                perm_id: 55,
                decision: PermissionDecision::Allow("opt_yes".to_string()),
            }
        );
        assert_eq!(result.answer_callback_id, Some("cbq-1".to_string()));

        // Second tap on the same (now-consumed) token is a no-op.
        let result = bridge.route(callback(2, 1, "cbq-2", "tok-a"));
        assert_eq!(result.action, InboundAction::Ignore);
        assert_eq!(result.answer_callback_id, Some("cbq-2".to_string()));
    }

    #[test]
    fn callback_with_unknown_token_is_ignored_but_still_acked() {
        let mut bridge = BridgeCore::new(true, Some(1));
        let result = bridge.route(callback(1, 1, "cbq-x", "never-registered"));
        assert_eq!(result.action, InboundAction::Ignore);
        assert_eq!(result.answer_callback_id, Some("cbq-x".to_string()));
    }

    #[test]
    fn callback_from_unauthorized_chat_is_ignored_but_still_acked() {
        let mut bridge = BridgeCore::new(true, Some(1));
        let result = bridge.route(callback(1, 2, "cbq-y", "irrelevant"));
        assert_eq!(result.action, InboundAction::Ignore);
        assert_eq!(result.answer_callback_id, Some("cbq-y".to_string()));
    }

    #[test]
    fn disabled_bridge_ignores_everything_but_still_acks_callbacks() {
        let mut bridge = BridgeCore::new(false, Some(1));

        let msg_result = bridge.route(message(1, 1, "hello", None));
        assert_eq!(msg_result.action, InboundAction::Ignore);
        assert_eq!(msg_result.answer_callback_id, None);

        let cb_result = bridge.route(callback(2, 1, "cbq-z", "tok"));
        assert_eq!(cb_result.action, InboundAction::Ignore);
        assert_eq!(cb_result.answer_callback_id, Some("cbq-z".to_string()));
    }

    #[test]
    fn offset_advances_to_max_update_id_plus_one_across_ignored_updates() {
        let mut bridge = BridgeCore::new(true, None);
        assert_eq!(bridge.current_offset(), 0);

        bridge.route(message(5, 999, "unauthorized", None));
        assert_eq!(bridge.current_offset(), 6);

        // Lower update_id than what's already been seen must not move
        // the offset backwards.
        bridge.route(message(3, 999, "unauthorized again", None));
        assert_eq!(bridge.current_offset(), 6);

        bridge.route(callback(10, 999, "cbq", "unknown-token"));
        assert_eq!(bridge.current_offset(), 11);
    }

    #[test]
    fn sent_pings_bound_evicts_oldest_entries() {
        let mut bridge = BridgeCore::new(true, Some(1));
        let extra = 5;

        for i in 0..(SENT_PINGS_CAP + extra) as i64 {
            bridge.record_sent(i, pane(1, i as u64));
        }

        assert_eq!(bridge.sent_pings.len(), SENT_PINGS_CAP);
        assert_eq!(bridge.sent_pings_order.len(), SENT_PINGS_CAP);

        // The oldest message_ids (0..extra) were evicted; a reply-to
        // against one of them now falls back to last_pinged instead of
        // resolving directly.
        let evicted_id = 0i64;
        assert!(!bridge.sent_pings.contains_key(&evicted_id));
        let result = bridge.route(message(1000, 1, "late reply", Some(evicted_id)));
        assert_eq!(
            result.action,
            InboundAction::InjectPrompt {
                pane: pane(1, (SENT_PINGS_CAP + extra - 1) as u64),
                text: "late reply".to_string(),
            }
        );

        // A still-present (recent) message_id still resolves directly.
        let surviving_id = (SENT_PINGS_CAP + extra - 1) as i64;
        assert!(bridge.sent_pings.contains_key(&surviving_id));
    }

    #[test]
    fn build_ping_plain_completion_has_no_keyboard() {
        let mut bridge = BridgeCore::new(true, Some(1));
        let msg = bridge.build_ping(BridgePing {
            pane: pane(1, 1),
            header: "Turn finished".to_string(),
            tail: TelegramTail::Plain(String::new()),
            permission: None,
        });

        assert_eq!(msg.chat_id, 1);
        assert_eq!(msg.header, "Turn finished");
        assert!(msg.keyboard.is_none());
        assert!(bridge.pending_permissions.is_empty());
    }

    #[test]
    fn build_ping_permission_registers_two_distinct_tokens() {
        let mut bridge = BridgeCore::new(true, Some(1));
        let msg = bridge.build_ping(BridgePing {
            pane: pane(1, 1),
            header: "Approve this?".to_string(),
            tail: TelegramTail::Plain(String::new()),
            permission: Some(PermissionPromptRef {
                perm_id: 7,
                buttons: vec![
                    (
                        "Allow".to_string(),
                        PermissionDecision::Allow("opt_allow".to_string()),
                    ),
                    (
                        "Reject".to_string(),
                        PermissionDecision::Reject("opt_reject".to_string()),
                    ),
                ],
            }),
        });

        let keyboard = msg.keyboard.expect("keyboard present");
        assert_eq!(keyboard.buttons.len(), 2);
        assert_eq!(keyboard.buttons[0].0, "Allow");
        assert_eq!(keyboard.buttons[1].0, "Reject");
        assert_ne!(keyboard.buttons[0].1, keyboard.buttons[1].1);

        assert_eq!(bridge.pending_permissions.len(), 2);
        let allow_token = &keyboard.buttons[0].1;
        let reject_token = &keyboard.buttons[1].1;
        assert_eq!(
            bridge.pending_permissions.get(allow_token),
            Some(&(
                pane(1, 1),
                7,
                PermissionDecision::Allow("opt_allow".to_string())
            ))
        );
        assert_eq!(
            bridge.pending_permissions.get(reject_token),
            Some(&(
                pane(1, 1),
                7,
                PermissionDecision::Reject("opt_reject".to_string())
            ))
        );
    }

    #[test]
    fn build_ping_permission_registers_one_token_per_button_beyond_two() {
        // The real motivating case: codex-acp can offer more than one
        // Allow-shaped option (Allow Once / Allow for Session / an
        // execpolicy-amendment allow) alongside Reject. Every option the
        // agent offers must become its own button + token, not collapse
        // to a single Allow/Reject pair.
        let mut bridge = BridgeCore::new(true, Some(1));
        let msg = bridge.build_ping(BridgePing {
            pane: pane(1, 1),
            header: "Approve this?".to_string(),
            tail: TelegramTail::Plain(String::new()),
            permission: Some(PermissionPromptRef {
                perm_id: 9,
                buttons: vec![
                    (
                        "Allow Once".to_string(),
                        PermissionDecision::Allow("allow_once".to_string()),
                    ),
                    (
                        "Allow for Session".to_string(),
                        PermissionDecision::Allow("allow_always".to_string()),
                    ),
                    (
                        "Allow Commands Starting With …".to_string(),
                        PermissionDecision::Allow("accept_execpolicy_amendment".to_string()),
                    ),
                    (
                        "Reject".to_string(),
                        PermissionDecision::Reject("reject_once".to_string()),
                    ),
                ],
            }),
        });

        let keyboard = msg.keyboard.expect("keyboard present");
        assert_eq!(keyboard.buttons.len(), 4);
        let labels: Vec<&str> = keyboard.buttons.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(
            labels,
            vec![
                "Allow Once",
                "Allow for Session",
                "Allow Commands Starting With …",
                "Reject",
            ]
        );

        let tokens: std::collections::HashSet<&String> =
            keyboard.buttons.iter().map(|(_, t)| t).collect();
        assert_eq!(tokens.len(), 4, "every button gets its own distinct token");
        assert_eq!(bridge.pending_permissions.len(), 4);

        let execpolicy_token = &keyboard.buttons[2].1;
        assert_eq!(
            bridge.pending_permissions.get(execpolicy_token),
            Some(&(
                pane(1, 1),
                9,
                PermissionDecision::Allow("accept_execpolicy_amendment".to_string())
            ))
        );
    }

    #[test]
    fn pending_permissions_bound_evicts_oldest_entries() {
        let mut bridge = BridgeCore::new(true, Some(1));
        let prompt = || PermissionPromptRef {
            perm_id: 1,
            buttons: vec![
                (
                    "Allow".to_string(),
                    PermissionDecision::Allow("opt_allow".to_string()),
                ),
                (
                    "Reject".to_string(),
                    PermissionDecision::Reject("opt_reject".to_string()),
                ),
            ],
        };

        let first_msg = bridge.build_ping(BridgePing {
            pane: pane(1, 1),
            header: "first".to_string(),
            tail: TelegramTail::Plain(String::new()),
            permission: Some(prompt()),
        });
        let first_keyboard = first_msg.keyboard.expect("keyboard present");
        let first_allow_token = first_keyboard.buttons[0].1.clone();
        let first_reject_token = first_keyboard.buttons[1].1.clone();

        // Each call below registers 2 more tokens; enough calls to push
        // well past the cap (mirrors `sent_pings_bound_evicts_oldest_entries`).
        for _ in 0..PENDING_PERMISSIONS_CAP {
            bridge.build_ping(BridgePing {
                pane: pane(1, 1),
                header: "more".to_string(),
                tail: TelegramTail::Plain(String::new()),
                permission: Some(prompt()),
            });
        }

        assert_eq!(bridge.pending_permissions.len(), PENDING_PERMISSIONS_CAP);
        assert_eq!(
            bridge.pending_permissions_order.len(),
            PENDING_PERMISSIONS_CAP
        );

        // The oldest tokens (from the very first ping) were evicted —
        // a callback tap against one of them is unresolvable, same as
        // any other unknown/consumed token.
        assert!(!bridge.pending_permissions.contains_key(&first_allow_token));
        assert!(!bridge.pending_permissions.contains_key(&first_reject_token));

        let result = bridge.route(callback(1, 1, "cbq-evicted", &first_allow_token));
        assert_eq!(result.action, InboundAction::Ignore);
        assert_eq!(result.answer_callback_id, Some("cbq-evicted".to_string()));
    }

    #[test]
    fn pair_code_expired_not_yet_expired() {
        let now = std::time::Instant::now();
        let generated_at = now - std::time::Duration::from_secs(1);
        assert!(!pair_code_expired(generated_at, now, PAIR_CODE_TTL));
    }

    #[test]
    fn pair_code_expired_at_exact_boundary() {
        let now = std::time::Instant::now();
        let generated_at = now - PAIR_CODE_TTL;
        assert!(pair_code_expired(generated_at, now, PAIR_CODE_TTL));
    }

    #[test]
    fn pair_code_expired_well_past_ttl() {
        let now = std::time::Instant::now();
        let generated_at = now - (PAIR_CODE_TTL * 2);
        assert!(pair_code_expired(generated_at, now, PAIR_CODE_TTL));
    }

    #[test]
    fn pair_code_still_works_within_attempt_and_ttl_budget() {
        // Regression check: a couple of wrong guesses (well under
        // `PAIR_CODE_MAX_ATTEMPTS`) must not invalidate the code — the
        // correct code still pairs afterward.
        let mut bridge = BridgeCore::new(true, None);
        let code = bridge.new_pair_code();

        let result = bridge.route(message(1, 42, "/pair WRONG1", None));
        assert_eq!(result.action, InboundAction::Ignore);
        let result = bridge.route(message(2, 42, "/pair WRONG2", None));
        assert_eq!(result.action, InboundAction::Ignore);

        let result = bridge.route(message(3, 42, &format!("/pair {code}"), None));
        assert_eq!(result.action, InboundAction::Paired { chat_id: 42 });
        assert_eq!(bridge.authorized_chat_id, Some(42));
    }

    #[test]
    fn pair_code_exhausted_by_max_attempts_rejects_even_correct_guess() {
        let mut bridge = BridgeCore::new(true, None);
        let code = bridge.new_pair_code();

        for i in 0..PAIR_CODE_MAX_ATTEMPTS {
            let result = bridge.route(message(i as i64, 42, "/pair WRONGCODE", None));
            assert_eq!(result.action, InboundAction::Ignore);
            assert_eq!(bridge.authorized_chat_id, None);
        }

        // The code is now exhausted (attempts == PAIR_CODE_MAX_ATTEMPTS) —
        // even the CORRECT code is rejected, proving the throttle locks
        // out the code rather than just the wrong guesses.
        let result = bridge.route(message(
            PAIR_CODE_MAX_ATTEMPTS as i64,
            42,
            &format!("/pair {code}"),
            None,
        ));
        assert_eq!(result.action, InboundAction::Ignore);
        assert_eq!(bridge.authorized_chat_id, None);
        assert!(bridge.pending_pair_code.is_none());
    }
}
