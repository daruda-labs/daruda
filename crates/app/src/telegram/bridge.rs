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

/// Identifies one agent-chat pane across the whole process: a
/// workspace (window) uuid plus that workspace's locally-scoped pane
/// id. `PaneId` is only unique within a workspace, so routing an
/// inbound reply needs both halves. Mirrors `LaneRef { project, lane }`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct PaneRef {
    pub workspace: WorkspaceUuid,
    pub pane: u64,
}

/// What a phone button press means: the option the user picked, and which
/// way they picked it.
///
/// Narrower than `daruda_acp::PermissionDecision` on purpose, and not merely
/// because this file avoids depending on `daruda_acp`. That type is the
/// answer sent back to the agent, so it also carries `Cancelled` — nobody
/// decided, the turn died. A button press is always a decision, so giving
/// this enum a `Cancelled` variant would make an unreachable state
/// constructible here.
///
/// `Workspace::respond_bot_permission` maps this to an option id plus a
/// `PermissionKindView` and routes it through the pane, which is what builds
/// the ACP-side decision.
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
    /// A parsed command awaiting execution. Resolution of any ordinal happens
    /// in the executor, which owns `CommandState`.
    RunCommand {
        command: crate::control::spec::ControlCommand,
    },
    /// The text started with `/` but is not one of ours. Answered, never
    /// swallowed — a silently dropped message is the defect this replaces.
    ///
    /// Only for a name daruda *does* own, used wrongly (`/say` with no
    /// argument). A name daruda does not own is [`Self::UnknownSlash`] or
    /// [`Self::UnclaimedSlash`] — the agent's command namespace is open and
    /// this one is closed, so "not ours" cannot mean "nobody's".
    ReportParseError {
        error: crate::control::spec::ParseError,
    },
    /// A `/name` daruda does not own, aimed at `pane`.
    ///
    /// Whether it is the agent's own command or a typo of one of ours is a
    /// question only that pane can answer — it advertises its own list — and
    /// this layer is GPUI-free, so the decision waits for the one that can
    /// read it.
    UnknownSlash {
        pane: PaneRef,
        /// The name without its slash, to ask the agent about.
        name: String,
        /// The message as sent, forwarded verbatim when the agent owns it.
        text: String,
        /// The nearest daruda command, for the answer when it does not.
        suggestion: Option<&'static str>,
    },
    /// A `/name` daruda does not own, with nowhere to send it — the app had
    /// no lane to offer either.
    ///
    /// Not [`Self::ReportParseError`]: having no target says nothing about
    /// whose command the name is, and answering "did you mean /use?" to the
    /// agent's `/usage` is a claim this layer cannot support. Whose it was is
    /// still the agents' to settle; only a vocabulary that rules the name out
    /// earns the suggestion, and otherwise the true answer is the missing
    /// target.
    ///
    /// The message body is gone by here. [`Unaimed::Slash`] carried it as far
    /// as a target could have been found; past that there is no pane to put
    /// it on, and `aim` records its loss rather than letting an arm drop it
    /// quietly.
    UnclaimedSlash {
        /// The name without its slash, to ask the agents about.
        name: String,
        /// The nearest daruda command, for the answer when none of them owns it.
        suggestion: Option<&'static str>,
    },
    /// An approval card's button was tapped.
    ResolveApproval {
        id: crate::control::approval::ApprovalId,
        choice: crate::control::approval::ApprovalChoice,
    },
    /// A listing button was tapped — make that pane the current target.
    SelectTarget {
        pane: PaneRef,
    },
    /// A listing button from a superseded listing was tapped. Distinguished
    /// from an unknown token so the answer can say "send /list again" instead
    /// of the permission-prompt wording, and so the tapped message keeps its
    /// buttons — scrolling back to an old listing and tapping is ordinary use,
    /// not a decision to consume.
    StaleListing,
    /// Plain text with nowhere to send it: no reply-to, no selection, no
    /// prior ping, and no lane the app could offer either. Terminal — unlike
    /// [`Unaimed::Text`], which is the same message before that last question
    /// was asked.
    NoTarget,
    /// An update shape this bridge does not act on. Distinct from
    /// [`Self::Ignore`], which means "we could have acted but chose not to" —
    /// this one is not about the sender at all, so it must not be answered
    /// and must not be logged as an unauthorized inbound.
    Unsupported,
}

/// A routed update that still needs a target before anything can be done
/// with it.
///
/// The second axis of what `route` decides — *did we find somewhere to send
/// this* — split out rather than mirrored into [`InboundAction`]. `route` is
/// GPUI-free and cannot read a workspace, so it answers the first axis (is
/// this a command, a slash, plain text) and hands this one on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unaimed {
    /// Plain text: no reply-to, no selection, no prior ping.
    Text { text: String },
    /// A `/name` daruda does not own, in the same state.
    Slash {
        /// The name without its slash, to ask the agents about.
        name: String,
        /// The message as sent, forwarded verbatim if a target turns up.
        text: String,
        /// The nearest daruda command, for the answer when none is found.
        suggestion: Option<&'static str>,
    },
}

/// What `route` decided, before the target question is settled.
///
/// Two states, and only one of them can be acted on. The poll loop matches
/// [`InboundAction`], which it can only obtain by passing this through the
/// step that resolves a target — so skipping that step is a type error rather
/// than a comment nobody reads. That matters because the skipped case is
/// silent: the arms that would receive an unaimed action do nothing, so the
/// sender's message would vanish with no answer at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Routed {
    /// Nothing left to resolve.
    Ready(InboundAction),
    /// Needs the app's own active lane, or the terminal answer for not
    /// finding one.
    NeedsTarget(Unaimed),
}

impl Routed {
    /// The settled half, for the tests whose case needs no target.
    ///
    /// Panics on one that still does: needing a target is the thing under
    /// test wherever it happens, so those cases match on the variant instead
    /// of unwrapping it.
    #[cfg(test)]
    pub(crate) fn ready(self) -> InboundAction {
        match self {
            Self::Ready(action) => action,
            Self::NeedsTarget(unaimed) => {
                panic!("expected a settled action, got {unaimed:?}")
            }
        }
    }
}

/// `BridgeCore::route`'s full result: the action, plus whether
/// `answerCallbackQuery` must be called (only for `Callback` updates,
/// so the phone's button stops spinning — regardless of the action,
/// including `Ignore`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteResult {
    pub action: Routed,
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

/// What the outbound queue carries.
///
/// The two differ in one load-bearing way. A [`Self::Ping`] is attributed to a
/// pane, so the message id Telegram hands back registers a reply-to and makes
/// that pane `last_pinged`. A [`Self::Notice`] is attributed to nothing and
/// must stay that way — it answers a command the phone sent, and a reply to
/// "your flow failed" must not become a prompt for whichever agent happened to
/// speak last. Same reasoning that keeps a command reply off `record_sent`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outbound {
    Ping(BridgePing),
    /// A card asking the user to allow something an agent wants to create.
    /// Belongs to no pane, so it is not a [`BridgePing`]: nothing here should
    /// become the next plain message's destination.
    Approval(ApprovalPrompt),
    /// Already-localized, already-plain text. Never markdown-parsed: it is
    /// composed from daruda's own strings and can carry a flow file name whose
    /// punctuation a markdown pass would misread.
    Notice(String),
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

/// A pending approval, ready to send. The two tokens are already registered
/// with [`BridgeCore`], so a tap on either resolves without a second lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalPrompt {
    /// One line naming what is being allowed.
    pub summary: String,
    /// `(label, callback token)` — allow first, so the thumb order matches
    /// every other card daruda sends. Exactly two, in the type: a card with
    /// one button or five is not a thing this asks.
    pub buttons: [(String, String); 2],
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

    #[test]
    fn commands_are_ignored_before_pairing() {
        let mut core = BridgeCore::new(true, None, 0);
        let action = core.route(message(1, 999, "/list", None)).action.ready();
        assert_eq!(
            action,
            InboundAction::Ignore,
            "unauthorized chats must not enumerate"
        );
    }

    #[test]
    fn an_authorized_command_routes_to_run_command() {
        let mut core = BridgeCore::new(true, Some(42), 0);
        let action = core.route(message(1, 42, "/list", None)).action.ready();
        assert_eq!(
            action,
            InboundAction::RunCommand {
                command: crate::control::spec::ControlCommand::List
            }
        );
    }

    /// A typo is never swallowed. The answer is settled a layer up now — no
    /// agent advertises `lst`, so the suggestion is daruda's to give — but the
    /// suggestion has to reach that layer intact for it to be given at all.
    #[test]
    fn a_typo_carries_its_suggestion_instead_of_vanishing() {
        let mut core = BridgeCore::new(true, Some(42), 0);
        let action = core.route(message(1, 42, "/lst", None)).action;
        assert_eq!(
            action,
            Routed::NeedsTarget(Unaimed::Slash {
                name: "lst".into(),
                text: "/lst".into(),
                suggestion: Some("list"),
            })
        );
    }

    /// The regression this fixes. Claude owns `/usage`, daruda owns `/use`,
    /// and they share the `/` namespace — so daruda answered a command it does
    /// not have with a suggestion for one the user did not want, and the
    /// agent never saw it. Every agent slash command was unreachable from the
    /// phone this way; `/usage` is the one that has a near-miss to make it
    /// look deliberate.
    #[test]
    fn a_slash_we_do_not_own_is_held_for_the_pane_to_claim() {
        let mut core = BridgeCore::new(true, Some(42), 0);
        let target = pane(1, 10);
        core.command_state_mut().select(Some(target));
        let action = core.route(message(1, 42, "/usage", None)).action.ready();
        assert_eq!(
            action,
            InboundAction::UnknownSlash {
                pane: target,
                name: "usage".into(),
                text: "/usage".into(),
                suggestion: Some("use"),
            },
            "an agent command must reach the agent, not a typo answer"
        );
    }

    /// The same slash with nothing to aim it at. daruda still cannot say the
    /// name is a typo of one of its own — it never could — so the question is
    /// handed on rather than answered with a suggestion for a command the
    /// sender did not want.
    #[test]
    fn an_unowned_slash_with_no_target_is_still_not_ours_to_refuse() {
        let mut core = BridgeCore::new(true, Some(42), 0);
        let action = core.route(message(1, 42, "/usage", None)).action;
        assert_eq!(
            action,
            Routed::NeedsTarget(Unaimed::Slash {
                name: "usage".into(),
                text: "/usage".into(),
                suggestion: Some("use"),
            }),
            "with no target the reason is the missing target, not a typo"
        );
    }

    /// A command we *do* own, used wrongly, stays ours — the agent has no
    /// `/say` and forwarding it would answer a daruda mistake with an agent's
    /// confusion.
    #[test]
    fn our_own_command_used_wrongly_is_still_ours_to_answer() {
        let mut core = BridgeCore::new(true, Some(42), 0);
        core.command_state_mut().select(Some(pane(1, 10)));
        let action = core.route(message(1, 42, "/say", None)).action.ready();
        assert_eq!(
            action,
            InboundAction::ReportParseError {
                error: crate::control::spec::ParseError::MissingArgument { command: "say" }
            }
        );
    }

    #[test]
    fn plain_text_with_no_target_reports_instead_of_vanishing() {
        let mut core = BridgeCore::new(true, Some(42), 0);
        let action = core.route(message(1, 42, "hello", None)).action;
        assert_eq!(
            action,
            Routed::NeedsTarget(Unaimed::Text {
                text: "hello".into()
            })
        );
    }

    #[test]
    fn a_selected_target_takes_plain_text_without_a_reply_to() {
        let mut core = BridgeCore::new(true, Some(42), 0);
        let target = pane(1, 10);
        core.command_state_mut().select(Some(target));
        let action = core
            .route(message(1, 42, "add tests too", None))
            .action
            .ready();
        assert_eq!(
            action,
            InboundAction::InjectPrompt {
                pane: target,
                text: "add tests too".to_string(),
            }
        );
    }

    #[test]
    fn tapping_a_list_button_selects_that_target() {
        use crate::control::spec::Ordinal;

        let mut core = BridgeCore::new(true, Some(42), 0);
        let a = pane(1, 10);
        core.command_state_mut()
            .record_listing(&crate::telegram::command::tests::listing_of(&[a]));
        let token = core
            .command_state_mut()
            .listing_token(Ordinal(1))
            .expect("token");
        let action = core.route(callback(1, 42, "cb", &token)).action.ready();
        assert_eq!(action, InboundAction::SelectTarget { pane: a });
    }

    /// The invariant the parser now upholds and `route` depends on: the offset
    /// moves past an update the bridge cannot act on, so it is not re-delivered
    /// forever.
    #[test]
    fn an_unsupported_update_still_advances_the_offset() {
        let mut core = BridgeCore::new(true, Some(42), 0);
        let result = core.route(Update {
            update_id: 9,
            kind: UpdateKind::Unsupported,
        });
        assert_eq!(result.action.ready(), InboundAction::Unsupported);
        assert_eq!(result.answer_callback_id, None, "nothing to acknowledge");
        assert_eq!(core.current_offset(), 10);
    }

    /// Even while disabled: the poll loop does not run then, but a disable
    /// that lands mid-batch must not strand the offset either.
    #[test]
    fn an_unsupported_update_advances_the_offset_while_disabled() {
        let mut core = BridgeCore::new(false, Some(42), 0);
        core.route(Update {
            update_id: 3,
            kind: UpdateKind::Unsupported,
        });
        assert_eq!(core.current_offset(), 4);
    }

    #[test]
    fn a_seeded_offset_skips_already_processed_updates() {
        let mut core = BridgeCore::new(true, Some(42), 100);
        assert_eq!(core.current_offset(), 100);
        core.route(message(99, 42, "/list", None));
        assert_eq!(
            core.current_offset(),
            100,
            "an old update must not rewind the offset"
        );
    }

    #[test]
    fn a_fresh_core_starts_at_zero() {
        let core = BridgeCore::new(true, None, 0);
        assert_eq!(core.current_offset(), 0);
    }

    /// A permission prompt is always one horizontal row of options, so the
    /// assertions below read that row directly — and fail loudly if the
    /// prompt ever grows a second one.
    fn only_row(keyboard: &InlineKeyboard) -> &[(String, String)] {
        assert_eq!(keyboard.rows.len(), 1, "a permission prompt is one row");
        &keyboard.rows[0]
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
        let mut bridge = BridgeCore::new(true, None, 0);
        let result = bridge.route(message(1, 999, "hello", None));
        assert_eq!(result.action.ready(), InboundAction::Ignore);
        assert_eq!(result.answer_callback_id, None);
    }

    #[test]
    fn pair_code_exact_match_pairs_and_authorizes_future_messages() {
        let mut bridge = BridgeCore::new(true, None, 0);
        let code = bridge.new_pair_code();

        let result = bridge.route(message(1, 555, &format!("/pair {code}"), None));
        assert_eq!(
            result.action.ready(),
            InboundAction::Paired { chat_id: 555 }
        );
        assert_eq!(bridge.authorized_chat_id, Some(555));

        // A follow-up message from the now-authorized chat routes as
        // authorized (falls through to Ignore here only because there's
        // no reply-to / last-pinged pane yet — proves the auth gate
        // passed).
        bridge.last_pinged = Some(pane(1, 1));
        let result = bridge.route(message(2, 555, "ping back", None));
        assert_eq!(
            result.action.ready(),
            InboundAction::InjectPrompt {
                pane: pane(1, 1),
                text: "ping back".to_string(),
            }
        );
    }

    #[test]
    fn pair_code_match_is_case_insensitive() {
        let mut bridge = BridgeCore::new(true, None, 0);
        bridge.pending_pair_code = Some(fresh_pending_code("AB12CD"));

        let result = bridge.route(message(1, 42, "/pair ab12cd", None));
        assert_eq!(result.action.ready(), InboundAction::Paired { chat_id: 42 });
    }

    #[test]
    fn pair_code_mismatch_is_ignored_and_stays_unauthorized() {
        let mut bridge = BridgeCore::new(true, None, 0);
        bridge.pending_pair_code = Some(fresh_pending_code("AB12CD"));

        let result = bridge.route(message(1, 42, "/pair WRONG1", None));
        assert_eq!(result.action.ready(), InboundAction::Ignore);
        assert_eq!(bridge.authorized_chat_id, None);
    }

    #[test]
    fn reply_to_found_in_sent_pings_wins_over_last_pinged() {
        let mut bridge = BridgeCore::new(true, Some(1), 0);
        let reply_target = pane(1, 10);
        let different_last = pane(1, 99);

        bridge.record_sent(41, reply_target);
        bridge.record_sent(50, different_last);
        assert_eq!(bridge.last_pinged, Some(different_last));

        let result = bridge.route(message(1, 1, "answer", Some(41)));
        assert_eq!(
            result.action.ready(),
            InboundAction::InjectPrompt {
                pane: reply_target,
                text: "answer".to_string(),
            }
        );
    }

    #[test]
    fn reply_to_missing_from_sent_pings_falls_back_to_last_pinged() {
        let mut bridge = BridgeCore::new(true, Some(1), 0);
        let fallback = pane(1, 7);
        bridge.record_sent(1, fallback);

        let result = bridge.route(message(2, 1, "answer", Some(999)));
        assert_eq!(
            result.action.ready(),
            InboundAction::InjectPrompt {
                pane: fallback,
                text: "answer".to_string(),
            }
        );
    }

    #[test]
    fn no_reply_to_and_no_last_pinged_reports_no_target() {
        let mut bridge = BridgeCore::new(true, Some(1), 0);
        let result = bridge.route(message(1, 1, "hello", None));
        // Not `Ignore`: an authorized message that reaches nothing is answered,
        // because silence reads as the bot being broken.
        assert_eq!(
            result.action,
            Routed::NeedsTarget(Unaimed::Text {
                text: "hello".into()
            })
        );
    }

    /// The card stays on screen while the tool call waits, so tapping the
    /// same button twice is ordinary use — the token must survive it. (The
    /// *decision* is deduplicated by the approval store, not here.)
    #[test]
    fn an_approval_token_survives_repeated_taps() {
        use crate::control::approval::{ApprovalChoice, ApprovalId};
        let mut bridge = BridgeCore::new(true, Some(42), 0);
        let (approve, refuse) = bridge.record_pending_approval(ApprovalId(1));
        assert_ne!(approve, refuse, "one token per button");

        for _ in 0..2 {
            assert_eq!(
                bridge
                    .route(callback(1, 42, "cbq-1", &approve))
                    .action
                    .ready(),
                InboundAction::ResolveApproval {
                    id: ApprovalId(1),
                    choice: ApprovalChoice::Approved,
                }
            );
        }
        assert_eq!(
            bridge
                .route(callback(2, 42, "cbq-2", &refuse))
                .action
                .ready(),
            InboundAction::ResolveApproval {
                id: ApprovalId(1),
                choice: ApprovalChoice::Refused,
            }
        );
    }

    #[test]
    fn an_approval_tap_from_an_unauthorized_chat_is_ignored() {
        use crate::control::approval::ApprovalId;
        let mut bridge = BridgeCore::new(true, Some(42), 0);
        let (approve, _refuse) = bridge.record_pending_approval(ApprovalId(1));
        assert_eq!(
            bridge
                .route(callback(1, 99, "cbq-1", &approve))
                .action
                .ready(),
            InboundAction::Ignore
        );
    }

    /// A card that outlived the process leaves a button in the chat history;
    /// its token must not resolve against a request this run happens to
    /// number the same.
    #[test]
    fn an_approval_token_from_another_process_does_not_resolve() {
        use crate::control::approval::ApprovalId;
        let mut before = BridgeCore::new(true, Some(42), 0);
        let (stale, _) = before.record_pending_approval(ApprovalId(1));

        let mut after = BridgeCore::new(true, Some(42), 0);
        let _ = after.record_pending_approval(ApprovalId(1));
        assert_eq!(
            after.route(callback(1, 42, "cbq-1", &stale)).action.ready(),
            InboundAction::Ignore
        );
    }

    /// Three token namespaces share one callback channel, so a token from one
    /// must never be read as another's.
    #[test]
    fn the_three_token_namespaces_do_not_collide() {
        use crate::control::approval::ApprovalId;
        let mut bridge = BridgeCore::new(true, Some(42), 0);
        let target = pane(1, 3);
        bridge.pending_permissions.insert(
            "abcdef0123456789".to_string(),
            (target, 55, PermissionDecision::Allow("opt_yes".to_string())),
        );
        let (approve, _) = bridge.record_pending_approval(ApprovalId(1));

        assert!(approve.starts_with(APPROVAL_TOKEN_PREFIX));
        assert!(!approve.starts_with(crate::telegram::command::LISTING_TOKEN_PREFIX));
        // A permission token is a bare `Uuid::simple`, so it carries neither
        // prefix and still reaches the permission table.
        assert!(matches!(
            bridge
                .route(callback(1, 42, "cbq-1", "abcdef0123456789"))
                .action
                .ready(),
            InboundAction::RespondPermission { .. }
        ));
    }

    #[test]
    fn callback_with_known_token_responds_and_consumes_it() {
        let mut bridge = BridgeCore::new(true, Some(1), 0);
        let target = pane(1, 3);
        bridge.pending_permissions.insert(
            "tok-a".to_string(),
            (target, 55, PermissionDecision::Allow("opt_yes".to_string())),
        );

        let result = bridge.route(callback(1, 1, "cbq-1", "tok-a"));
        assert_eq!(
            result.action.ready(),
            InboundAction::RespondPermission {
                pane: target,
                perm_id: 55,
                decision: PermissionDecision::Allow("opt_yes".to_string()),
            }
        );
        assert_eq!(result.answer_callback_id, Some("cbq-1".to_string()));

        // Second tap on the same (now-consumed) token is a no-op.
        let result = bridge.route(callback(2, 1, "cbq-2", "tok-a"));
        assert_eq!(result.action.ready(), InboundAction::Ignore);
        assert_eq!(result.answer_callback_id, Some("cbq-2".to_string()));
    }

    #[test]
    fn callback_with_unknown_token_is_ignored_but_still_acked() {
        let mut bridge = BridgeCore::new(true, Some(1), 0);
        let result = bridge.route(callback(1, 1, "cbq-x", "never-registered"));
        assert_eq!(result.action.ready(), InboundAction::Ignore);
        assert_eq!(result.answer_callback_id, Some("cbq-x".to_string()));
    }

    #[test]
    fn callback_from_unauthorized_chat_is_ignored_but_still_acked() {
        let mut bridge = BridgeCore::new(true, Some(1), 0);
        let result = bridge.route(callback(1, 2, "cbq-y", "irrelevant"));
        assert_eq!(result.action.ready(), InboundAction::Ignore);
        assert_eq!(result.answer_callback_id, Some("cbq-y".to_string()));
    }

    #[test]
    fn disabled_bridge_ignores_everything_but_still_acks_callbacks() {
        let mut bridge = BridgeCore::new(false, Some(1), 0);

        let msg_result = bridge.route(message(1, 1, "hello", None));
        assert_eq!(msg_result.action.ready(), InboundAction::Ignore);
        assert_eq!(msg_result.answer_callback_id, None);

        let cb_result = bridge.route(callback(2, 1, "cbq-z", "tok"));
        assert_eq!(cb_result.action.ready(), InboundAction::Ignore);
        assert_eq!(cb_result.answer_callback_id, Some("cbq-z".to_string()));
    }

    #[test]
    fn offset_advances_to_max_update_id_plus_one_across_ignored_updates() {
        let mut bridge = BridgeCore::new(true, None, 0);
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
        let mut bridge = BridgeCore::new(true, Some(1), 0);
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
            result.action.ready(),
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
        let mut bridge = BridgeCore::new(true, Some(1), 0);
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
        let mut bridge = BridgeCore::new(true, Some(1), 0);
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
        assert_eq!(only_row(&keyboard).len(), 2);
        assert_eq!(only_row(&keyboard)[0].0, "Allow");
        assert_eq!(only_row(&keyboard)[1].0, "Reject");
        assert_ne!(only_row(&keyboard)[0].1, only_row(&keyboard)[1].1);

        assert_eq!(bridge.pending_permissions.len(), 2);
        let allow_token = &only_row(&keyboard)[0].1;
        let reject_token = &only_row(&keyboard)[1].1;
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
        let mut bridge = BridgeCore::new(true, Some(1), 0);
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
        assert_eq!(only_row(&keyboard).len(), 4);
        let labels: Vec<&str> = only_row(&keyboard)
            .iter()
            .map(|(l, _)| l.as_str())
            .collect();
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
            only_row(&keyboard).iter().map(|(_, t)| t).collect();
        assert_eq!(tokens.len(), 4, "every button gets its own distinct token");
        assert_eq!(bridge.pending_permissions.len(), 4);

        let execpolicy_token = &only_row(&keyboard)[2].1;
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
        let mut bridge = BridgeCore::new(true, Some(1), 0);
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
        let first_allow_token = only_row(&first_keyboard)[0].1.clone();
        let first_reject_token = only_row(&first_keyboard)[1].1.clone();

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
        assert_eq!(result.action.ready(), InboundAction::Ignore);
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
        let mut bridge = BridgeCore::new(true, None, 0);
        let code = bridge.new_pair_code();

        let result = bridge.route(message(1, 42, "/pair WRONG1", None));
        assert_eq!(result.action.ready(), InboundAction::Ignore);
        let result = bridge.route(message(2, 42, "/pair WRONG2", None));
        assert_eq!(result.action.ready(), InboundAction::Ignore);

        let result = bridge.route(message(3, 42, &format!("/pair {code}"), None));
        assert_eq!(result.action.ready(), InboundAction::Paired { chat_id: 42 });
        assert_eq!(bridge.authorized_chat_id, Some(42));
    }

    #[test]
    fn pair_code_exhausted_by_max_attempts_rejects_even_correct_guess() {
        let mut bridge = BridgeCore::new(true, None, 0);
        let code = bridge.new_pair_code();

        for i in 0..PAIR_CODE_MAX_ATTEMPTS {
            let result = bridge.route(message(i as i64, 42, "/pair WRONGCODE", None));
            assert_eq!(result.action.ready(), InboundAction::Ignore);
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
        assert_eq!(result.action.ready(), InboundAction::Ignore);
        assert_eq!(bridge.authorized_chat_id, None);
        assert!(bridge.pending_pair_code.is_none());
    }
}
