//! Chat render model — the GPUI-free shape the `app` view layer renders.
//!
//! ACP protocol traffic (`session/update` notifications and
//! `session/request_permission` requests) is folded into a flat list of
//! [`ChatItem`]s by [`crate::mapping`]. The view reads this list only; it never
//! touches protocol types.

use std::path::PathBuf;

use crate::failure::AcpFailure;

mod compaction;
mod config_option;
mod notice;
pub use compaction::{CompactionItem, CompactionStatusView};
pub use config_option::{
    ConfigChoiceView, ConfigOptionCategoryView, ConfigOptionKindView, ConfigOptionView,
    ConfigValueView, DEFAULT_MODEL_CHOICE, resolved_default_model,
};
pub use notice::{NoticeSeverityView, SessionNoticeView};

/// One renderable item in the agent conversation, in arrival order.
#[derive(Debug, Clone, PartialEq)]
pub enum ChatItem {
    /// A user prompt (echoed locally on send, or streamed back by the agent).
    UserText(String),
    /// Assistant response text. `streaming` is true while chunks still arrive.
    ///
    /// `message_id` is the ACP message this text belongs to (when the agent
    /// supplies one): consecutive chunks sharing it accrue into this one item,
    /// and a change in `message_id` starts a new item — so distinct agent
    /// messages within a turn stay separate even with no tool call between
    /// them. `None` when the agent omits it; then chunks merge by adjacency
    /// (legacy behaviour). The renderer uses the last `AssistantText` of a
    /// response run as the turn's "conclusion".
    AssistantText {
        text: String,
        streaming: bool,
        message_id: Option<String>,
        /// What role this message plays in its turn, from the agent's own
        /// label (see [`crate::adapter::AcpAdapter::message_phase`]). Fixed
        /// when the message starts; a later chunk of the same message cannot
        /// restate it. A field rather than a `ChatItem` variant of its own:
        /// commentary is assistant text, so it renders, filters, and merges
        /// through exactly the same paths.
        phase: crate::adapter::MessagePhase,
    },
    /// Agent internal reasoning. `streaming` / `message_id` mirror
    /// [`ChatItem::AssistantText`].
    Thinking {
        text: String,
        streaming: bool,
        message_id: Option<String>,
    },
    /// A tool invocation and its evolving status/output.
    ToolCall(ToolCallItem),
    /// Context compaction anchored at the first update or summary chunk.
    Compaction(Box<CompactionItem>),
    /// A pending or resolved tool-permission request.
    Permission(PermissionItem),
    /// A surfaced failure (connection, protocol, or runtime), carrying its
    /// classification so the renderer can offer the matching remedy rather
    /// than only printing text. See [`crate::failure`].
    Failure(AcpFailure),
    /// The user stopped the turn here. Structure, not conversation: it marks
    /// where a run was cut, so it carries no text of its own — the host owns
    /// the wording and localizes it.
    ///
    /// Two sources produce it and must agree. Live, the host pushes it on Stop;
    /// the adapter cannot, because its `session/update` loop drops the SDK's
    /// interrupt entry twice over — a `cancelled` session short-circuits the
    /// `user`/`assistant` arm, and a single-text-block `user` message is
    /// skipped as feed noise. On `session/load` neither guard exists, so the
    /// same entry arrives as a `UserMessageChunk` and [`crate::mapping`] folds
    /// it back into this variant rather than a `UserText` bubble the user never
    /// typed. (Verified in `@agentclientprotocol/claude-agent-acp`'s
    /// `dist/acp-agent.js` — the guards at the `session.cancelled` break and the
    /// "messages we don't want in the feed" skip, against `replaySessionHistory`
    /// running only `stripLocalCommandMetadata`. Re-check on an adapter bump.)
    ///
    /// One divergence is unreachable from here: a prompt whose text *is* the
    /// marker stays a user bubble live (the local echo bypasses the mapper) and
    /// folds into this variant after restore. ACP carries no `isMeta` flag to
    /// tell the two apart.
    Interrupted,
}

/// A tool call, updated in place as `ToolCallUpdate`s arrive.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCallItem {
    /// Stable id used to match later updates to this call.
    pub id: String,
    pub title: String,
    pub kind: ToolKindView,
    /// The agent's own tool name (`Bash`, `Read`, `Grep`, …), read from the
    /// agent's vendor `_meta` via [`crate::adapter::AcpAdapter::tool_name`].
    /// More specific than `kind` and in the agent-CLI vocabulary the user
    /// knows, so the renderer prefers it as the header label. `None` when the
    /// agent surfaces no tool name — the renderer falls back to `kind`.
    pub tool_name: Option<String>,
    pub status: ToolStatusView,
    /// File modifications shown as diffs (rendered via daruda's diff editor).
    pub diffs: Vec<DiffView>,
    /// Typed output blocks produced by the tool (text, resource links).
    pub output: Vec<ToolOutputBlock>,
    /// Raw tool input, kept for an expandable "details" affordance.
    pub raw_input: Option<serde_json::Value>,
    /// The id of the parent tool call that spawned this one, when the adapter
    /// reports it (`_meta.claudeCode.parentToolUseId`). Set on a subagent's
    /// inner tool calls — the Claude adapter flattens subagent activity into the
    /// one session, so a child is linked to its parent `Task`/`Agent` call only
    /// through this field. `None` for a top-level call. The renderer nests
    /// children inside the parent card instead of listing them as siblings.
    pub parent_tool_id: Option<String>,
    /// Files this call touched, as the agent reported them. Mapped but not yet
    /// rendered: the title already names the file for most tools, and parsing it
    /// back out is what this replaces once a surface needs the path as data.
    pub locations: Vec<PathBuf>,
    /// Exit status of a shell-execution tool call, read from the agent's
    /// vendor side channel via [`crate::adapter::AcpAdapter::command_exit`].
    /// `None` when the adapter hasn't reported one (a non-shell tool, or a
    /// shell tool still running / whose adapter never surfaces it).
    pub exit: Option<CommandExit>,
}

/// Exit status of a shell-execution tool call. ACP has no standard field for
/// this — each adapter reports it (or doesn't) in its own side channel, so the
/// model only records what was actually reported; whether an absent/zero exit
/// is worth displaying is the renderer's call, not this type's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandExit {
    pub code: Option<i32>,
    pub signal: Option<String>,
}

impl ToolCallItem {
    /// True when this is a shell command launched detached (Claude Code's Bash
    /// `run_in_background: true`). The flag rides in the tool input, forwarded
    /// verbatim as `raw_input` by the adapter, so we read it there. Gated to
    /// [`ToolKindView::Execute`] since only shell tools carry it.
    pub fn is_background(&self) -> bool {
        self.kind == ToolKindView::Execute
            && self
                .raw_input
                .as_ref()
                .and_then(|v| v.get("run_in_background"))
                .and_then(serde_json::Value::as_bool)
                == Some(true)
    }

    /// The spawned subagent's type when this call is a subagent launch (Claude
    /// Code's `Task` tool carries `subagent_type` in its input). Read
    /// defensively from `raw_input` — `None` for a regular tool call or when the
    /// adapter omits the field — so the renderer can name the subagent on the
    /// parent card (e.g. "Subagent: code-reviewer") and fall back to the generic
    /// label when the type is unavailable.
    pub fn subagent_type(&self) -> Option<&str> {
        self.raw_input
            .as_ref()?
            .get("subagent_type")?
            .as_str()
            .filter(|s| !s.is_empty())
    }

    /// The full task spec handed to a spawned subagent (Claude Code's `Task`
    /// tool carries it as `prompt`) — the thing a user actually needs to audit
    /// "did the subagent do what I asked", as opposed to the one-line `title`.
    /// Unlike the tool call's `output`, which a completion event overwrites with
    /// the subagent's own result summary (see `fold_output`'s replace
    /// semantics), `raw_input` is never touched by a completion update, so this
    /// stays available for the lifetime of the card.
    pub fn subagent_prompt(&self) -> Option<&str> {
        self.raw_input
            .as_ref()?
            .get("prompt")?
            .as_str()
            .filter(|s| !s.is_empty())
    }

    /// Whether a subagent launch was dispatched to run detached
    /// (`run_in_background: true`). Distinct from [`Self::is_background`],
    /// which is scoped to shell (`Execute`) calls.
    pub fn subagent_run_in_background(&self) -> Option<bool> {
        self.raw_input.as_ref()?.get("run_in_background")?.as_bool()
    }

    /// Whether this call is shaped like a subagent launch at all — either
    /// input field is enough, since an adapter is not guaranteed to send both.
    /// The single source for "does this card get subagent-card treatment":
    /// its own fold key, its own display category, and the glyph both the card
    /// and its group bar key off. Keeping it one predicate is what stops those
    /// from drifting apart.
    pub fn is_subagent_launch(&self) -> bool {
        self.subagent_type().is_some() || self.subagent_prompt().is_some()
    }
}

/// A single block of tool-call output. Typed so non-text content (images,
/// audio, embedded blobs, resource links) survives rather than being
/// flattened to a base64 text blob.
#[derive(Debug, Clone, PartialEq)]
pub enum ToolOutputBlock {
    /// Markdown text — the ACP spec says clients SHOULD render tool text as
    /// Markdown, and adapters escape it accordingly. `truncated_from` is
    /// `Some(original_byte_len)` when the text was capped, `None` otherwise.
    Text {
        text: String,
        truncated_from: Option<usize>,
    },
    /// Shell output recovered from an adapter's terminal side channel (see
    /// [`crate::adapter::AcpAdapter::sideband_output`]). Unlike [`Self::Text`]
    /// this never passed through the adapter's markdown escaping, so it must
    /// render as flush monospace, verbatim — a `#` or `*` a command printed is
    /// literal, not a heading or emphasis. Same `truncated_from` contract.
    RawText {
        text: String,
        truncated_from: Option<usize>,
    },
    /// One file's verbatim contents, as read by a file-reading tool, plus the
    /// source language its path implies. [`Self::Text`]'s markdown contract does
    /// not apply: [`crate::output_highlight`] has already undone the adapter's
    /// escaping fence and the tool's `cat -n` line-number gutter, so `text` is
    /// what the file holds and renders through the host's code editor.
    ///
    /// `language` names a language rather than tagging a fence so the host never
    /// has to parse markdown back out to find it — which is what made an
    /// unfenced read lose both its highlighting and its bounded embed.
    /// `None` when the extension is absent or unrecognized. Same
    /// `truncated_from` contract.
    SourceText {
        text: String,
        language: Option<String>,
        truncated_from: Option<usize>,
    },
    /// A decodable image; `data` is base64 (decoded to a real image at the app
    /// render boundary). `mime` may be empty when the source omitted it.
    Image { data: String, mime: String },
    /// A non-rendered binary payload (audio, embedded blob). Carries only a
    /// descriptor; `byte_len` is the estimated decoded size for a label.
    Media { mime: String, byte_len: usize },
    /// A resource the tool produced or referenced. `mime` is optional protocol
    /// metadata that lets hosts offer richer rendering without guessing every
    /// arbitrary resource URI's content kind.
    ResourceLink {
        uri: String,
        name: String,
        mime: Option<String>,
    },
}

/// A file modification carried by a tool call.
#[derive(Debug, Clone, PartialEq)]
pub struct DiffView {
    pub path: PathBuf,
    /// `None` for a newly created file.
    pub old_text: Option<String>,
    pub new_text: String,
}

/// Execution status of a tool call (mirror of the protocol's `ToolCallStatus`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolStatusView {
    Pending,
    InProgress,
    Completed,
    Failed,
    /// The turn was stopped before this tool call settled. Set host-side by
    /// [`crate::cancel_pending_tools`] when the user cancels — agents never
    /// emit it, so it has no `ToolCallStatus` mapping.
    Cancelled,
}

impl ToolStatusView {
    /// True while a tool call is still underway — neither settled nor stopped.
    ///
    /// Groups `Pending` with `InProgress`: the adapter marks every tool call
    /// `Pending` at first sight and only promotes to `InProgress` when the SDK
    /// emits a `tool_progress` ping, which many tools never get. Since
    /// [`crate::cancel_pending_tools`] settles any leftover `Pending` /
    /// `InProgress` to `Cancelled` at turn end, a live `Pending` always means an
    /// in-flight tool in the active turn — so the view treats the two alike.
    ///
    /// The shared predicate for the boolean live-state sites (badge, fold
    /// `is_active`, running-tool title). The exhaustive tool-status rollups in
    /// the chat package (`agent_chat_helpers.rs`, `render/status_icon.rs`,
    /// `render/tool/labels.rs`) mirror this same `InProgress | Pending`
    /// grouping; they stay hand-written so adding a status variant is a
    /// compile error there, but must be kept in step with this grouping.
    pub fn is_live(self) -> bool {
        matches!(self, Self::Pending | Self::InProgress)
    }
}

/// Tool category (mirror of the protocol's `ToolKind`) — drives icon/treatment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolKindView {
    Read,
    Edit,
    Delete,
    Move,
    Search,
    Execute,
    Think,
    Fetch,
    SwitchMode,
    Other,
}

/// A tool-permission request rendered as an inline card.
#[derive(Debug, Clone, PartialEq)]
pub struct PermissionItem {
    /// The daruda-internal request id this card represents (the same id the
    /// connection parked under and the host resolves via
    /// `AcpSessionHandle::respond_permission`). Carried on the card so the host
    /// correlates a specific card to its park when several permissions are
    /// outstanding at once (parallel tool calls) — not just the trailing one.
    pub id: u64,
    /// The tool call this request is about — the key to its diffs and output
    /// in the transcript.
    pub tool_call_id: String,
    /// Title of the tool call needing approval, if the agent supplied one.
    pub tool_title: Option<String>,
    /// A short, safe one-line summary of the tool's raw input (e.g.
    /// `"file: /tmp/x.rs"`, `"command: npm install"`), when `raw_input`
    /// carries one of a small set of known-short fields. See
    /// `mapping::summarize_raw_input` for the exact field list and why
    /// bulk-content fields (a diff's old/new text, a write's full file
    /// content) are deliberately excluded rather than summarized.
    pub raw_input_summary: Option<String>,
    /// Choices to render as buttons, in the agent's order.
    pub options: Vec<PermissionChoice>,
    /// How the card was resolved, or `None` while still pending a decision.
    pub resolved: Option<PermissionResolution>,
}

/// How a pending permission card was resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionResolution {
    /// The user chose this option id.
    Chosen(String),
    /// The turn was cancelled before the user decided; the agent received a
    /// `Cancelled` outcome and the card's buttons are disabled.
    Cancelled,
}

/// One selectable permission choice.
#[derive(Debug, Clone, PartialEq)]
pub struct PermissionChoice {
    pub option_id: String,
    pub name: String,
    pub kind: PermissionKindView,
}

/// Hint about a permission choice (mirror of `PermissionOptionKind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionKindView {
    AllowOnce,
    AllowAlways,
    RejectOnce,
    RejectAlways,
}

/// A single entry of the agent's execution plan (mirror of the protocol's
/// `PlanEntry`). The agent replaces the whole plan on each update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanEntryView {
    pub content: String,
    /// Mapped from protocol but not yet consumed by the UI — kept for future
    /// priority-based sorting or visual differentiation.
    pub priority: PlanPriority,
    pub status: PlanStatus,
}

/// Execution status of a plan entry (mirror of the protocol's
/// `PlanEntryStatus`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanStatus {
    Pending,
    InProgress,
    Completed,
    /// The run ended before this step finished. Set host-side by
    /// [`crate::cancel_pending_plan_entries`] — agents never emit it, so it has
    /// no `PlanEntryStatus` mapping. Distinct from `Pending`, which the run
    /// never reached: this is the step it was on when it stopped.
    Cancelled,
}

/// Priority level of a plan entry (mirror of the protocol's
/// `PlanEntryPriority`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanPriority {
    High,
    Medium,
    Low,
}

impl From<&agent_client_protocol::schema::v1::PlanEntry> for PlanEntryView {
    fn from(e: &agent_client_protocol::schema::v1::PlanEntry) -> Self {
        use agent_client_protocol::schema::v1::{PlanEntryPriority, PlanEntryStatus};
        let priority = match &e.priority {
            PlanEntryPriority::High => PlanPriority::High,
            PlanEntryPriority::Medium => PlanPriority::Medium,
            PlanEntryPriority::Low => PlanPriority::Low,
            // `PlanEntryPriority` is `#[non_exhaustive]` — the wildcard is required
            // for forward-compat with future variants added upstream.
            #[allow(unreachable_patterns)]
            _ => PlanPriority::Medium,
        };
        let status = match &e.status {
            PlanEntryStatus::Pending => PlanStatus::Pending,
            PlanEntryStatus::InProgress => PlanStatus::InProgress,
            PlanEntryStatus::Completed => PlanStatus::Completed,
            // `PlanEntryStatus` is `#[non_exhaustive]` — the wildcard is required
            // for forward-compat with future variants added upstream.
            #[allow(unreachable_patterns)]
            _ => PlanStatus::Pending,
        };
        PlanEntryView {
            content: e.content.clone(),
            priority,
            status,
        }
    }
}

/// One advertised session mode (mirror of the protocol's `SessionMode`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionModeView {
    /// Stable identifier used to switch into this mode via `set_mode`.
    pub id: String,
    /// Human-readable label shown in the mode chip.
    pub name: String,
    /// Optional longer description, shown in a tooltip or mode picker.
    pub description: Option<String>,
}

/// Mode state advertised at session-connect time, updated by
/// `CurrentModeUpdate` notifications, and re-derived from the `Mode` config
/// option via [`ModeStateView::from_config_options`] (mirror of the protocol's
/// `SessionModeState`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModeStateView {
    /// The modes the agent supports.
    pub available: Vec<SessionModeView>,
    /// `id` of the mode the agent is currently in.
    pub current: String,
}

/// Which optional session methods the agent advertised at `initialize`
/// (`AgentCapabilities`). Each flag gates the matching host affordance
/// (resume, list, close, fork). All-false is the baseline agent that supports
/// only `session/new` + `session/prompt` + `session/cancel` + `session/update`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SessionCapabilitiesView {
    /// `session/load` — replay a prior session's history on connect.
    pub load: bool,
    /// `session/list` — enumerate resumable sessions.
    pub list: bool,
    /// `session/resume` — resume a session without replaying its history.
    pub resume: bool,
    /// `session/close` — explicitly end a session.
    pub close: bool,
    /// `promptCapabilities.image` — accept embedded image blocks.
    pub images: bool,
    /// `promptCapabilities.embeddedContext` — accept file content snapshots.
    pub embedded_context: bool,
    // `session/fork` is intentionally absent: it is gated behind the schema's
    // `unstable_session_fork` feature, which daruda does not enable, so the
    // protocol field is not compiled in and the capability is unreachable.
}

/// What one turn cost, from the `session/prompt` reply.
///
/// Distinct from [`UsageView`], which is the *session's* running context fill:
/// this is scoped to the turn that just ended, so a surface can say what a
/// single answer took. Every field past the first two is optional because the
/// agents disagree on which they report — one sends cache writes, another sends
/// reasoning tokens, and neither sends the other's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TurnUsageView {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub thought_tokens: Option<u64>,
    pub cached_read_tokens: Option<u64>,
    pub cached_write_tokens: Option<u64>,
}

impl From<&agent_client_protocol::schema::v1::Usage> for TurnUsageView {
    fn from(u: &agent_client_protocol::schema::v1::Usage) -> Self {
        Self {
            input_tokens: u.input_tokens,
            output_tokens: u.output_tokens,
            total_tokens: u.total_tokens,
            thought_tokens: u.thought_tokens,
            cached_read_tokens: u.cached_read_tokens,
            cached_write_tokens: u.cached_write_tokens,
        }
    }
}

/// Live token/context accounting from a `session/update` `UsageUpdate`
/// notification. `used`/`size` describe the **current context-window fill**
/// (distinct from the CLI's cumulative account usage shown in the Usage tab),
/// so the host can render a "context: used / size" meter.
#[derive(Debug, Clone, PartialEq)]
pub struct UsageView {
    /// Tokens currently held in the context window.
    pub used: u64,
    /// Total context window size in tokens.
    pub size: u64,
    /// Cumulative session cost, when the agent reports it.
    pub cost: Option<CostView>,
    /// The account's rate-limit standing, when the agent reports it. Only the
    /// session's own adapter can read it, so the protocol `From` leaves it unset.
    pub rate_limit: Option<RateLimitView>,
}

/// The account rate-limit report riding on a usage update (claude-agent-acp's
/// `_meta["_claude/rateLimit"]`, the SDK's `SDKRateLimitInfo`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RateLimitView {
    /// The limit window this report is about.
    pub window: Option<RateLimitWindow>,
    /// When `window` resets, in unix seconds.
    pub resets_at: Option<i64>,
    pub is_using_overage: bool,
}

impl RateLimitView {
    /// The reply is being billed as extra usage.
    pub fn bills_overage(&self) -> bool {
        self.is_using_overage
    }
}

/// Which limit a report is about. `Other` keeps a window this build does not
/// know verbatim: the set grows with the plans on offer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RateLimitWindow {
    FiveHour,
    SevenDay,
    Other(String),
}

impl RateLimitWindow {
    pub fn from_wire(window: &str) -> Self {
        match window {
            "five_hour" => Self::FiveHour,
            "seven_day" => Self::SevenDay,
            other => Self::Other(other.to_owned()),
        }
    }
}

/// Session cost carried by an [`UsageView`] (mirror of the protocol's `Cost`).
#[derive(Debug, Clone, PartialEq)]
pub struct CostView {
    pub amount: f64,
    pub currency: String,
}

impl UsageView {
    /// Fold this report over the previous one. `cost` is optional per update and
    /// an absent one means "not reported this time", not zero: claude-agent-acp
    /// prices only its `result` updates, so its mid-turn fills carry none.
    #[must_use]
    pub fn carrying_cost_from(mut self, previous: Option<&UsageView>) -> Self {
        if self.cost.is_none() {
            self.cost = previous.and_then(|p| p.cost.clone());
        }
        self
    }
}

impl From<&agent_client_protocol::schema::v1::UsageUpdate> for UsageView {
    fn from(u: &agent_client_protocol::schema::v1::UsageUpdate) -> Self {
        Self {
            used: u.used,
            size: u.size,
            cost: u.cost.as_ref().map(|c| CostView {
                amount: c.amount,
                currency: c.currency.clone(),
            }),
            rate_limit: None,
        }
    }
}

/// A slash command the agent advertises via `AvailableCommandsUpdate`
/// (mirror of the protocol's `AvailableCommand`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlashCommand {
    pub name: String,
    pub description: String,
    pub input: SlashCommandInput,
}

/// Whether a command takes an argument. Mirrors the protocol's
/// `Option<AvailableCommandInput>` as an explicit enum so the no-arg case is
/// not a `None` sentinel (makes the invalid state unrepresentable).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlashCommandInput {
    /// Command takes no argument — sending `/name` is complete.
    NoInput,
    /// Command takes free-text after the name; `hint` is shown as guidance.
    FreeText { hint: String },
}

impl From<&agent_client_protocol::schema::v1::AvailableCommand> for SlashCommand {
    fn from(c: &agent_client_protocol::schema::v1::AvailableCommand) -> Self {
        use agent_client_protocol::schema::v1::AvailableCommandInput;
        let input = match &c.input {
            None => SlashCommandInput::NoInput,
            Some(AvailableCommandInput::Unstructured(u)) => SlashCommandInput::FreeText {
                hint: u.hint.clone(),
            },
            // Forward-compat fallback: unknown future variants map to NoInput so
            // the host is not broken by a protocol extension it doesn't know about.
            #[allow(unreachable_patterns)]
            Some(_) => SlashCommandInput::NoInput,
        };
        SlashCommand {
            name: c.name.clone(),
            description: c.description.clone(),
            input,
        }
    }
}

impl From<&agent_client_protocol::schema::v1::SessionModeState> for ModeStateView {
    fn from(s: &agent_client_protocol::schema::v1::SessionModeState) -> Self {
        ModeStateView {
            available: s
                .available_modes
                .iter()
                .map(|m| SessionModeView {
                    id: m.id.to_string(),
                    name: m.name.clone(),
                    description: m.description.clone(),
                })
                .collect(),
            current: s.current_mode_id.to_string(),
        }
    }
}

impl ModeStateView {
    /// Re-derive the mode state from an advertised config-option set.
    ///
    /// `SessionModeState` is delivered once, in the `session/new` /
    /// `session/load` response, but an agent may rebuild its mode list
    /// mid-session — the Claude adapter drops `auto` when the newly selected
    /// model reports no classifier support — and announces the new list only
    /// through the `Mode`-category config option. Re-deriving on every config
    /// change keeps the host's advertised modes and current mode from going
    /// stale against the agent's real ones.
    ///
    /// `None` when the set carries no `Mode` option, or that option advertises
    /// no choices — the caller then keeps the mode state it already has. Unlike
    /// [`From<&SessionModeState>`], which builds the initial state and so has
    /// nothing to lose by accepting an empty list, this refresh overwrites live
    /// state: an empty selector is far likelier to be an adapter transient than
    /// a real "this agent has no modes now", so prefer the last known-good list.
    ///
    /// Crate-internal: [`crate::mode_tracker`] is the only caller, so the host
    /// sees one reconciled mode fact instead of a second mirror to fold in.
    pub(crate) fn from_config_options(options: &[ConfigOptionView]) -> Option<Self> {
        let mode = options
            .iter()
            .find(|o| o.category == ConfigOptionCategoryView::Mode)?;
        // A mode is a choice among named modes, so only the select kind can
        // describe one; a boolean `Mode` option is malformed and yields no
        // mode state rather than a bogus two-entry selector.
        let ConfigOptionKindView::Select {
            current_value,
            options: choices,
        } = &mode.kind
        else {
            return None;
        };
        if choices.is_empty() {
            return None;
        }
        Some(ModeStateView {
            available: choices
                .iter()
                .map(|c| SessionModeView {
                    id: c.value.clone(),
                    name: c.name.clone(),
                    description: c.description.clone(),
                })
                .collect(),
            current: current_value.clone(),
        })
    }
}

#[cfg(test)]
mod tests;
