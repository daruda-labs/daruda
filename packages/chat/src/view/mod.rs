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
//! Workspace state; self-notifying pane entities are the sanctioned AGENTS.md
//! rule #10 exception also used by `TerminalView` and `ToastLayer`.

use std::collections::{BTreeSet, HashMap, HashSet};

use daruda_acp::{ChatItem, ConnectPhase, PlanEntryView, SessionCapabilitiesView, UsageView};
use daruda_content::link_target::{self, LinkTarget};
use daruda_core::process_env;
use daruda_store::project::PaneCwd;
use gpui::{
    AnyWindowHandle, App, Bounds, Context, FocusHandle, Focusable, FollowMode, ListAlignment,
    ListState, Pixels, ScrollHandle, Subscription, Task, Window, prelude::*, px,
};

use super::fold::FoldState;
use super::pane_choice::PaneChoice;
use super::phone_turn::{FirstResponseOutcome, PhoneTurn};
use super::rows::tail::TailWindow;
use super::rows::{FilterMatchIndex, LiveSubagentUnits, RenderRow};
use super::session_config::SessionConfig;
use super::transcript_defaults::TranscriptDefaults;
use daruda_store::project::PaneId;
use daruda_transcript::command_analysis::CommandAnalysisIndex;
use daruda_transcript::display_filter::DisplayFilter;
use daruda_transcript::editor::state::{FilterEditorState, FoldEditorState};

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

/// Which agent a pane runs under, as the host resolves it from the catalog.
/// Given together at construction and on [`AgentChatView::switch_agent`].
pub struct AgentIdentity {
    /// The `[[agents]]` catalog id.
    pub id: String,
    pub name: String,
    pub icon: Option<&'static str>,
}

/// A user-visible milestone of the one-time Node.js runtime provisioning shown
/// in the connecting banner — only the slow, user-facing phases; instant ones
/// (system-node found, cache probe) never surface one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimePrepPhase {
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
pub enum AgentSessionStatus {
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
    /// [`Remedy::NoneAvailable`](daruda_acp::Remedy::NoneAvailable).
    Error {
        message: String,
        remedy: daruda_acp::Remedy,
    },
}

impl AgentSessionStatus {
    /// Between asking for a session and having one — runtime provisioning,
    /// adapter spawn, handshake / replay.
    pub fn is_connecting(&self) -> bool {
        matches!(
            self,
            Self::PreparingRuntime(_) | Self::Connecting | Self::Handshaking(_)
        )
    }
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

/// A Telegram-origin turn's first-response watch side effect, returned by
/// [`AgentChatView::apply_event`] so the Workspace event pump can relay exactly
/// one phone-visible acknowledgement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PhoneAckEffect {
    None,
    Relay(FirstResponseOutcome),
    Fallback,
}

/// Prompt-send sequencing: the buffered/parked queues plus the in-flight-turn
/// tracking that gates draining them, always read/mutated together by the
/// queue ops. One field on `AgentChatView` instead of four, and keeps `turn`
/// private the same way as a bare field would.
#[derive(Default)]
pub struct PromptQueue {
    /// Prompts buffered because the session isn't ready to prompt or a turn is
    /// already in flight, in submission order. Not echoed into `items` — the
    /// queued-prompt strip reads this until `pump_pending_prompt` drains and
    /// echoes it as a `UserText`. Cleared on connect failure; never serialized.
    pub(crate) pending_prompts: Vec<QueuedPrompt>,
    /// Prompts parked by a Stop (`cancel_turn`) while buffered behind the
    /// cancelled turn. NOT auto-drained — resumed explicitly (the strip's
    /// Resume button) or discarded (a second Esc / clear-all). Runtime-only.
    pub(crate) paused_prompts: Vec<QueuedPrompt>,
    /// Monotonic counter minting the next [`PromptId`]. Never read outside
    /// `view/`. Runtime-only; never serialized.
    next_prompt_id: u64,
    /// The queued prompt currently being edited in the composer, if any. Set
    /// by `begin_edit`, cleared on send/cancel/drain. While `Some`, a send
    /// replaces its text instead of enqueuing a new prompt. Runtime-only.
    pub(crate) editing_prompt: Option<PromptId>,
    /// Whether a second empty-composer Enter resumes `paused_prompts`. Set by
    /// the first such Enter, cleared by a composer change and by every path
    /// that mutates the parked queue. Read only through `resume_armed()`, which
    /// also requires a non-empty parked queue. Runtime-only; never serialized.
    resume_armed: bool,
    /// Whether a prompt turn is in flight, carrying its start instant.
    /// Module-private: this is prompt-queue sequencing, not the pane's
    /// activity signal — external code must read `is_busy` / `activity_state`
    /// / `activity_elapsed` instead. Tests use the `#[cfg(test)]` hooks.
    turn: Turn,
}

/// The pane's activity bookkeeping: whether it's busy, since when, and what
/// completion is owed once it settles. Its consumer methods (`is_busy`,
/// `reconcile_activity`, …) stay `impl AgentChatView` rather than `Self`
/// since they also need `queue.turn` and `items`, which live outside this type.
#[derive(Default)]
pub(crate) struct ActivityTracker {
    /// Per-subagent last-activity timestamps by parent tool id. Bumped on each
    /// child tool-call event; keeps a subagent's badge "active" across the
    /// gaps between its sequential child calls (see [`SUBAGENT_QUIESCENCE`]).
    pub(crate) subagent_last_activity: HashMap<String, std::time::Instant>,
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
    pub(crate) span: ActivitySpan,
    /// Outcome captured when the turn/session ended, held until the pane
    /// actually settles busy→idle (may trail `end_turn` while subagents
    /// finish), so the completion signal fires at the true settle point.
    pub(crate) pending_completion: Option<TurnOutcome>,
    /// What each settled run cost, keyed by the run's first item — the same key
    /// the run's fold uses, so a record and the bar above it name one thing.
    pub(super) turn_records: HashMap<usize, TurnRecord>,
}

/// Native ACP (Agent Client Protocol) chat pane, owned as `Entity<AgentChatView>`.
/// Owns the live [`daruda_acp::AcpSessionHandle`]; dropping the view (pane
/// close) drops the handle and the event-pump task — no explicit teardown.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
// `--screenshot-scenario` addresses a tab by token, from the app. A pure value
// enum with no host coupling, so nothing leaks with it.
pub enum ActivityOptionsTab {
    Fold,
    Filter,
    RecentSteps,
}

impl ActivityOptionsTab {
    pub const ALL: [Self; 3] = [Self::Fold, Self::Filter, Self::RecentSteps];

    /// Stable `--screenshot-scenario` suffix. One source, so a capture cannot
    /// name a tab the parser spells differently.
    #[cfg(feature = "screenshot")]
    pub fn token(self) -> &'static str {
        match self {
            Self::Fold => "fold",
            Self::Filter => "filter",
            Self::RecentSteps => "recent-steps",
        }
    }

    /// Only the `--screenshot-scenario` parser reads a tab back from its token.
    #[cfg(feature = "screenshot")]
    pub fn from_token(token: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|tab| tab.token() == token)
    }
}

pub struct AgentChatView {
    /// The pane's ACP session and what it may do with it — see [`Session`].
    /// Private so every transition goes through the methods below.
    session: Session,
    /// The owning pane's id — keys element ids, log dedup tags, and pane lookup.
    pub(super) pane_id: PaneId,
    /// The workspace window this view renders in, captured at construction so
    /// diff-editor / `InputState` creation can re-enter the workspace window.
    /// Never a way back to the `Workspace`: asks go out as [`AgentChatEvent`].
    pub(super) window_handle: AnyWindowHandle,
    /// Pane-level focus handle for `Cmd+W` close routing. The view's `render`
    /// tracks it (like `TerminalView`), so `wrapper_focus_handle` returns
    /// `None` for this content kind.
    pub(super) focus_handle: FocusHandle,
    /// Lane working directory the session is rooted at. `None` = no
    /// resolvable cwd; `Some(Remote)` for a different-machine session.
    /// Local-only consumers must go through `PaneCwd::as_local` / `into_local`.
    pub(super) cwd: Option<PaneCwd>,
    /// The agent this pane runs under (config `[[agents]]` catalog id),
    /// resolved to a launch command at connect. Persisted so restore resumes
    /// under the same agent, whose session id is then resumable.
    pub(super) agent_id: String,
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
    pub(super) agent_name: String,
    /// The agent's mark for the activity bar, resolved by the host along with
    /// `agent_id` and `agent_name`.
    pub(super) agent_icon: Option<&'static str>,
    /// Connection lifecycle state. Drives the status line + input/cancel
    /// affordance.
    pub(super) status: AgentSessionStatus,
    /// Persisted ACP session id, set once `Connected` establishes a live
    /// session or seeded from restore. Present → the lazy connect resumes via
    /// `session/load` instead of starting fresh.
    pub(super) session_id: Option<String>,
    /// Session-mode id the *user* picked for this pane — set only by the mode
    /// chip and its cycle shortcut, never mirrored from what the adapter
    /// reports, so a mode the adapter switched to on its own cannot outrank
    /// the agent's `default_mode`. Persisted, and requested on every connect.
    pub(super) picked_mode_id: Option<String>,
    /// Model id the *user* picked for this pane — set only by the model chip,
    /// never mirrored from what the adapter reports and never written by the
    /// connect-time apply of the agent's `default_model` (that would make the
    /// setting unchangeable, since a pick outranks it). Persisted, and
    /// reapplied on every connect, so the pick outlives both the session it
    /// was made in and the app run.
    pub(super) picked_model_id: Option<String>,
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
    pub(super) items: Vec<ChatItem>,
    pub(super) host_cards: crate::host_cards::HostCards,
    pub(super) host_cards_scroll: ScrollHandle,
    /// The buffered/parked queue of prompts not yet on the wire, plus the
    /// in-flight-turn sequencing that gates draining it — see [`PromptQueue`].
    pub(super) queue: PromptQueue,
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
    pub(super) pending_permissions: HashSet<u64>,
    pub(super) permission_session: u64,
    /// Permission ids relayed to the current phone for this connection.
    /// Cleared on recipient change or session teardown; the periodic sweep
    /// prunes resolved requests and relays outstanding ids absent here.
    pub(super) permissions_told_to_phone: HashSet<u64>,
    /// The phone's side of the in-flight turn — see [`PhoneTurn`]. Armed the
    /// instant a Telegram-origin prompt dispatches, answered once a report
    /// goes out, and `None` when the turn is not the phone's, was consumed by
    /// a permission request, or has been completed. Read by the connect-ops
    /// event pump, the periodic Telegram flush pump, and the completion
    /// relay. Transitions go through the methods in `view/queue_ops/phone.rs`; the
    /// type itself is sealed.
    phone_turn_state: Option<PhoneTurn>,
    /// Diff editors, mermaid diagrams, and tool-output images built async
    /// from the conversation's content — see [`AssetCache`].
    pub(super) assets: AssetCache,
    /// The pane's activity bookkeeping — see [`ActivityTracker`]. Its consumer
    /// methods stay `impl AgentChatView` since they also need `queue.turn`
    /// and `items`, neither of which moved into `ActivityTracker`.
    pub(super) activity: ActivityTracker,
    /// Persisted pane mode plus session-only block overrides.
    pub(super) fold: FoldState,
    /// The fold editor's own state — the turn column it shows and the matrix
    /// its `Custom` segment restores. Session-only: the mode itself is
    /// persisted, so only "press a preset then come back" is scoped here.
    pub(super) fold_editor: FoldEditorState,
    /// Which Visible-items sections are shut. Session-only presentation, like
    /// `fold_editor`: the filter itself is what persists.
    pub(super) filter_editor: FilterEditorState,
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
    pub(super) content_width: ChatContentWidth,
    /// Whether [`Self::content_width`] is still the seeded value. A pane that
    /// never touched the toggle follows config across reloads and restarts;
    /// one that did keeps its own answer.
    pub(super) content_width_chosen: bool,
    /// The recent-steps axis, one choice per level: how many work steps of a
    /// response stay on screen, and how many calls inside one of those steps.
    /// Two choices rather than one over a pair, so pinning the step level
    /// leaves the call level following config.
    pub(super) tail_steps: PaneChoice<TailWindow>,
    pub(super) tail_calls: PaneChoice<TailWindow>,
    /// Display filter and whether it still follows config.
    pub(super) display_filter: PaneChoice<DisplayFilter>,
    /// The defaults the Workspace last resolved for this pane's agent. Kept so
    /// a reset can hand an axis back to config without the view re-deriving it;
    /// written only where the seeds are applied (construction and
    /// [`Self::reseed_transcript_defaults`]).
    pub(super) defaults: TranscriptDefaults,
    /// Virtualized conversation list state (gpui `list`). [`FollowMode::Tail`]
    /// auto-scrolls with streaming output and re-engages when the user scrolls
    /// back to the bottom. Synced with `items` via [`Self::sync_list_after`].
    pub(super) list_state: ListState,
    /// Render-row projection of `items` under `fold`, recomputed by
    /// [`Self::rebuild_rows`] on every change. The virtualized list indexes
    /// over this. Derived cache — single rebuild site.
    pub(super) rows: Vec<RenderRow>,
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
    /// Structured command metadata, refreshed from raw input in `rebuild_rows`.
    pub(super) command_analysis: CommandAnalysisIndex,
    /// Cached start of the newest turn.
    pub(super) turn_boundary: super::agent_chat_helpers::TurnBoundary,
    /// Workspace-resolved syntax-highlight theme id for this pane's diff embeds.
    /// The Workspace owns the resolved value (user + project config layers), so
    /// it cannot be derived here — it is seeded at construction and re-pushed on
    /// a config reload. Construction rather than the first ACP event, because a
    /// fold expand or a seeded transcript can ask for a diff embed without one.
    /// Single update site: [`Self::set_syntax_theme`].
    syntax_theme: String,
    /// The configured external editor's preset name, so a diff header can say
    /// which editor "open externally" launches. Pushed by the host, like
    /// `syntax_theme`; single update site: [`Self::set_preferred_editor`].
    preferred_editor: String,
    /// Persists this pane's own choice as its agent's default. The host owns
    /// the settings store; the pane only learns whether the write landed.
    agent_default_writer: AgentDefaultWriter,
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
    pub(super) session_config: SessionConfig,
    /// Optional session methods the agent advertised at connect (`load` /
    /// `list` / `resume` / `close`), consumed by resume gating. Re-read each
    /// connect; default = baseline agent (nothing extra).
    pub(super) session_capabilities: SessionCapabilitiesView,
    pub(super) draft_attachments: Vec<daruda_acp::PromptAttachment>,
    /// Live context-window / cost accounting from `UsageChanged`. Drives the
    /// context meter — distinct from the cumulative Usage tab. Cleared on a
    /// fresh session.
    pub(super) session_usage: Option<UsageView>,
    /// Set when this turn was stopped because Claude reported extra-usage
    /// billing; cleared when the next prompt dispatches.
    pub(super) overage_notice: Option<OverageNotice>,
    /// Indices of the `ChatItem::Interrupted` markers an overage stop pushed,
    /// so they read as such. A restored pane shows "Stopped by user" for them:
    /// the replayed entry carries no stop reason.
    pub(super) overage_stop_markers: BTreeSet<usize>,
    /// The agent's live execution plan (`PlanChanged`); full-replaced each
    /// update. Runtime-only; never serialized.
    pub(super) plan: Vec<PlanEntryView>,
    /// Agent-provided session title (`SessionInfoChanged`); `None` = fallback
    /// label.
    pub(super) session_title: Option<String>,
    /// Agent-provided last-activity timestamp (ISO 8601); `None` = unknown.
    /// Shown as a tooltip on the activity-bar title.
    pub(super) session_updated_at: Option<String>,
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
    pub(super) list_bounds: Option<Bounds<Pixels>>,
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
    pub(super) selection_drag_active: bool,
    /// Inactive-pane dim amount in `[0.0, 1.0]`: `0.0` = focused / full color,
    /// `> 0.0` = unfocused split leaf (see [`Self::dim`]). Single write site:
    /// `Workspace::refresh_pane_dimming` (MVU one-way flow).
    pub(super) dim_amount: f32,
    /// Observes the host theme so a light/dark toggle re-rasterizes cached
    /// mermaid diagrams (keyed by `(source, dark)`), which would otherwise
    /// stay stale until the next ACP event triggers a reconcile.
    _theme_observer: Subscription,
    /// Test-only render counter, asserted by the cache-scoping regression test
    /// to confirm a `cx.notify()` on this view actually re-renders it.
    #[cfg(any(test, feature = "test-support"))]
    pub(super) render_count: std::cell::Cell<u32>,
}

pub use crate::host_cards::{HostCard, HostCardAction};
pub use event::{AgentChatEvent, PermissionOutcome, PromptDropReason};
mod activity_types;
pub(crate) use activity_types::{ActivitySpan, ToolSummaryStyle, TurnRecord};
pub use activity_types::{ActivityState, ChatContentWidth, TurnOutcome};
mod asset_cache;
pub(crate) use asset_cache::AssetCache;
mod construct;
mod display_ops;
mod fold_ops;
mod prompt_types;
use prompt_types::PhoneTurnAction;
pub use prompt_types::{
    EmptySubmitOutcome, EscapeOutcome, InternalDispatch, PromptDispatch, PromptId, PromptOrigin,
    QueuedPrompt,
};
mod session_access;
pub use pane_choices::ChatPaneChoices;
pub use session_access::LoadIntent;
pub(crate) use session_access::Session;
mod activity_ops;
mod agent_default_ops;
pub use activity_ops::RunSummary;
pub use agent_default_ops::AgentDefaultWriter;
mod apply_event;
mod attachments;
mod event;
mod host_commands;
mod host_surface;
pub(super) mod list_sync;
mod pane_choices;
mod queue_ops;
pub use attachments::display_prompt;
mod overage_notice;
#[cfg(feature = "devtools")]
mod seed_ops;
pub use overage_notice::OverageNotice;
mod session_ops;
#[cfg(feature = "screenshot")]
mod shot_ops;
#[cfg(any(test, feature = "test-support"))]
mod test_hooks;

impl AgentChatView {
    /// Where `link` points as this pane's session sees it: against its local
    /// cwd, or not at all for a session on another machine. The click and the
    /// pane menu both read this, so the menu never offers what a click refuses.
    pub fn classify_link(&self, link: &str) -> LinkTarget {
        match &self.cwd {
            Some(PaneCwd::Remote(_)) => link_target::classify_remote(link),
            Some(PaneCwd::Local(cwd)) => link_target::classify(link, Some(cwd)),
            None => link_target::classify(link, None),
        }
    }

    /// [`Self::classify_link`] for a tool's resource-link URI — a file by
    /// definition, and `mime` is the tool's declared type.
    pub fn classify_resource(&self, uri: &str, mime: Option<&str>) -> LinkTarget {
        match &self.cwd {
            Some(PaneCwd::Remote(_)) => link_target::classify_remote(uri),
            Some(PaneCwd::Local(cwd)) => link_target::classify_resource(uri, mime, Some(cwd)),
            None => link_target::classify_resource(uri, mime, None),
        }
    }
}

impl Focusable for AgentChatView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for AgentChatView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(any(test, feature = "test-support"))]
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
