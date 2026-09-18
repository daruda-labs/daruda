//! Self-owned ACP chat pane entity.
//!
//! Like `TerminalView`, this pane owns its conversation model plus UI/runtime
//! state, renders itself, and notifies itself. Workspace ops create the pane,
//! own the ACP pump/error pipeline, and pass config-derived inputs into
//! `apply_event` so the view does not mirror workspace config — with one
//! deliberate exception, `syntax_theme`: a fold expand has to materialize diff
//! embeds outside any ACP event, so the view records that one resolved value
//! (single update site, [`AgentChatView::set_syntax_theme`]).
//!
//! ## SAFETY(MVU): self-contained pane entity
//!
//! This entity owns and mutates its own model, like `TerminalView`; it is not a
//! Workspace-owned model written through a one-way op. The MVU rule applies to
//! Workspace state; self-notifying pane entities are the sanctioned CLAUDE.md
//! rule #10 exception also used by `TerminalView` and `ToastLayer`.

use std::collections::{HashMap, HashSet, VecDeque};

use daruda_acp::{
    AcpSessionHandle, ChatItem, ConnectPhase, PlanEntryView, SessionCapabilitiesView, UsageView,
};
use daruda_core::process_env;
use daruda_store::project::PaneCwd;
use gpui::{
    AnyWindowHandle, App, Bounds, Context, FocusHandle, Focusable, FollowMode, ListAlignment,
    ListState, Pixels, ScrollHandle, Subscription, Task, Window, prelude::*, px,
};

use super::fold::FoldState;
use super::pane_choice::PaneChoice;
use super::render::{
    DiffEditors, DiffStats, MermaidImages, OutputEditors, ResourceImages, ToolImages,
};
use super::rows::tail::TailWindow;
use super::rows::{FilterMatchIndex, LiveSubagentUnits, RenderRow};
use super::session_config::SessionConfig;
use super::telegram_ops::{FirstResponseOutcome, PhoneTurn};
use super::transcript_defaults::TranscriptDefaults;
use crate::transcript::display_filter::DisplayFilter;
use crate::transcript::editor::state::FoldEditorState;
use crate::workspace::main_area::pane_tree::PaneId;

/// How long a subagent's run stays "active" after its last child tool event
/// (~4s observed), bridging gaps between its sequential child calls — the
/// parent Task's own status completes early with no clean terminal signal.
const SUBAGENT_QUIESCENCE: std::time::Duration = std::time::Duration::from_secs(8);

/// Debug gate for list-measurement tracing, cached from `DARUDA_DEBUG_AGENT_LIST`.
/// Off by default; set the env var to capture the remeasure timeline when the
/// intermittent oversized-gap bug recurs.
fn debug_list_trace_enabled() -> bool {
    use std::sync::OnceLock;
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| process_env::DEBUG_AGENT_LIST.is_present())
}

/// A user-visible milestone of the one-time Node.js runtime provisioning shown
/// in the connecting banner — only the slow, user-facing phases; instant ones
/// (system-node found, cache probe) never surface one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::workspace) enum RuntimePrepPhase {
    /// Downloading the Node.js archive.
    Downloading,
    /// Verifying the downloaded archive's checksum.
    Verifying,
    /// Extracting the archive.
    Extracting,
}

/// Connection lifecycle of an [`AgentChatView`]'s ACP session. An enum (not a
/// `bool` + companion field) so connecting/live/failed are distinct variants;
/// `Error` carries the failure message it renders.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::workspace) enum AgentSessionStatus {
    /// Restored or freshly created but no session started yet — stays dormant
    /// until focused (`focus_pane` → `maybe_connect_agent_chat`).
    Idle,
    /// Provisioning Node.js before the adapter even spawns — only on a machine
    /// with no usable system install. Carries the milestone, not localized text.
    PreparingRuntime(RuntimePrepPhase),
    /// Adapter asked to start, session not yet ready for prompts, no milestone
    /// reported yet. Upgrades in-place to `Handshaking` on the first
    /// `AcpEvent::ConnectProgress`.
    Connecting,
    /// Refines `Connecting` with the in-flight handshake step, bounded by
    /// `CONNECT_HANDSHAKE_TIMEOUT` so it can never hang forever — a timeout
    /// surfaces as `Error` like any other connect failure.
    Handshaking(ConnectPhase),
    /// `initialize` + `session/new` succeeded — the session accepts prompts and
    /// the event pump is folding updates into `items`.
    Connected,
    /// The connection or protocol failed; the message is surfaced both here
    /// (status line) and through the error pipeline.
    ///
    /// `remedy` is what the banner may offer. It is decided here, in the
    /// reducer, rather than in `render` — the view only picks an affordance
    /// for a remedy it is handed. A locally-detected failure (no lane cwd, a
    /// blocked remote path) has nothing to offer and carries
    /// [`Remedy::NoneAvailable`].
    Error {
        message: String,
        remedy: daruda_acp::Remedy,
    },
}

/// Whether a `session/prompt` turn is currently in flight. `InFlight` carries
/// the wall-clock start instant (runtime-only; never persisted) so the enum
/// can't represent "in flight but no start time" or "idle with a start time".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum Turn {
    #[default]
    Idle,
    InFlight {
        started_at: std::time::Instant,
    },
    /// A Stop settled the turn locally and the agent still owes one `cancelled`
    /// `TurnEnded` for it. A prompt typed here buffers client-side rather than
    /// racing onto the wire, so a stale ack cannot be misattributed to the turn
    /// the user re-prompted.
    AwaitingCancelAck,
}

impl Turn {
    /// True while a prompt turn is on the wire (Send ↔ Stop affordance, badge).
    /// A cancelled turn is *not* in flight: Stop settles it immediately so a
    /// hung agent that never acks cannot leave the pane pulsing.
    fn is_in_flight(&self) -> bool {
        matches!(self, Turn::InFlight { .. })
    }

    /// Whether a prompt may go on the wire — nothing in flight *and* no ack
    /// owed. One question rather than two fields a call site has to remember to
    /// ask in full.
    fn can_dispatch(&self) -> bool {
        matches!(self, Turn::Idle)
    }

    /// True between a Stop and the terminal signal that closes its window.
    fn awaiting_cancel_ack(&self) -> bool {
        matches!(self, Turn::AwaitingCancelAck)
    }

    /// When the in-flight turn started, for the per-turn record. `None` for a
    /// turn that never ran on this clock — a restored one, or one a Stop cut.
    fn started_at(&self) -> Option<std::time::Instant> {
        match self {
            Turn::InFlight { started_at } => Some(*started_at),
            Turn::Idle | Turn::AwaitingCancelAck => None,
        }
    }

    /// A prompt went out. `now` is passed in so a test can pin the clock.
    fn start(&mut self, now: std::time::Instant) {
        *self = Turn::InFlight { started_at: now };
    }

    /// The turn ended on its own terms. Leaves an open cancel window alone: the
    /// ack it waits for has not landed, and `cancel_turn` settles *before* it
    /// arrives, so clearing here would reopen the race this state exists for.
    fn finish(&mut self) {
        if !self.awaiting_cancel_ack() {
            *self = Turn::Idle;
        }
    }

    /// A Stop cut the turn. Answers whether there was one to cut, which is what
    /// decides whether an ack is owed and a stop marker earned — an Escape that
    /// only cancels a trailing subagent cuts no turn.
    fn cancel(&mut self) -> bool {
        let was_in_flight = self.is_in_flight();
        if was_in_flight {
            *self = Turn::AwaitingCancelAck;
        }
        was_in_flight
    }

    /// The ack landed, or the session that owed it is gone. A no-op on a turn
    /// that is not waiting for one, so a fresh-session reset cannot settle a
    /// live turn behind the caller's back.
    fn acknowledge_cancel(&mut self) {
        if self.awaiting_cancel_ack() {
            *self = Turn::Idle;
        }
    }
}

/// Whether a `session/load` replay is still streaming. `Loading` carries the id
/// the load was *asked* for, which is what distinguishes a real resume from one
/// the agent silently downgraded to a fresh `session/new` — that answers with a
/// different id. Holding the id here rather than re-deriving the gate from
/// `session_id` also makes "replaying, but no session to replay" unrepresentable.
enum Replay {
    Live,
    Loading { requested: String },
}

impl Replay {
    /// The gate a connect opens: `Some(id)` is a `session/load` whose replay
    /// must be coalesced, `None` a fresh `session/new` with nothing to replay.
    fn for_connect(resume: Option<String>) -> Self {
        match resume {
            Some(requested) => Self::Loading { requested },
            None => Self::Live,
        }
    }

    /// True while replayed events must accumulate without a per-event rebuild.
    fn is_loading(&self) -> bool {
        matches!(self, Self::Loading { .. })
    }

    /// True when `session_id` is the id this replay asked to load — a real
    /// resume, as opposed to a load the agent downgraded to a fresh session.
    fn resumed(&self, session_id: &str) -> bool {
        matches!(self, Self::Loading { requested } if requested == session_id)
    }
}

/// The pane's busy span, as of the last `reconcile_activity` tick. `Busy`
/// carries the wall-clock start instant so the enum can't represent "busy with
/// no start time" or "idle with one" — the level and the anchor move together
/// on every edge, which is the only way they are ever written.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(in crate::workspace) enum ActivitySpan {
    #[default]
    Idle,
    Busy {
        started_at: std::time::Instant,
    },
}

impl ActivitySpan {
    /// The edge-detection memory: whether the pane was busy at the last tick.
    pub(in crate::workspace) fn is_busy(self) -> bool {
        matches!(self, Self::Busy { .. })
    }

    /// How long the current span has been running, or `None` when idle.
    fn elapsed(self) -> Option<std::time::Duration> {
        match self {
            Self::Idle => None,
            Self::Busy { started_at } => Some(started_at.elapsed()),
        }
    }
}

/// What one settled turn cost, keyed by the run it belongs to.
///
/// The three values come from three places and none of them survives on its
/// own: the duration lives only while the span is `Busy`, the wall clock has no
/// source at all (the pane measures with a monotonic clock, which cannot name a
/// time of day), and the token count arrives on the turn's reply. They are
/// recorded together at the busy→idle settle edge — the one point that answers
/// "when did this finish" for everything else in the pane.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct TurnRecord {
    pub(super) worked_for: std::time::Duration,
    pub(super) finished_at: chrono::DateTime<chrono::Local>,
    /// `None` for an agent that reports no usage.
    pub(super) output_tokens: Option<u64>,
}

/// Terminal outcome of an activity span, captured when the turn/session ends
/// but fired (notification + backing-task done) only when the pane actually
/// settles busy→idle (which may trail `end_turn` while subagents finish).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::workspace) enum TurnOutcome {
    Completed,
    Errored,
    Stopped,
}

/// The pane's derived activity, the single source consumed by the working
/// indicator, the lane badge, and the status-pulse gate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::workspace) enum ActivityState {
    Idle,
    Working,
    AwaitingPermission,
}

/// AgentChat conversation content-column width mode. `Reading` constrains each
/// row to the configured reading width; `Full` spends the whole pane.
///
/// Deliberately no `Default`: which one a pane starts on is `agent
/// .use_reading_width`'s answer, and a type-level default would be a second
/// place to state it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::workspace) enum ChatContentWidth {
    Full,
    Reading,
}

impl ChatContentWidth {
    pub(in crate::workspace) fn from_config(use_reading_width: bool) -> Self {
        if use_reading_width {
            Self::Reading
        } else {
            Self::Full
        }
    }

    pub(in crate::workspace) fn is_reading(self) -> bool {
        matches!(self, Self::Reading)
    }

    pub(super) fn toggle(self) -> Self {
        match self {
            Self::Full => Self::Reading,
            Self::Reading => Self::Full,
        }
    }
}

/// Stable identity of a queued prompt, minted per pane. Lets the strip target
/// an entry for removal independent of its position, which shifts as earlier
/// entries drain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(in crate::workspace) struct PromptId(u64);

impl std::fmt::Display for PromptId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Where a prompt came from — the bottom-dock composer, or a phone-relayed
/// Telegram reply. Only the latter arms the Telegram first-response watch on
/// dispatch; an in-app prompt never does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::workspace) enum PromptOrigin {
    InApp,
    Telegram,
}

/// Whether a send put the prompt on the wire immediately or only queued it
/// behind an in-flight turn — the Telegram relay needs this to decide whether
/// a "queued" notice is owed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::workspace) enum PromptDispatch {
    SentNow,
    Queued,
    /// The pane is already holding `QUEUE_DEPTH_MAX` prompts, so this one was
    /// not taken. A named outcome rather than a silent drop: every producer
    /// (an agent's tool call, a phone `/say`, a person typing) has something
    /// to tell whoever sent it.
    QueueFull,
}

/// A Telegram-origin turn's first-response watch side effect, returned by
/// [`AgentChatView::apply_event`] so the Workspace event pump can relay exactly
/// one phone-visible acknowledgement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::workspace) enum PhoneAckEffect {
    None,
    Relay(FirstResponseOutcome),
    Fallback,
}

/// Which Telegram watch action `apply_event`'s tail owes, decided once per
/// event arm so the three lifecycle methods (clear/check/finish) each have
/// exactly one call site instead of being duplicated at every arm.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum PhoneTurnAction {
    #[default]
    None,
    /// A Stop's cancel-ack landed — the turn is over and no relay is owed.
    Clear,
    /// A permission card interrupted the turn. The card itself was relayed
    /// as the phone's first sign of life (before this fold), so the wait is
    /// answered — but the turn is still the phone's, and its completion is
    /// still owed.
    Answer,
    /// New conversation content arrived — check whether it resolves the
    /// armed watch.
    CheckUpdate,
    /// The turn or session settled — resolve the watch to its terminal
    /// effect.
    Finish,
}

/// A prompt submitted while a turn was in flight, held in the queue until it
/// can be dispatched. Not echoed into the transcript — lives only here (and
/// the queued-prompt strip) until drained and echoed as a `UserText`.
#[derive(Debug, Clone)]
pub(in crate::workspace) struct QueuedPrompt {
    pub id: PromptId,
    pub text: String,
    pub origin: PromptOrigin,
}

/// What the Escape shortcut resolved to for a focused Agent chat pane. Returned
/// so the `Workspace` shim fires the settle-edge completion only when a turn
/// was actually cancelled.
pub(in crate::workspace) enum EscapeOutcome {
    /// Cancelled an in-flight turn (or trailing background-subagent work) — the
    /// caller must run `reconcile_activity` + `fire_activity_completion`.
    Cancelled,
    /// Discarded a parked queue (the "Esc twice clears the queue" gesture); no
    /// turn was running, so there is no completion to fire.
    ClearedQueue,
    /// Nothing to act on — the caller reports not-handled so Escape propagates.
    Ignored,
}

/// Prompt-send sequencing: the buffered/parked queues plus the in-flight-turn
/// tracking that gates draining them, always read/mutated together by the
/// queue ops. One field on `AgentChatView` instead of four, and keeps `turn`
/// private the same way as a bare field would.
#[derive(Default)]
pub(in crate::workspace) struct PromptQueue {
    /// Prompts buffered because the session isn't ready to prompt or a turn is
    /// already in flight, in submission order. Not echoed into `items` — the
    /// queued-prompt strip reads this until `pump_pending_prompt` drains and
    /// echoes it as a `UserText`. Cleared on connect failure; never serialized.
    pub(in crate::workspace) pending_prompts: Vec<QueuedPrompt>,
    /// Prompts parked by a Stop (`cancel_turn`) while buffered behind the
    /// cancelled turn. NOT auto-drained — resumed explicitly (the strip's
    /// Resume button) or discarded (a second Esc / clear-all). Runtime-only.
    pub(in crate::workspace) paused_prompts: Vec<QueuedPrompt>,
    /// Monotonic counter minting the next [`PromptId`]. Never read outside
    /// `view/`. Runtime-only; never serialized.
    next_prompt_id: u64,
    /// The queued prompt currently being edited in the composer, if any. Set
    /// by `begin_edit`, cleared on send/cancel/drain. While `Some`, a send
    /// replaces its text instead of enqueuing a new prompt. Runtime-only.
    pub(in crate::workspace) editing_prompt: Option<PromptId>,
    /// Whether a prompt turn is in flight, carrying its start instant.
    /// Module-private: this is prompt-queue sequencing, not the pane's
    /// activity signal — external code must read `is_busy` / `activity_state`
    /// / `activity_elapsed` instead. Tests use the `#[cfg(test)]` hooks.
    turn: Turn,
}

/// Async-built rendering artifacts derived from the conversation: read-only
/// diff editors, read-only verbatim tool-output editors, rasterized mermaid
/// diagrams, and decoded tool-output images. Each cache is filled by its own
/// reconciler in `reconcile.rs`; `render/` only ever reads them by reference.
/// One field instead of nine, with one [`Self::clear`] wiping all of them on a
/// session reset.
#[derive(Default)]
pub(in crate::workspace) struct AssetCache {
    /// Read-only diff editor entities for tool-call file edits, keyed by
    /// `"{tool_call_id}#{diff_index}"`. Built by `reconcile_diff_editors`;
    /// `render` only embeds them, never creates one.
    pub(in crate::workspace) diff_editors: DiffEditors,
    /// Fingerprint each `diff_editors` entry was built from. Lets
    /// `reconcile_diff_editors` detect a replaced diff (streaming growth from
    /// a partial snapshot) and rebuild instead of leaving it stale.
    pub(super) diff_editor_sources: HashMap<String, u64>,
    /// Added/removed line counts per tool-call diff, same keys as
    /// `diff_editors`. Runtime cache; never serialized.
    pub(super) diff_stats: DiffStats,
    /// Read-only editor entities for verbatim tool-output blocks, keyed by
    /// `"{tool_call_id}#{block_index}"`. Built by `reconcile_output_editors`;
    /// `render` only embeds them, never creates one.
    pub(in crate::workspace) output_editors: OutputEditors,
    /// Fingerprint each `output_editors` entry was built from, so a streamed
    /// output that grew is rebuilt rather than left frozen on a partial
    /// snapshot.
    pub(super) output_editor_sources: HashMap<String, u64>,
    /// Rendered mermaid diagrams by fence-source hash, filled async by
    /// `reconcile_mermaid`. Shared `Arc<Mutex<…>>` so gpui's texture cache
    /// hits instead of re-uploading the bitmap every frame.
    pub(super) mermaid_images: MermaidImages,
    /// Source hashes with a rasterization currently spawned, so
    /// `reconcile_mermaid` doesn't re-spawn while one is still rendering.
    pub(super) mermaid_inflight: HashSet<u64>,
    /// Decoded tool-output images by content hash. `Some` = decoded; `None` =
    /// cached decode failure (renders a label once, never re-spawned). Filled
    /// async by `reconcile_tool_images`.
    pub(super) tool_images: ToolImages,
    /// Content hashes with a decode currently spawned, so
    /// `reconcile_tool_images` doesn't re-spawn one still decoding.
    pub(super) tool_image_inflight: HashSet<u64>,
    /// Decoded local image resources by `"{tool_call_id}#{block_index}"`.
    pub(super) resource_images: ResourceImages,
    /// Fingerprint of the resource URI + MIME currently represented by each
    /// cache entry, so a replaced streamed block invalidates the old bitmap.
    pub(super) resource_image_sources: HashMap<String, u64>,
    /// Least-recently reconciled to most-recently reconciled resource keys.
    /// Keeps the decoded cache bounded even when a transcript contains many
    /// expanded image-producing tools.
    pub(super) resource_image_order: VecDeque<String>,
    /// Fingerprint currently being read for each resource block. Storing the
    /// value as well as the key prevents a late old task overwriting a newer
    /// source at the same block index.
    pub(super) resource_image_inflight: HashMap<String, u64>,
}

impl AssetCache {
    /// Wipe every cache in place — same `Arc`s, contents cleared — so a live
    /// render-side clone of the shared image caches (cloned fresh per render;
    /// see `render/blocks.rs`) stays attached rather than orphaned.
    fn clear(&mut self) {
        self.diff_editors.clear();
        self.diff_editor_sources.clear();
        self.diff_stats.clear();
        self.output_editors.clear();
        self.output_editor_sources.clear();
        self.clear_mermaid();
        self.tool_image_inflight.clear();
        if let Ok(mut m) = self.tool_images.lock() {
            m.clear();
        }
        self.resource_image_sources.clear();
        self.resource_image_order.clear();
        self.resource_image_inflight.clear();
        if let Ok(mut m) = self.resource_images.lock() {
            m.clear();
        }
    }

    pub(in crate::workspace) fn clear_mermaid(&mut self) {
        self.mermaid_inflight.clear();
        if let Ok(mut m) = self.mermaid_images.lock() {
            m.clear();
        }
    }
}

/// The pane's activity bookkeeping: whether it's busy, since when, and what
/// completion is owed once it settles. Its consumer methods (`is_busy`,
/// `reconcile_activity`, …) stay `impl AgentChatView` rather than `Self`
/// since they also need `queue.turn` and `items`, which live outside this type.
#[derive(Default)]
pub(in crate::workspace) struct ActivityTracker {
    /// Per-subagent last-activity timestamps by parent tool id. Bumped on each
    /// child tool-call event; keeps a subagent's badge "active" across the
    /// gaps between its sequential child calls (see [`SUBAGENT_QUIESCENCE`]).
    pub(in crate::workspace) subagent_last_activity: HashMap<String, std::time::Instant>,
    /// When each tool call seen in the current busy span was first reported, by
    /// tool id. The protocol carries no timestamp and the mapper is deliberately
    /// clock-free, so the host records the first sighting here — the only source
    /// a card's "how long has this been running" can have.
    ///
    /// Bounded at the settle edge, which keeps only the calls still live (a
    /// top-level one can outlast the edge). A replay is skipped at the insert: a
    /// restored call's real start is long before the reconnect, so timing it
    /// from there would be a lie.
    pub(super) tool_started_at: HashMap<String, std::time::Instant>,
    /// The busy span as of the last `reconcile_activity` tick: both the
    /// edge-detection memory that turns the `is_busy` level signal into
    /// idle→busy / busy→idle transitions, and the wall-clock anchor the
    /// working-indicator timer runs off. The anchor spans the whole run (turn
    /// + trailing subagents), not just the foreground turn.
    pub(in crate::workspace) span: ActivitySpan,
    /// Outcome captured when the turn/session ended, held until the pane
    /// actually settles busy→idle (may trail `end_turn` while subagents
    /// finish), so the completion signal fires at the true settle point.
    pub(in crate::workspace) pending_completion: Option<TurnOutcome>,
    /// What each settled run cost, keyed by the run's first item — the same key
    /// the run's fold uses, so a record and the bar above it name one thing.
    pub(super) turn_records: HashMap<usize, TurnRecord>,
}

/// Native ACP (Agent Client Protocol) chat pane, owned as `Entity<AgentChatView>`.
/// Owns the live [`daruda_acp::AcpSessionHandle`]; dropping the view (pane
/// close) drops the handle and the event-pump task — no explicit teardown.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
// `pub(crate)`, unlike its siblings: `--screenshot-scenario` addresses a tab by
// token from `crate::screenshot`, outside the workspace fence. A pure value
// enum with no workspace coupling, so nothing leaks with it.
pub(crate) enum ActivityOptionsTab {
    Fold,
    Filter,
    RecentSteps,
}

impl ActivityOptionsTab {
    pub(crate) const ALL: [Self; 3] = [Self::Fold, Self::Filter, Self::RecentSteps];

    /// Stable token — element ids and the `--screenshot-scenario` suffix. One
    /// source, so a capture cannot name a tab the panel spells differently.
    pub(crate) fn token(self) -> &'static str {
        match self {
            Self::Fold => "fold",
            Self::Filter => "filter",
            Self::RecentSteps => "recent-steps",
        }
    }

    /// Only the `--screenshot-scenario` parser reads a tab back from its token.
    #[cfg(feature = "screenshot")]
    pub(crate) fn from_token(token: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|tab| tab.token() == token)
    }
}

pub(in crate::workspace) struct AgentChatView {
    /// The owning pane's id — keys element ids, log dedup tags, and pane lookup.
    pub(super) pane_id: PaneId,
    /// The workspace window this view renders in, captured at construction so
    /// diff-editor / `InputState` creation can re-enter the workspace window.
    /// Also the way back to the owning `Workspace` for render-time actions
    /// this self-owned entity dispatches into it (diff-header "open in file
    /// view" / "open externally") — resolved on demand via
    /// `WindowRegistry::workspace_for_window`, the same lookup this view's own
    /// pane context-menu builder already uses (`render/mod.rs`).
    pub(super) window_handle: AnyWindowHandle,
    /// Pane-level focus handle for `Cmd+W` close routing. The view's `render`
    /// tracks it (like `TerminalView`), so `wrapper_focus_handle` returns
    /// `None` for this content kind.
    pub(in crate::workspace) focus_handle: FocusHandle,
    /// Lane working directory the session is rooted at. `None` = no
    /// resolvable cwd; `Some(Remote)` for a different-machine session.
    /// Local-only consumers must go through `PaneCwd::as_local` / `into_local`.
    pub(in crate::workspace) cwd: Option<PaneCwd>,
    /// The agent this pane runs under (config `[[agents]]` catalog id),
    /// resolved to a launch command at connect. Persisted so restore resumes
    /// under the same agent, whose session id is then resumable.
    pub(in crate::workspace) agent_id: String,
    /// Bare adapter command used by the current connection. Vocabulary events
    /// are attributed to this frozen source rather than the live catalog,
    /// which may be edited while the old process is still connected.
    pub(super) agent_vocabulary_source: Option<String>,
    /// What the connected agent called itself at `initialize`. Decides the ACP
    /// dialect the pane reads traffic with; `None` until a connect reports one,
    /// which leaves [`agent_id`](Self::agent_id) to decide. Not persisted — the
    /// agent re-reports it on every connect, and a restored pane has no traffic
    /// to map until it reconnects.
    pub(super) agent_program: Option<String>,
    /// Display name for `agent_id`, refreshed on config reload. Used as the
    /// activity-bar title fallback before the session reports its own title.
    pub(in crate::workspace) agent_name: String,
    /// Connection lifecycle state. Drives the status line + input/cancel
    /// affordance.
    pub(in crate::workspace) status: AgentSessionStatus,
    /// Persisted ACP session id, set once `Connected` establishes a live
    /// session or seeded from restore. Present → the lazy connect resumes via
    /// `session/load` instead of starting fresh.
    pub(in crate::workspace) session_id: Option<String>,
    /// ACP session-mode id this session was last known to be in, mirrored on
    /// every `Connected`/`ModeChanged` and persisted so a resume can reapply
    /// it via `session/set_mode`.
    ///
    /// WORKAROUND: `claude-agent-acp` recomputes its mode from `settings.json`
    /// on every process launch instead of the session's actual last mode, so
    /// `session/load`'s response alone can't be trusted. The host tracks and
    /// reapplies this itself until that's fixed upstream.
    pub(in crate::workspace) last_known_mode_id: Option<String>,
    /// Model id the *user* picked for this pane — set only by the model chip,
    /// never mirrored from what the adapter reports and never written by the
    /// connect-time apply of the agent's `default_model` (that would make the
    /// setting unchangeable, since a pick outranks it). Persisted, and
    /// reapplied on every connect, so the pick outlives both the session it
    /// was made in and the app run.
    pub(in crate::workspace) last_known_model_id: Option<String>,
    /// Whether a resume (`session/load`) is replaying its history. While
    /// `Loading`, `apply_event` accumulates items but skips the per-event
    /// rebuild + notify (O(n²) over the replay) until `Connected` clears it.
    /// Module-private: outside `view/` the gate is opened only by naming the
    /// resume target through [`Self::begin_connect`], and read through
    /// [`Self::is_replaying`].
    replay: Replay,
    /// Conversation render model, in arrival order. The event pump
    /// appends/folds into this; the renderer reads it.
    //
    // INVARIANT: `FoldKey::Assistant`/`Thinking` and the per-message markdown
    // selection ids are keyed by item index; valid only because `items` is
    // append-only. Removing/reordering items would require clearing
    // `FoldState` and would break markdown selection identity.
    pub(in crate::workspace) items: Vec<ChatItem>,
    /// Live ACP session handle. `None` until connect resolves, or after a
    /// terminal [`AcpEvent::Error`] (not cleared by a per-turn `TurnFailed` —
    /// the connection is still alive). Dropping it closes the command channel.
    pub(in crate::workspace) handle: Option<AcpSessionHandle>,
    /// The buffered/parked queue of prompts not yet on the wire, plus the
    /// in-flight-turn sequencing that gates draining it — see [`PromptQueue`].
    pub(in crate::workspace) queue: PromptQueue,
    /// Text to put in front of this session's *first* prompt, without showing
    /// it in the transcript. `None` for every pane but the orchestrator's,
    /// which is briefed on what it is and which tools it has — see
    /// `orchestrator::briefing`.
    ///
    /// One-shot: [`Self::wire_text`] takes it. A pane that carries none is
    /// structurally unable to inject anything, so no send path needs to ask
    /// whose pane it is.
    briefing: Option<String>,
    /// GPUI-side pump that drains the `AcpEvent` receiver and folds events into
    /// `items` / `status`. Dropped with the view, ending the loop.
    pub(super) _event_pump: Option<Task<()>>,
    /// Outstanding permission-card request ids, mirroring items with
    /// unresolved cards. Updated in lockstep at every touch site
    /// (`PermissionRequested` / `respond_permission` / teardown); holds every
    /// outstanding id since permission requests can run in parallel.
    pub(in crate::workspace) pending_permissions: HashSet<u64>,
    /// Permission ids relayed to the current phone for this connection.
    /// Cleared on recipient change or session teardown; the periodic sweep
    /// prunes resolved requests and relays outstanding ids absent here.
    pub(in crate::workspace) permissions_told_to_phone: HashSet<u64>,
    /// The phone's side of the in-flight turn — see [`PhoneTurn`]. Armed the
    /// instant a Telegram-origin prompt dispatches, answered once a report
    /// goes out, and `None` when the turn is not the phone's, was consumed by
    /// a permission request, or has been completed. Read by the connect-ops
    /// event pump, the periodic Telegram flush pump, and the completion
    /// relay. Transitions go through the methods in `view/queue_ops.rs`; the
    /// type itself is sealed.
    phone_turn_state: Option<PhoneTurn>,
    /// Diff editors, mermaid diagrams, and tool-output images built async
    /// from the conversation's content — see [`AssetCache`].
    pub(in crate::workspace) assets: AssetCache,
    /// The pane's activity bookkeeping — see [`ActivityTracker`]. Its consumer
    /// methods stay `impl AgentChatView` since they also need `queue.turn`
    /// and `items`, neither of which moved into `ActivityTracker`.
    pub(in crate::workspace) activity: ActivityTracker,
    /// Persisted pane mode plus session-only block overrides.
    pub(in crate::workspace) fold: FoldState,
    /// The fold editor's own state — the turn column it shows and the matrix
    /// its `Custom` segment restores. Session-only: the mode itself is
    /// persisted, so only "press a preset then come back" is scoped here.
    pub(super) fold_editor: FoldEditorState,
    /// Active section in the compact Activity Bar's combined options popover.
    pub(super) activity_options_tab: ActivityOptionsTab,
    #[cfg(feature = "screenshot")]
    pub(super) screenshot_filter_open: bool,
    #[cfg(feature = "screenshot")]
    pub(super) screenshot_fold_open: bool,
    #[cfg(feature = "screenshot")]
    pub(super) screenshot_options_open: bool,
    /// Per-pane content-column width mode, seeded from
    /// `agent.use_reading_width` and persisted only once the pane's own toggle
    /// has moved it (see `SerializedAgentChatContent::content_width`).
    pub(in crate::workspace) content_width: ChatContentWidth,
    /// Whether [`Self::content_width`] is still the seeded value. A pane that
    /// never touched the toggle follows config across reloads and restarts;
    /// one that did keeps its own answer.
    pub(in crate::workspace) content_width_chosen: bool,
    /// The recent-steps axis, one choice per level: how many work steps of a
    /// response stay on screen, and how many calls inside one of those steps.
    /// Two choices rather than one over a pair, so pinning the step level
    /// leaves the call level following config.
    pub(in crate::workspace) tail_steps: PaneChoice<TailWindow>,
    pub(in crate::workspace) tail_calls: PaneChoice<TailWindow>,
    /// Display filter and whether it still follows config.
    pub(in crate::workspace) display_filter: PaneChoice<DisplayFilter>,
    /// The defaults the Workspace last resolved for this pane's agent. Kept so
    /// a reset can hand an axis back to config without the view re-deriving it;
    /// written only where the seeds are applied (construction and
    /// [`Self::reseed_transcript_defaults`]).
    pub(in crate::workspace) defaults: TranscriptDefaults,
    /// Virtualized conversation list state (gpui `list`). [`FollowMode::Tail`]
    /// auto-scrolls with streaming output and re-engages when the user scrolls
    /// back to the bottom. Synced with `items` via [`Self::sync_list_after`].
    pub(in crate::workspace) list_state: ListState,
    /// Render-row projection of `items` under `fold`, recomputed by
    /// [`Self::rebuild_rows`] on every change. The virtualized list indexes
    /// over this. Derived cache — single rebuild site.
    pub(in crate::workspace) rows: Vec<RenderRow>,
    /// The activity level `rows` was projected under — the key of that cache's
    /// one time-dependent input. `activity_state()` reads the clock (a trailing
    /// subagent stays busy until its quiescence window lapses), so the working
    /// indicator can go stale with no model change to trigger a rebuild. Keeping
    /// the key lets [`Self::reproject_if_activity_changed`] restore the
    /// projection whichever path moved the level.
    pub(super) rows_activity: ActivityState,
    /// Which subagent units still have work running, derived from `items` in one
    /// pass. Read by the projection *and* by every tool card's badge, so it is
    /// cached here rather than recomputed per query. Derived cache — rebuilt in
    /// [`Self::rebuild_rows`] alongside `rows`, its single update site.
    pub(super) live_units: LiveSubagentUnits,
    /// Cached subtree-aware display-filter matches.
    pub(super) filter_matches: FilterMatchIndex,
    /// Cached start of the newest turn.
    pub(super) turn_boundary: super::agent_chat_helpers::TurnBoundary,
    /// Workspace-resolved syntax-highlight theme id for this pane's diff embeds.
    /// The Workspace owns the resolved value (user + project config layers), so
    /// it cannot be derived here — it is seeded at construction and re-pushed on
    /// a config reload. Construction rather than the first ACP event, because a
    /// fold expand or a seeded transcript can ask for a diff embed without one.
    /// Single update site: [`Self::set_syntax_theme`].
    syntax_theme: String,
    /// Activity-bar title derived from `session_title` + the first user prompt.
    /// Neither input moves per frame, but resolving it *in* `render` made it the
    /// paint path's top cost on a pane whose first message was long. Derived
    /// cache — rebuilt in [`Self::rebuild_rows`] alongside `rows` / `live_units`,
    /// its single update site, which every mutation of either input already ends
    /// in. (A `session/load` replay defers that funnel, so the bar shows the
    /// agent-name fallback until the catch-up — the same window in which `rows`
    /// is deliberately stale and no conversation is on screen yet.)
    activity_title: Option<String>,
    /// What the connected agent advertises: modes, config options, slash
    /// commands. Established at `Connected`, cleared together on teardown.
    pub(in crate::workspace) session_config: SessionConfig,
    /// Optional session methods the agent advertised at connect (`load` /
    /// `list` / `resume` / `close`), consumed by resume gating. Re-read each
    /// connect; default = baseline agent (nothing extra).
    pub(super) session_capabilities: SessionCapabilitiesView,
    /// Live context-window / cost accounting from `UsageChanged`. Drives the
    /// context meter — distinct from the cumulative Usage tab. Cleared on a
    /// fresh session.
    pub(super) session_usage: Option<UsageView>,
    /// The agent's live execution plan (`PlanChanged`); full-replaced each
    /// update. Runtime-only; never serialized.
    pub(super) plan: Vec<PlanEntryView>,
    /// Agent-provided session title (`SessionInfoChanged`); `None` = fallback
    /// label.
    pub(in crate::workspace) session_title: Option<String>,
    /// Agent-provided last-activity timestamp (ISO 8601); `None` = unknown.
    /// Shown as a tooltip on the activity-bar title.
    pub(in crate::workspace) session_updated_at: Option<String>,
    /// Whether the bottom plan region is collapsed to its header. Defaults to
    /// `false` (expanded); toggled via [`Self::toggle_plan_collapsed`].
    pub(super) plan_collapsed: bool,
    /// Latches once this pane has logged a `dropped_terminal_output` warning,
    /// so a systemic adapter mismatch (every command in the session would
    /// trip it) logs one line instead of one per command. Reset alongside
    /// `plan`/`session_title`/`session_usage` on a fresh (non-resumed) session.
    pub(super) warned_dropped_terminal_output: bool,
    /// Scroll position of the expanded plan checklist, backing its 4px daruda
    /// thumb overlay. Runtime-only; never serialized.
    pub(super) plan_scroll: ScrollHandle,
    /// Window-space bounds of the scrolling list viewport, captured each
    /// paint. Read by the drag-selection autoscroll poll to detect the cursor
    /// leaving the viewport. `None` until the first paint.
    pub(in crate::workspace) list_bounds: Option<Bounds<Pixels>>,
    /// Width of the pane root, captured each paint.
    pub(super) pane_width: Option<Pixels>,
    /// Drag-selection autoscroll poll task (mirrors
    /// `TerminalView::autoscroll_task`). Replace-and-cancel on each new drag;
    /// `None` when idle.
    pub(super) autoscroll_task: Option<Task<()>>,
    /// Drag-selection-in-progress signal, set on mouse-down and cleared via
    /// `end_selection_drag`. Independent of the selected block's paint
    /// lifetime, so the autoscroll poll still stops on mouse-release even if
    /// the block unmounts mid-drag in the virtualized list.
    pub(in crate::workspace) selection_drag_active: bool,
    /// Inactive-pane dim amount in `[0.0, 1.0]`: `0.0` = focused / full color,
    /// `> 0.0` = unfocused split leaf (see [`Self::dim`]). Single write site:
    /// `Workspace::refresh_pane_dimming` (MVU one-way flow).
    pub(in crate::workspace) dim_amount: f32,
    /// Observes the host theme so a light/dark toggle re-rasterizes cached
    /// mermaid diagrams (keyed by `(source, dark)`), which would otherwise
    /// stay stale until the next ACP event triggers a reconcile.
    _theme_observer: Subscription,
    /// Test-only render counter, asserted by the cache-scoping regression test
    /// to confirm a `cx.notify()` on this view actually re-renders it.
    #[cfg(test)]
    pub(in crate::workspace) render_count: std::cell::Cell<u32>,
}

mod activity_ops;
mod apply_event;
pub(super) mod list_sync;
mod queue_ops;
mod session_ops;

impl AgentChatView {
    /// Build a fresh view. The session is *not* started here — the Workspace
    /// connects it lazily on first focus (`maybe_connect_agent_chat`), so cold
    /// restore doesn't spin up an agent process per pane. `status` is decided
    /// by the caller (Idle when a cwd is present, Error otherwise).
    #[allow(clippy::too_many_arguments)] // Restore/create seed values — bundling them into a struct only wraps callers.
    pub(super) fn new(
        pane_id: PaneId,
        window_handle: AnyWindowHandle,
        cwd: Option<PaneCwd>,
        status: AgentSessionStatus,
        session_id: Option<String>,
        mode_id: Option<String>,
        agent_id: String,
        agent_name: String,
        title: Option<String>,
        defaults: TranscriptDefaults,
        syntax_theme: String,
        cx: &mut Context<Self>,
    ) -> Self {
        // Re-rasterize mermaid diagrams when the UI theme changes. The chat
        // surface/foreground come from the terminal mirror, but the host
        // profile still uses UI semantic colors for notes/status accents.
        let theme_observer = cx.observe_global::<crate::ui::theme::DarudaTheme>(|this, cx| {
            this.assets.clear_mermaid();
            let dark = Self::host_is_dark(cx);
            this.reconcile_mermaid(dark, cx);
            // A theme swap invalidates the whole conversation's cached artifacts,
            // not one call's — the only correct scope here is the full pass.
            this.reconcile_tool_images(&super::reconcile::ReconcileScope::All, cx);
            // Diff embeds bake their palette in and cannot re-theme themselves.
            this.reconcile_embeds_after_theme_change(cx);
        });
        Self {
            pane_id,
            window_handle,
            focus_handle: cx.focus_handle(),
            cwd,
            status,
            session_id,
            last_known_mode_id: mode_id,
            // Patched in by session restore right after construction, same as
            // the mode id above — see `Workspace::rebuild_layout`.
            last_known_model_id: None,
            agent_id,
            agent_program: None,
            agent_vocabulary_source: None,
            agent_name,
            // A restored pane connects lazily; the resume decision (and the
            // replay gate that coalesces it) is made at connect time by
            // `maybe_connect_agent_chat`, not here.
            replay: Replay::Live,
            items: Vec::new(),
            handle: None,
            queue: PromptQueue::default(),
            briefing: None,
            _event_pump: None,
            pending_permissions: HashSet::new(),
            permissions_told_to_phone: HashSet::new(),
            phone_turn_state: None,
            activity: ActivityTracker::default(),
            assets: AssetCache::default(),
            fold: FoldState::with_mode(defaults.fold_mode),
            fold_editor: FoldEditorState::default(),
            activity_options_tab: ActivityOptionsTab::Fold,
            #[cfg(feature = "screenshot")]
            screenshot_filter_open: false,
            #[cfg(feature = "screenshot")]
            screenshot_fold_open: false,
            #[cfg(feature = "screenshot")]
            screenshot_options_open: false,
            content_width: defaults.content_width,
            content_width_chosen: false,
            tail_steps: PaneChoice::Seeded(defaults.tail.steps),
            tail_calls: PaneChoice::Seeded(defaults.tail.calls),
            display_filter: PaneChoice::Seeded(defaults.filter),
            defaults,
            list_state: {
                // Starts empty; `sync_list_after` splices items in as events
                // arrive. `Top` alignment + `Tail` follow = scroll history up
                // freely, auto-stick to the bottom while streaming. Overdraw
                // 512px renders a little above/below the viewport so a fast
                // scroll doesn't flash blank rows.
                let state = ListState::new(0, ListAlignment::Top, px(512.));
                state.set_follow_mode(FollowMode::Tail);
                state
            },
            rows: Vec::new(),
            rows_activity: ActivityState::Idle,
            live_units: LiveSubagentUnits::default(),
            filter_matches: FilterMatchIndex::default(),
            turn_boundary: Default::default(),
            syntax_theme,
            activity_title: None,
            session_config: SessionConfig::default(),
            session_capabilities: SessionCapabilitiesView::default(),
            session_usage: None,
            plan: Vec::new(),
            // Seed the persisted title so a restored dormant pane shows its
            // label before the session loads; the agent replaces it on the
            // first `SessionInfoChanged` after resume. An empty string is
            // treated as absent, matching the tab-label seed in
            // `create_agent_chat_pane` so the header and tab agree.
            session_title: title.filter(|t| !t.is_empty()),
            session_updated_at: None,
            plan_collapsed: false,
            warned_dropped_terminal_output: false,
            plan_scroll: ScrollHandle::new(),
            list_bounds: None,
            pane_width: None,
            autoscroll_task: None,
            selection_drag_active: false,
            dim_amount: 0.0,
            _theme_observer: theme_observer,
            #[cfg(test)]
            render_count: std::cell::Cell::new(0),
        }
    }

    /// The resolved activity-bar title, or `None` when the session has neither
    /// an agent-supplied title nor a user prompt yet (the caller falls back to
    /// the pane's agent name).
    pub(in crate::workspace) fn activity_title(&self) -> Option<&str> {
        self.activity_title.as_deref()
    }

    /// Seed the agent-supplied title the way a `SessionInfoChanged` would,
    /// including the `rebuild_rows` that derives `activity_title` from it —
    /// writing the field alone leaves the derived cache stale.
    #[cfg(test)]
    pub(in crate::workspace) fn set_session_title_for_test(&mut self, title: &str) {
        self.session_title = Some(title.to_string());
        self.rebuild_rows();
    }

    /// The Workspace-resolved syntax theme id this pane's diff embeds are built
    /// against. Always set — see the field's doc for why it is a construction
    /// seed rather than something the first event fills in.
    pub(in crate::workspace) fn syntax_theme(&self) -> &str {
        &self.syntax_theme
    }

    /// Record the Workspace-resolved syntax theme id. Single update site for
    /// `syntax_theme`; `apply_event` writes the value the Workspace passes it
    /// each event, and the config-reload path writes the new palette name
    /// directly — an idle pane gets no events, so waiting for one would leave the
    /// diff embeds fingerprinted against a palette the user already left.
    pub(in crate::workspace) fn set_syntax_theme(&mut self, theme: &str) {
        if self.syntax_theme != theme {
            self.syntax_theme = theme.to_owned();
        }
    }

    /// Set the inactive-pane dim (`0.0` = focused, `> 0.0` = unfocused split
    /// leaf). No `cx.notify` here — the caller notifies only the views whose
    /// amount changed, mirroring `TerminalView::set_dim_amount`.
    pub(in crate::workspace) fn set_dim_amount(&mut self, amount: f32) {
        self.dim_amount = amount.clamp(0.0, 1.0);
    }

    /// Enter the `Connecting` status and repaint. Self-notifying so the event
    /// pump can't advance the connection state without dirtying the pane.
    pub(super) fn set_connecting(&mut self, cx: &mut Context<Self>) {
        self.status = AgentSessionStatus::Connecting;
        cx.notify();
    }

    /// Enter `Connecting` for a connect that is about to be spawned, arming the
    /// replay gate from that connect's own resume target: `Some(id)` asks for
    /// `session/load` of `id`, `None` for a fresh `session/new`.
    ///
    /// Taking the resume target as the argument is the point — the gate and the
    /// request cannot disagree, because there is no way to open one without
    /// naming the other. Distinct from [`Self::set_connecting`], which only
    /// downgrades the status banner and must leave a live replay alone.
    pub(in crate::workspace) fn begin_connect(
        &mut self,
        resume: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.replay = Replay::for_connect(resume);
        self.status = AgentSessionStatus::Connecting;
        cx.notify();
    }

    /// True while a `session/load` replay is still streaming — rows are being
    /// accumulated rather than projected. The gate is module-private, so this is
    /// how a test outside `view/` observes it (same shape as `turn_is_idle`).
    #[cfg(test)]
    pub(in crate::workspace) fn is_replaying(&self) -> bool {
        self.replay.is_loading()
    }

    /// Enter the `Error` status carrying `message` and repaint. Self-notifying
    /// so the event pump can't surface a failure without dirtying the pane.
    ///
    /// Callers that hold a classified failure pass its
    /// [`remedy`](daruda_acp::AcpFailure::remedy); a locally-raised error with
    /// nothing to offer passes [`Remedy::NoneAvailable`].
    pub(in crate::workspace) fn set_error(
        &mut self,
        message: String,
        remedy: daruda_acp::Remedy,
        cx: &mut Context<Self>,
    ) {
        self.status = AgentSessionStatus::Error { message, remedy };
        cx.notify();
    }

    /// Enter the `PreparingRuntime` status at `phase` and repaint.
    /// Self-notifying so the runtime-progress drain can't advance the banner
    /// without dirtying the pane.
    pub(super) fn set_preparing(&mut self, phase: RuntimePrepPhase, cx: &mut Context<Self>) {
        self.status = AgentSessionStatus::PreparingRuntime(phase);
        cx.notify();
    }

    /// Enter the `Handshaking` status at `phase` and repaint. Called from
    /// `apply_event` only while still `Connecting`/`Handshaking` — a
    /// `ConnectProgress` arriving after `Connected`/`Error` is stale and must
    /// not resurrect the connecting banner.
    pub(super) fn set_handshaking(&mut self, phase: ConnectPhase, cx: &mut Context<Self>) {
        self.status = AgentSessionStatus::Handshaking(phase);
        cx.notify();
    }

    /// Blend `c` toward gray by the current dim amount, alpha preserved. The
    /// render wraps every color it applies with this so an unfocused pane
    /// grays like an inactive terminal while keeping window translucency.
    pub(super) fn dim(&self, c: gpui::Hsla) -> gpui::Hsla {
        crate::ui::theme::dim_toward_gray(c, self.dim_amount)
    }

    /// Whether the agent-chat paint surface is currently dark. This follows the
    /// terminal-mirrored chat background, not the surrounding UI theme, so a
    /// light UI shell with a dark terminal preset still gets dark-surface
    /// Mermaid colors and cache keys.
    pub(super) fn host_is_dark(cx: &App) -> bool {
        !crate::ui::theme::agent_chat_syntax_is_light(cx)
    }
}

impl Focusable for AgentChatView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for AgentChatView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(test)]
        self.render_count.set(self.render_count.get() + 1);
        super::render::render(self, cx)
    }
}

#[cfg(test)]
mod turn_tests {
    use super::Turn;
    use std::time::Instant;

    /// The four transitions, and the one an open cancel window must survive:
    /// Stop settles the turn *before* the ack arrives, so a `finish` in between
    /// must not close the window the ack is owed against.
    #[test]
    fn the_cancel_window_outlives_a_settle_and_closes_only_on_its_ack() {
        let mut turn = Turn::default();
        assert!(turn.can_dispatch(), "an idle turn takes a prompt");

        turn.start(Instant::now());
        assert!(turn.is_in_flight());
        assert!(!turn.can_dispatch(), "one turn at a time");
        assert!(turn.started_at().is_some());

        assert!(turn.cancel(), "there was a turn to cut");
        assert!(turn.awaiting_cancel_ack());
        assert!(
            !turn.is_in_flight(),
            "Stop settles locally so a hung agent cannot pulse forever"
        );
        assert!(!turn.can_dispatch(), "a re-prompt buffers until the ack");
        assert!(turn.started_at().is_none(), "a cut turn records no span");

        turn.finish();
        assert!(turn.awaiting_cancel_ack(), "the window survives the settle");

        turn.acknowledge_cancel();
        assert!(turn.can_dispatch(), "the ack reopens the wire");
    }

    /// An Escape that only cancels a trailing subagent cuts no turn, so it owes
    /// no ack and earns no stop marker.
    #[test]
    fn cancelling_an_idle_turn_opens_no_window() {
        let mut turn = Turn::Idle;
        assert!(!turn.cancel());
        assert!(turn.can_dispatch());
    }

    /// A fresh-session reset acknowledges a window that may not be open; doing
    /// so must not settle a turn that is actually running.
    #[test]
    fn acknowledging_without_a_window_leaves_a_live_turn_alone() {
        let mut turn = Turn::Idle;
        turn.start(Instant::now());
        turn.acknowledge_cancel();
        assert!(turn.is_in_flight(), "no window was open to close");
    }
}

#[cfg(test)]
pub(super) mod tests;
