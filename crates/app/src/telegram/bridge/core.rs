//! Bridge routing core — a pure, GPUI-free state machine that turns parsed
//! `Update`s into routing decisions and shapes outbound pings into
//! `OutboundMsg`s. All bridge policy (auth gate, pairing, reply-to
//! resolution, permission-token routing, offset bookkeeping) lives here and
//! is unit-tested with plain function calls.
//!
//! The vocabulary it decides in lives in [`super`]; this file is the rules.

use std::collections::{HashMap, VecDeque};

use uuid::Uuid;

use super::{
    BridgePing, CallbackEdit, InboundAction, OutboundMsg, PaneRef, PermissionDecision, RouteResult,
    Routed, Unaimed,
};
use crate::telegram::client::{InlineKeyboard, Update, UpdateKind};

#[cfg(test)]
mod tests;

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

/// Callback-data prefix for an approval button. Distinct from the listing
/// prefix and from a permission token (bare `Uuid::simple`, no `:`), so the
/// three namespaces cannot collide structurally.
const APPROVAL_TOKEN_PREFIX: &str = "apv";

/// Approval tokens kept before the oldest is evicted. Two per card, and a
/// card the user never answers times out on the app side — so this only
/// bounds a pathological run of unanswered ones.
const PENDING_APPROVALS_CAP: usize = 64;

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
    last_pinged: Option<PaneRef>,
    sent_pings: HashMap<i64, PaneRef>,
    sent_pings_order: VecDeque<i64>,
    pending_permissions: HashMap<String, (PaneRef, u64, PermissionDecision)>,
    pending_permissions_order: VecDeque<String>,
    /// Approval tokens. Deliberately *not* consumed on a tap, unlike
    /// `pending_permissions`: the card stays on screen, and tapping the same
    /// button twice is ordinary use rather than a second decision. The
    /// approval store discards the redundant answer.
    pending_approvals: HashMap<
        String,
        (
            crate::control::approval::ApprovalId,
            crate::control::approval::ApprovalChoice,
        ),
    >,
    pending_approvals_order: VecDeque<String>,
    command_state: crate::telegram::command::CommandState,
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
            last_pinged: None,
            sent_pings: HashMap::new(),
            sent_pings_order: VecDeque::new(),
            pending_permissions: HashMap::new(),
            pending_permissions_order: VecDeque::new(),
            pending_approvals: HashMap::new(),
            pending_approvals_order: VecDeque::new(),
            command_state: crate::telegram::command::CommandState::default(),
        }
    }

    /// The ordinal table and selected target for the authorized chat. The poll
    /// loop reaches through this to resolve an ordinal against the listing the
    /// user actually saw, and to record the next one.
    pub fn command_state_mut(&mut self) -> &mut crate::telegram::command::CommandState {
        &mut self.command_state
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

        // Parsing sits behind the gate on purpose: a listing names projects,
        // lanes, and session titles, so an unauthorized chat must not reach it.
        // A name we do not own is held back rather than answered: the agent's
        // slash namespace is open and ours is closed, so only the pane can say
        // whether `/usage` is its command or a typo of `/use`.
        let unknown = match crate::control::spec::parse(&text) {
            Ok(command) => return Routed::Ready(InboundAction::RunCommand { command }),
            Err(crate::control::spec::ParseError::NotACommand) => None,
            Err(crate::control::spec::ParseError::Unknown { input, suggestion }) => {
                Some((input, suggestion))
            }
            // A command we *do* own, used wrongly. Ours to answer.
            Err(error) => return Routed::Ready(InboundAction::ReportParseError { error }),
        };

        let reply_to = reply_to_message_id
            .and_then(|id| self.sent_pings.get(&id))
            .copied();

        match self
            .command_state
            .plain_text_target(reply_to, self.last_pinged)
        {
            Some(pane) => Routed::Ready(match unknown {
                Some((name, suggestion)) => InboundAction::UnknownSlash {
                    pane,
                    name,
                    text,
                    suggestion,
                },
                None => InboundAction::InjectPrompt { pane, text },
            }),
            // Nothing here names a target, and this layer cannot ask the app
            // for one. Handed on rather than answered — both the target and,
            // for a slash, whose command it is are still open questions.
            None => Routed::NeedsTarget(match unknown {
                Some((name, suggestion)) => Unaimed::Slash {
                    name,
                    text,
                    suggestion,
                },
                None => Unaimed::Text { text },
            }),
        }
    }

    fn route_callback(&mut self, chat_id: i64, data: String) -> InboundAction {
        if self.authorized_chat_id != Some(chat_id) {
            return InboundAction::Ignore;
        }

        // Listing tokens are tried first and are never consumed — tapping the
        // same row twice is ordinary use. Permission tokens below are.
        if data.starts_with(crate::telegram::command::LISTING_TOKEN_PREFIX) {
            return match self.command_state.resolve_token(&data) {
                Some(pane) => InboundAction::SelectTarget { pane },
                None => InboundAction::StaleListing,
            };
        }

        if data.starts_with(APPROVAL_TOKEN_PREFIX) {
            return match self.pending_approvals.get(&data) {
                Some((id, choice)) => InboundAction::ResolveApproval {
                    id: *id,
                    choice: *choice,
                },
                None => InboundAction::Ignore,
            };
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

    /// Mint the two callback tokens one approval card needs and remember what
    /// each means. Returns them in `(approve, refuse)` order.
    pub fn record_pending_approval(
        &mut self,
        id: crate::control::approval::ApprovalId,
    ) -> (String, String) {
        use crate::control::approval::ApprovalChoice;
        let approve = self.insert_pending_approval(id, ApprovalChoice::Approved);
        let refuse = self.insert_pending_approval(id, ApprovalChoice::Refused);
        (approve, refuse)
    }

    #[cfg(test)]
    pub fn pending_approval_token_count(&self) -> usize {
        self.pending_approvals.len()
    }

    /// Drop both of `id`'s tokens. Called when the request settles, so the
    /// bounded table holds only cards that can still be answered.
    pub fn forget_pending_approval(&mut self, id: crate::control::approval::ApprovalId) {
        self.pending_approvals.retain(|_, (held, _)| *held != id);
        self.pending_approvals_order
            .retain(|token| self.pending_approvals.contains_key(token));
    }

    fn insert_pending_approval(
        &mut self,
        id: crate::control::approval::ApprovalId,
        choice: crate::control::approval::ApprovalChoice,
    ) -> String {
        // Random rather than derived from `id`: a card survives a restart in
        // the user's chat history while this table does not, and a derived
        // token would let a stale button resolve a *new* request that happens
        // to reuse the number.
        let token = format!(
            "{APPROVAL_TOKEN_PREFIX}:{}",
            uuid::Uuid::new_v4().as_simple()
        );
        self.pending_approvals.insert(token.clone(), (id, choice));
        self.pending_approvals_order.push_back(token.clone());
        if self.pending_approvals_order.len() > PENDING_APPROVALS_CAP
            && let Some(oldest) = self.pending_approvals_order.pop_front()
        {
            self.pending_approvals.remove(&oldest);
        }
        token
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

            InlineKeyboard::single_row(buttons)
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
