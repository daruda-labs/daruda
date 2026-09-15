//! Bridge routing core — a pure, GPUI-free state machine that turns parsed
//! `Update`s into routing decisions and shapes outbound pings into
//! `OutboundMsg`s. All bridge policy (auth gate, pairing, reply-to
//! resolution, permission-token routing, offset bookkeeping) lives here and
//! is unit-tested with plain function calls.
//!
//! The vocabulary it decides in lives in [`super`]; this file is the rules.

use uuid::Uuid;

use super::{BridgePing, CallbackEdit, InboundAction, OutboundMsg, PaneRef, RouteResult, Routed};
use crate::remote_channel::bridge::RoutingCore;
#[cfg(test)]
use crate::remote_channel::bridge::core::APPROVAL_TOKEN_PREFIX;
#[cfg(test)]
use crate::remote_channel::bridge::core::{PENDING_PERMISSIONS_CAP, SENT_PINGS_CAP};
#[cfg(test)]
use crate::remote_channel::bridge::{InlineKeyboard, PermissionDecision, Unaimed};
use crate::telegram::client::{Update, UpdateKind};

#[cfg(test)]
mod tests;

/// How long a generated `/pair` code stays valid. After this, `/pair`
/// against it is rejected regardless of correctness and the pending
/// code is cleared — the user must generate a fresh code in Settings.
const PAIR_CODE_TTL: std::time::Duration = std::time::Duration::from_secs(600);

/// How many wrong `/pair` guesses a pending code tolerates before it's
/// invalidated. The bot's Telegram username is public/discoverable, so
/// without this an attacker could send unlimited `/pair <guess>`
/// attempts against an unresolved code at no cost.
const PAIR_CODE_MAX_ATTEMPTS: u32 = 5;

/// The currently pending `/pair` code: text, mint time, and wrong-guess
/// count, so `route_message` can enforce `PAIR_CODE_TTL` expiry and
/// `PAIR_CODE_MAX_ATTEMPTS` throttling.
struct PendingPairCode {
    code: String,
    generated_at: std::time::Instant,
    attempts: u32,
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
    pub(crate) routing: RoutingCore,
}

impl BridgeCore {
    /// Constructs the routing core from `TelegramConfig`'s persisted
    /// settings — called at startup and again on config reload.
    /// `update_offset` is seeded from the persisted high-water mark so a
    /// restart does not re-deliver, and re-run, a command that was already
    /// processed.
    pub fn new(enabled: bool, authorized_chat_id: Option<i64>, update_offset: i64) -> Self {
        Self {
            enabled,
            authorized_chat_id,
            pending_pair_code: None,
            update_offset,
            routing: RoutingCore::default(),
        }
    }

    /// The ordinal table and selected target for the authorized chat. The poll
    /// loop reaches through this to resolve an ordinal against the listing the
    /// user actually saw, and to record the next one.
    pub fn command_state_mut(&mut self) -> &mut crate::telegram::command::CommandState {
        self.routing.command_state_mut()
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

        // Handled before the `enabled` gate, and before anything else: the
        // offset advanced above, which is the entire reason an update the
        // bridge cannot act on is carried here rather than dropped by the
        // parser. Dropping one strands the offset behind it and Telegram
        // re-delivers it immediately, in a tight loop, forever.
        if matches!(update.kind, UpdateKind::Unsupported) {
            return RouteResult {
                action: Routed::Ready(InboundAction::Unsupported),
                answer_callback_id: None,
                callback_edit: None,
            };
        }

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
                UpdateKind::Message { .. } | UpdateKind::Unsupported => (None, None),
            };
            return RouteResult {
                action: Routed::Ready(InboundAction::Ignore),
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
            // Answered above; the match is exhaustive without a wildcard so a
            // future variant cannot slip through unrouted.
            UpdateKind::Unsupported => RouteResult {
                action: Routed::Ready(InboundAction::Unsupported),
                answer_callback_id: None,
                callback_edit: None,
            },
            UpdateKind::Callback {
                chat_id,
                callback_id,
                data,
                message_id,
                message_text,
            } => {
                // A callback names its pane through the token it carries, so
                // it never reaches the target question.
                let action = Routed::Ready(self.route_callback(chat_id, data));
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
    ) -> Routed {
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
                return Routed::Ready(InboundAction::Ignore);
            }

            let matches = self
                .pending_pair_code
                .as_ref()
                .is_some_and(|pending| pending.code.eq_ignore_ascii_case(code));
            if matches {
                self.authorized_chat_id = Some(chat_id);
                self.pending_pair_code = None;
                return Routed::Ready(InboundAction::Paired { chat_id });
            }

            if let Some(pending) = self.pending_pair_code.as_mut() {
                pending.attempts += 1;
            }
            return Routed::Ready(InboundAction::Ignore);
        }

        if self.authorized_chat_id != Some(chat_id) {
            return Routed::Ready(InboundAction::Ignore);
        }

        self.routing.route_text(text, reply_to_message_id)
    }

    fn route_callback(&mut self, chat_id: i64, data: String) -> InboundAction {
        if self.authorized_chat_id != Some(chat_id) {
            return InboundAction::Ignore;
        }

        self.routing.route_callback(data)
    }

    pub fn record_pending_approval(
        &mut self,
        id: crate::control::approval::ApprovalId,
    ) -> (String, String) {
        self.routing.record_pending_approval(id)
    }

    #[cfg(test)]
    pub fn pending_approval_token_count(&self) -> usize {
        self.routing.pending_approval_token_count()
    }

    pub fn forget_pending_approval(&mut self, id: crate::control::approval::ApprovalId) {
        self.routing.forget_pending_approval(id);
    }

    pub fn build_ping(&mut self, ping: BridgePing) -> OutboundMsg {
        debug_assert!(self.authorized_chat_id.is_some());
        let prepared = self.routing.build_ping(ping);
        OutboundMsg {
            chat_id: self.authorized_chat_id.unwrap_or_default(),
            header: prepared.header,
            tail: prepared.tail,
            keyboard: prepared.keyboard,
        }
    }

    pub fn record_sent(&mut self, message_id: i64, pane: PaneRef) {
        self.routing.record_sent(message_id, pane);
    }
}
