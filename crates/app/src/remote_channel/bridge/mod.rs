//! Shared remote-control messages and routing decisions.

use daruda_store::project::WorkspaceUuid;

pub(crate) mod core;
mod keyboard;
pub use core::RoutingCore;
pub use keyboard::InlineKeyboard;

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

/// A route's full result: the action, plus whether the tapped button must
/// be acknowledged (only for a callback update, so the phone's button stops
/// spinning — regardless of the action, including `Ignore`).
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

/// The event-specific trailing content of a ping, kept separate from the
/// always-plain `header`. Only some pings carry genuine agent-authored
/// markdown safe to run through a transport's markdown translation; plain
/// administrative text (a label, a tool title, a `raw_input` summary) must
/// never be markdown-parsed, or incidental punctuation in a path or command
/// (`file_name.txt`, `rm -rf *.log`) gets misread as emphasis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageTail {
    /// Plain text — escaped for the transport, never markdown-parsed.
    Plain(String),
    /// Genuine markdown (the agent's own response text) — translated into
    /// whatever subset the transport accepts.
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
/// it only cares which `MessageTail` variant `tail` is. `permission`
/// is `Some` only for a permission-wait ping (one button per option),
/// `None` for a plain completion ping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgePing {
    pub pane: PaneRef,
    pub header: String,
    pub tail: MessageTail,
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

/// Unaddressed content prepared by the shared routing core.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedPing {
    pub header: String,
    pub tail: MessageTail,
    pub keyboard: Option<InlineKeyboard>,
}
