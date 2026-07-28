//! Self-owned ACP chat pane entity.
//!
//! Like `TerminalView`, this pane owns its conversation model plus UI/runtime
//! state, renders itself, and notifies itself. Workspace ops create the pane,
//! own the ACP pump/error pipeline, and pass config-derived inputs into
//! `apply_event` so the view does not mirror workspace config.
//!
//! ## SAFETY(MVU): self-contained pane entity
//!
//! This entity owns and mutates its own model, like `TerminalView`; it is not a
//! Workspace-owned model written through a one-way op. The MVU rule applies to
//! Workspace state; self-notifying pane entities are the sanctioned CLAUDE.md
//! rule #10 exception also used by `TerminalView` and `ToastLayer`.

use std::collections::{HashMap, HashSet};

use daruda_acp::{
    AcpSessionHandle, ChatItem, ConnectPhase, PlanEntryView, SessionCapabilitiesView, UsageView,
};
use daruda_store::project::PaneCwd;
use gpui::{
    AnyWindowHandle, App, Bounds, Context, FocusHandle, Focusable, FollowMode, ListAlignment,
    ListState, Pixels, ScrollHandle, Subscription, Task, Window, prelude::*, px,
};

use super::fold::FoldState;
use super::render::{DiffEditors, DiffStats, MermaidImages, ToolImages};
use super::rows::RenderRow;
use super::session_config::SessionConfig;
use super::telegram_ops::{FirstResponseOutcome, FirstResponseWatch};
use crate::workspace::main_area::pane_tree::PaneId;

/// How long a background subagent's run stays "active" after its last child
/// tool event, bridging the gaps between a subagent's sequential child tool
/// calls (observed ~4s). The parent Task's own status completes early and
/// there is no clean terminal signal, so we treat a subagent as still running
/// until it has been quiet for this long.
const SUBAGENT_QUIESCENCE: std::time::Duration = std::time::Duration::from_secs(8);

/// Idle gap after the last post-turn (background) update before its accumulated
/// assistant text is relayed to Telegram as a follow-up. Long enough to coalesce
/// the streamed chunks (~700ms observed), short enough to feel prompt.
pub(in crate::workspace) const POST_TURN_QUIESCENCE: std::time::Duration =
    std::time::Duration::from_millis(1500);

/// Assistant-text items whose index is at or beyond `relayed` (the count already
/// covered by a prior relay), joined by a blank line. `None` when nothing new or
/// the delta is whitespace-only. Also returns the new covered count so the caller
/// can advance its marker. Counts whole items, not chars: a post-turn follow-up
/// is a fresh `AssistantText` item (the observed Claude background-completion
/// shape), so item-count marking is exact and dedup-free.
fn post_turn_delta(items: &[ChatItem], relayed: usize) -> Option<(String, usize)> {
    let texts: Vec<&str> = items
        .iter()
        .filter_map(|it| match it {
            ChatItem::AssistantText { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    if texts.len() <= relayed {
        return None;
    }
    let delta = texts[relayed..].join("\n\n");
    let delta = delta.trim();
    if delta.is_empty() {
        return None;
    }
    Some((delta.to_string(), texts.len()))
}

/// Debug gate for agent-chat list-measurement tracing (`trace_remeasure`).
/// Reads `DARUDA_DEBUG_AGENT_LIST` once and caches it. Off by default so the
/// trace is silent in normal builds; set the env var to capture the remeasure
/// timeline when the intermittent oversized-gap bug recurs.
fn debug_list_trace_enabled() -> bool {
    use std::sync::OnceLock;
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("DARUDA_DEBUG_AGENT_LIST").is_some())
}

/// A user-visible milestone of the one-time Node.js runtime provisioning shown
/// in the connecting banner. A closed set of only the slow, user-facing phases
/// (the instant "found system node" / "cache probe" milestones never surface),
/// so the banner text maps at render instead of being stored in the model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::workspace) enum RuntimePrepPhase {
    /// Downloading the Node.js archive.
    Downloading,
    /// Verifying the downloaded archive's checksum.
    Verifying,
    /// Extracting the archive.
    Extracting,
}

/// Connection lifecycle of an [`AgentChatView`]'s ACP session. Declared as an
/// enum so the connecting / live / failed states are distinct variants rather
/// than a `bool` + companion field; the live `Error` arm carries the failure
/// message it renders.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::workspace) enum AgentSessionStatus {
    /// Restored (or freshly created) but no session has been started — the ACP
    /// session is not persisted, so a restored pane stays dormant (no agent
    /// process) until the user focuses it. `focus_pane` then transitions it to
    /// `Connecting` via `maybe_connect_agent_chat`.
    Idle,
    /// Provisioning the Node.js runtime the adapter needs, before the adapter is
    /// even spawned. Only reached on a machine without a usable system Node.js,
    /// where a one-time managed download runs. Carries the milestone (not its
    /// localized text) so the banner copy is derived at render.
    PreparingRuntime(RuntimePrepPhase),
    /// The ACP adapter has been asked to start but the session is not yet ready
    /// for prompts (handshake + `session/new` in flight), and no milestone
    /// has been reported yet — a fresh connect starts here, before the
    /// adapter process has even spoken ACP. Upgrades in-place to
    /// `Handshaking` the moment the first `AcpEvent::ConnectProgress` lands.
    Connecting,
    /// Refines `Connecting` once the connection task reports which step of
    /// the handshake (`initialize` / `session/new` / `session/load` /
    /// `set_mode`) is currently in flight, bounded by
    /// `daruda_acp::session::CONNECT_HANDSHAKE_TIMEOUT` so this can never
    /// hang forever — a timeout surfaces as `Error` like any other connect
    /// failure.
    Handshaking(ConnectPhase),
    /// `initialize` + `session/new` succeeded — the session accepts prompts and
    /// the event pump is folding updates into `items`.
    Connected,
    /// The connection or protocol failed; the message is surfaced both here
    /// (status line) and through the error pipeline.
    Error(String),
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
}

impl Turn {
    /// True while a prompt turn is on the wire (Send ↔ Stop affordance, badge).
    fn is_in_flight(&self) -> bool {
        matches!(self, Turn::InFlight { .. })
    }
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

/// Stable identity of a queued prompt, minted per pane by
/// [`AgentChatView::enqueue_prompt`]. Lets the queued-prompt strip target a
/// specific entry for removal without depending on its position (which shifts
/// as earlier entries drain or are removed). Runtime-only; never persisted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(in crate::workspace) struct PromptId(u64);

impl std::fmt::Display for PromptId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Where a prompt came from — the bottom-dock composer, or a phone-relayed
/// Telegram reply. Only the latter arms [`AgentChatView::telegram_first_response_watch`]
/// on dispatch; an in-app prompt never does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::workspace) enum PromptOrigin {
    InApp,
    Telegram,
}

/// Whether [`AgentChatView::send_prompt_text_for_telegram`] put the prompt on
/// the wire immediately or only queued it behind an in-flight turn — the
/// Telegram relay needs this to decide whether a "queued" notice is owed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::workspace) enum PromptDispatch {
    SentNow,
    Queued,
}

/// A Telegram-origin turn's first-response watch side effect, returned by
/// [`AgentChatView::apply_event`] so the Workspace event pump can relay exactly
/// one phone-visible acknowledgement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::workspace) enum TelegramFirstResponseEffect {
    None,
    Relay(FirstResponseOutcome),
    Fallback,
}

/// Which Telegram first-response-watch action `apply_event` owes its tail,
/// decided once per event arm (like the `touched_tool` / `touched_text` /
/// `turn_settled` flags already in that function) rather than each arm
/// calling a `*_telegram_first_response_watch` method itself. The three
/// underlying methods (`clear_telegram_first_response_watch` /
/// `take_telegram_first_response` / `finish_telegram_first_response_watch`)
/// then each have exactly one call site in `apply_event` instead of being
/// duplicated at every arm that needs them, so a future arm added to the
/// match can't silently forget one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum TelegramWatchAction {
    #[default]
    None,
    /// A permission card interrupted the turn, or a Stop's cancel-ack
    /// landed — no relay is owed either way.
    Clear,
    /// New conversation content arrived — check whether it resolves the
    /// armed watch.
    CheckUpdate,
    /// The turn or session settled — resolve the watch to its terminal
    /// effect.
    Finish,
}

/// A prompt the user submitted while a turn was in flight (or before the
/// session connected), held in the queue until it can be dispatched. Unlike
/// the send-now path, a queued prompt is NOT echoed into the transcript — it
/// lives only here (and in the queued-prompt strip) until
/// [`AgentChatView::pump_pending_prompt`] drains it, at which point it is
/// echoed as a [`ChatItem::UserText`] at send time.
#[derive(Debug, Clone)]
pub(in crate::workspace) struct QueuedPrompt {
    pub id: PromptId,
    pub text: String,
    pub origin: PromptOrigin,
}

/// What the Escape shortcut resolved to for a focused Agent chat pane. Returned
/// by [`AgentChatView::handle_escape`] so the `Workspace` shim fires the
/// settle-edge completion only when a turn was actually cancelled.
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

/// The prompt-send sequencing for one Agent chat pane: the buffered/parked
/// queues and the in-flight-turn tracking that gates draining them. One
/// prompt turn runs at a time, so these four fields are always read and
/// mutated as a unit by the queue ops (`enqueue_prompt`, `resume_queue`,
/// `remove_queued`, `clear_queue`, `begin_edit`, `cancel_edit`,
/// `pump_pending_prompt`, `drain_next_queued_prompt`) and by `send_prompt_text`
/// / `cancel_turn` / `handle_escape`. Grouping them gives `AgentChatView` one
/// field for this concern instead of four, and keeps `turn` — already
/// module-private to `view.rs`, enforced by `scripts/lint-agent-activity.sh`
/// — private the same way inside this struct rather than as a bare
/// `AgentChatView` field.
#[derive(Default)]
pub(in crate::workspace) struct PromptQueue {
    /// Prompts buffered because they could not be sent yet, in submission
    /// order. A prompt is buffered when the session is not ready to prompt
    /// (`status != Connected` or `handle` is still `None` — the lazy connect
    /// happens on first focus) **or** a turn is already in flight. The session
    /// runs one turn at a time, so exactly one buffered prompt is drained per
    /// turn-completion by [`AgentChatView::pump_pending_prompt`]: the
    /// `Connected` event pumps the first, and each `TurnEnded` pumps the next,
    /// keeping only one turn tracked at a time.
    /// Cleared on a connect failure / terminal `Error` (there is no reconnect
    /// path, so buffered prompts can never be delivered); never serialized.
    ///
    /// A queued prompt is NOT echoed into `items` — it lives only here (surfaced
    /// by the bottom-dock queued-prompt strip, keyed by [`PromptId`]) until it
    /// drains, at which point `pump_pending_prompt` echoes it as a `UserText` at
    /// send time. The user can remove an individual entry
    /// ([`AgentChatView::remove_queued`]) or clear the whole queue
    /// ([`AgentChatView::clear_queue`]) from the strip.
    pub(in crate::workspace) pending_prompts: Vec<QueuedPrompt>,
    /// Prompts parked by a Stop (`cancel_turn`) while a queue was buffered
    /// behind the cancelled turn. Unlike [`Self::pending_prompts`], these are
    /// NOT auto-drained: they live outside the live queue so the cancel-ack /
    /// `TurnEnded` pump never touches them. The user resumes them explicitly
    /// (the queue strip's Resume button → [`AgentChatView::resume_queue`],
    /// which moves them back to the front of the live queue) or discards them
    /// (a second Esc / the strip's clear-all → [`AgentChatView::clear_queue`]).
    /// A prompt typed *after* the Stop sends immediately as a fresh turn,
    /// ahead of these. Runtime-only; never serialized; wiped on a session
    /// teardown / reset.
    pub(in crate::workspace) paused_prompts: Vec<QueuedPrompt>,
    /// Monotonic counter minting the next [`PromptId`] for a queued prompt.
    /// Never read outside `view.rs`. Runtime-only; never serialized.
    next_prompt_id: u64,
    /// The queued prompt currently being edited in the composer, if any. Set by
    /// [`AgentChatView::begin_edit`] (from the strip's ✎ button or ↑ in an
    /// empty composer) and cleared on send ([`AgentChatView::send_prompt_text`]
    /// replaces the slot in place), cancel ([`AgentChatView::cancel_edit`]),
    /// drain (the edit target pumped onto the wire), or any queue reset. While
    /// `Some`, the strip marks that row as the edit target and a send replaces
    /// its text rather than enqueuing a new prompt. Runtime-only; never
    /// serialized.
    pub(in crate::workspace) editing_prompt: Option<PromptId>,
    /// Whether a prompt turn is in flight (between submit and the matching
    /// `TurnEnded`). Drives the input affordance (Send ↔ Stop), disables
    /// re-submit while the agent is busy, and carries the turn's start instant
    /// (runtime-only, never persisted) for the elapsed-time display.
    ///
    /// Confined to this module: `Turn` is the prompt-queue sequencing state, not
    /// the pane's activity signal. Production code outside `view.rs` must never
    /// read it — "is the pane working / did it just finish" decisions go through
    /// [`AgentChatView::is_busy`] / [`AgentChatView::activity_state`] /
    /// [`AgentChatView::activity_elapsed`], and completion fires only via
    /// `fire_activity_completion` at the busy→idle edge. Tests reach it through
    /// the `#[cfg(test)]` hooks on [`AgentChatView`].
    turn: Turn,
}

/// Async-built rendering artifacts derived from the conversation: read-only
/// diff editors for tool-call file edits, rasterized mermaid diagrams, and
/// decoded tool-output images. Each cache is filled by its own reconciler in
/// `reconcile.rs` (`reconcile_diff_editors` / `reconcile_mermaid` /
/// `reconcile_tool_images`), gated in `apply_event` on the content that
/// feeds it changing; `render/` only ever reads them by reference (never
/// constructs or clears one). Grouped into one type so `AgentChatView`
/// carries a single "derived render asset" field instead of seven, and a
/// session reset (`teardown_transient_session_state`) wipes them with one
/// call ([`Self::clear`]) instead of seven individually-ordered lines.
#[derive(Default)]
pub(in crate::workspace) struct AssetCache {
    /// Read-only diff editor entities for tool-call file modifications, keyed by
    /// `"{tool_call_id}#{diff_index}"` (one editor per file in a tool call).
    /// Built (and rebuilt on content change) by `reconcile_diff_editors` — the
    /// same diff-through-editor renderer the File viewer uses. Entities are
    /// created in the reconcile op, never in `render` (which only embeds them).
    pub(in crate::workspace) diff_editors: DiffEditors,
    /// `diff_source_fingerprint` of the diff each `diff_editors` entry was
    /// built from, same keys as `diff_editors`. Lets `reconcile_diff_editors`
    /// detect a `ToolCallUpdate` that replaced a tool call's diffs (streaming
    /// write/edit growing from a partial snapshot) and rebuild instead of
    /// leaving the cached editor frozen on stale content.
    pub(in crate::workspace) diff_editor_sources: HashMap<String, u64>,
    /// Added / removed line counts per tool-call diff, keyed by the same
    /// `"{tool_call_id}#{diff_index}"` as `diff_editors`. Runtime cache —
    /// never serialized (the conversation itself is not persisted, only `cwd`).
    pub(in crate::workspace) diff_stats: DiffStats,
    /// Rendered mermaid diagrams keyed by fence-source hash, filled async by
    /// `reconcile_mermaid`. Stored as a GPU-ready image (converted once at
    /// insert) so each render clones the same image — gpui's texture cache
    /// hits instead of re-uploading the bitmap every frame. Shared
    /// `Arc<Mutex<…>>` so the `code_block_render` hook reads the live cache.
    /// Runtime cache; never serialized.
    pub(in crate::workspace) mermaid_images: MermaidImages,
    /// Source hashes with a rasterization currently spawned, so
    /// `reconcile_mermaid` doesn't re-spawn the same diagram while it is still
    /// rendering on the background executor. Runtime cache; never serialized.
    pub(in crate::workspace) mermaid_inflight: HashSet<u64>,
    /// Decoded tool-output images keyed by base64-content hash
    /// (`tool_image_key`). `Some` = decoded & GPU-ready; `None` = decode failed
    /// (so a failure label renders instead and the key is never re-spawned).
    /// Filled async by `reconcile_tool_images`. Shared `Arc<Mutex<…>>` for the
    /// same reason as `mermaid_images`. Runtime cache; never serialized.
    pub(in crate::workspace) tool_images: ToolImages,
    /// Content hashes with a decode currently spawned, so
    /// `reconcile_tool_images` doesn't re-spawn the same image while it is
    /// still decoding on the background executor. Runtime cache; never
    /// serialized.
    pub(in crate::workspace) tool_image_inflight: HashSet<u64>,
}

impl AssetCache {
    /// Wipe every cache in place — same `Arc`s, contents cleared — rather
    /// than `*self = Self::default()`, so a live clone of `mermaid_images` /
    /// `tool_images` (the markdown code-block hook clones the `Arc` fresh on
    /// every render; see `render/blocks.rs`) stays attached to the same
    /// underlying map instead of pointing at a now-orphaned one.
    fn clear(&mut self) {
        self.diff_editors.clear();
        self.diff_editor_sources.clear();
        self.diff_stats.clear();
        self.mermaid_inflight.clear();
        if let Ok(mut m) = self.mermaid_images.lock() {
            m.clear();
        }
        self.tool_image_inflight.clear();
        if let Ok(mut m) = self.tool_images.lock() {
            m.clear();
        }
    }
}

/// The pane's activity bookkeeping: whether it is busy right now, since when,
/// and what completion is owed once it settles. Read and mutated together by
/// `AgentChatView::{is_busy, reconcile_activity, reconcile_post_turn,
/// take_pending_post_turn, activity_elapsed, maybe_active}` — those stay
/// methods on `AgentChatView` (not `Self`) because they also need `turn` and
/// `items`, which live outside this struct. Grouped here so that cohesive
/// read/write pattern has one field on `AgentChatView` instead of seven.
#[derive(Default)]
pub(in crate::workspace) struct ActivityTracker {
    /// Per-subagent last-activity timestamps, keyed by the parent tool id every
    /// child tool stamps in its `parent_tool_id`. Bumped in `apply_event` each
    /// time one of a subagent's child tools produces a tool-call event, and read
    /// by [`daruda_acp::subagent_activity`] to hold the subagent's run "active"
    /// across the gaps between its sequential child calls (see
    /// [`SUBAGENT_QUIESCENCE`]). Runtime-only; never serialized.
    pub(in crate::workspace) subagent_last_activity: HashMap<String, std::time::Instant>,
    /// Wall-clock start of the current busy activity span (turn + any trailing
    /// subagents), set on the idle→busy edge and cleared on busy→idle by
    /// `AgentChatView::reconcile_activity`. Anchors the working-indicator
    /// elapsed timer across the whole span rather than just the foreground
    /// turn. Runtime-only; never serialized.
    pub(in crate::workspace) activity_started_at: Option<std::time::Instant>,
    /// Whether the pane was busy at the last `AgentChatView::reconcile_activity`
    /// tick — the edge-detection memory that turns the `is_busy` level signal
    /// into idle→busy / busy→idle transitions. Runtime-only; never serialized.
    pub(in crate::workspace) was_busy: bool,
    /// The outcome captured when the turn/session ended, held until the pane
    /// actually settles busy→idle (which may trail `end_turn` while subagents
    /// finish). Taken and returned by `AgentChatView::reconcile_activity` on
    /// the busy→idle edge so the completion signal fires at the true settle
    /// point. Runtime-only; never serialized.
    pub(in crate::workspace) pending_completion: Option<TurnOutcome>,
    /// Count of `AssistantText` items already delivered to Telegram (by the turn
    /// completion relay or a prior post-turn relay). Baseline for the post-turn
    /// delta. Snapped at every `settle_turn`.
    pub(in crate::workspace) post_turn_relayed_assistant_texts: usize,
    /// Set to `now` whenever a post-turn (no in-flight turn, not restoring) update
    /// touches text/tools; cleared when the follow-up is relayed. Drives the
    /// quiescence settle that `reconcile_post_turn` detects on the pulse tick.
    pub(in crate::workspace) post_turn_dirty_at: Option<std::time::Instant>,
    /// True between a Stop and its `cancelled` `TurnEnded` ack. Stop settles the
    /// turn locally (responsive + hung-safe) and fires `Stopped`, but the cancel
    /// is still outstanding on the wire. While set, `send_prompt_text` buffers a
    /// re-prompt into `pending_prompts` (client-side, so a second Stop can clear
    /// it) instead of racing it onto the wire ahead of the cancel; the ack then
    /// clears this flag and `pump_pending_prompt` drains the buffered prompt as a
    /// fresh turn. Cleared by the first `TurnEnded`/`Error` after the Stop, or on
    /// session reset. Runtime-only.
    pub(in crate::workspace) cancel_in_flight: bool,
}

/// Native ACP (Agent Client Protocol) chat pane, owned as `Entity<AgentChatView>`.
///
/// Owns the live session: the [`daruda_acp::AcpSessionHandle`] the workspace
/// ops drive (`send_prompt` / `cancel` / `respond_permission`). Dropping the
/// view (pane close) drops the handle — closing the command channel so the
/// connection task exits — and drops the event-pump task, ending its loop. No
/// explicit teardown is needed.
pub(in crate::workspace) struct AgentChatView {
    /// The owning pane's id. Used to key element ids (stable across two open
    /// chat panes) and the log dedup tags, and so the Workspace pump can look
    /// this view back up by pane.
    pub(in crate::workspace) pane_id: PaneId,
    /// The workspace window this view renders in. Captured at construction so
    /// diff-editor / `InputState` creation can re-enter the *workspace* window
    /// (`cx.update_window`) — after extraction `cx.entity_id()` is this view,
    /// not the Workspace, so `WindowRegistry::handle_for_workspace` no longer
    /// resolves it.
    pub(in crate::workspace) window_handle: AnyWindowHandle,
    /// Pane-level focus handle for `Cmd+W` close routing. The view's `render`
    /// tracks it (like `TerminalView`), so `wrapper_focus_handle` returns
    /// `None` for this content kind.
    pub(in crate::workspace) focus_handle: FocusHandle,
    /// Lane working directory the agent session is rooted at. `None` when the
    /// pane was opened without a resolvable lane cwd; `Some(PaneCwd::Remote)`
    /// for a session rooted on a different machine. Local-only consumers
    /// must go through `PaneCwd::as_local` / `into_local` — see that type's
    /// docs.
    pub(in crate::workspace) cwd: Option<PaneCwd>,
    /// The agent this pane runs under — an id from the config `[[agents]]`
    /// catalog. Resolved to a launch command at connect time. Set at
    /// create/restore (default = catalog[0]) and persisted so the pane comes
    /// back under the same agent, whose session id is then resumable.
    pub(in crate::workspace) agent_id: String,
    /// Display name for `agent_id`, copied from the config catalog at pane
    /// creation/restore and refreshed on config reload. Used as the activity
    /// bar title fallback before the session reports its own title.
    pub(in crate::workspace) agent_name: String,
    /// Connection lifecycle state. Drives the status line + input/cancel
    /// affordance.
    pub(in crate::workspace) status: AgentSessionStatus,
    /// Persisted ACP session id, `Some` once a live session has been
    /// established (set by the `Connected` event) or seeded from persisted
    /// state on a restored pane. When present, the lazy connect resumes it
    /// via `session/load` instead of starting a fresh session. Persisted so
    /// the conversation comes back across restarts.
    pub(in crate::workspace) session_id: Option<String>,
    /// ACP session-mode id this session was last known to be in, mirrored
    /// from `session_config.modes.current` on every `Connected` / `ModeChanged`
    /// (unlike `session_config`, never cleared by `/clear` or a retry's
    /// teardown) and persisted so a resumed session can have it reapplied via
    /// `session/set_mode` on the next connect.
    ///
    /// WORKAROUND: `session/load`'s response can carry the resumed session's
    /// real mode, but the `claude-agent-acp` adapter recomputes it from
    /// `settings.json` on every process launch instead of the session's
    /// actual last mode — see `daruda_acp::session`'s `restore_mode` param,
    /// which this field feeds on connect.
    pub(in crate::workspace) last_known_mode_id: Option<String>,
    /// `true` while a resume (`session/load`) is replaying its history: the
    /// adapter streams many `session/update`s before the `Connected` reply.
    /// While set, `apply_event` accumulates items but skips the per-event
    /// `rebuild_rows()` + `cx.notify()` (O(n²) over the replay); the single
    /// catch-up runs when `Connected` fires and clears this. Transient.
    pub(in crate::workspace) restoring: bool,
    /// Conversation render model, in arrival order. The event pump
    /// appends/folds into this; the renderer reads it.
    //
    // INVARIANT: `FoldKey::Assistant`/`Thinking` and the per-message markdown
    // selection ids (`("agent-chat-md-*", ix)`) are keyed by item index; this
    // is valid only because `items` is append-only (only the tail mutates in
    // place; no item is removed or reordered). Any future feature that removes
    // or reorders items MUST clear `FoldState` (its index-keyed overrides would
    // otherwise mis-target) and would also break markdown selection identity.
    pub(in crate::workspace) items: Vec<ChatItem>,
    /// Live ACP session handle. `None` until `connect_session` resolves; stays
    /// `None` on a connect failure, and is cleared back to `None` on a terminal
    /// [`AcpEvent::Error`] (the connection task has ended, so the handle is dead).
    /// A per-turn [`AcpEvent::TurnFailed`] does NOT clear it — the connection is
    /// still alive there. Dropping it (pane close) closes the command channel and
    /// shuts the connection task down.
    pub(in crate::workspace) handle: Option<AcpSessionHandle>,
    /// The buffered/parked queue of prompts not yet on the wire, plus the
    /// in-flight-turn sequencing that gates draining it — see [`PromptQueue`].
    pub(in crate::workspace) queue: PromptQueue,
    /// GPUI-side pump that drains the `AcpEvent` receiver and folds events into
    /// `items` / `status`. Dropped with the view, ending the loop.
    pub(in crate::workspace) _event_pump: Option<Task<()>>,
    /// Request ids of the permission cards still awaiting a host decision. A
    /// fast index over `items` — mirrors `{card.id : card.resolved.is_none()}`
    /// and is updated in lockstep with the cards at every site that touches
    /// them: `PermissionRequested` inserts (pushes card + id), `respond_permission`
    /// removes (resolves card + drops id), `cancel_pending_permission` drains
    /// (marks all cards + empties the set), and `teardown_transient_session_state`
    /// resets (clears `items` + the set together). Unlike a single `Option<u64>`
    /// it holds *every* outstanding request, so parallel tool-call permissions
    /// each resolve to their own park instead of the newest clobbering the rest.
    pub(in crate::workspace) pending_permissions: HashSet<u64>,
    /// Set the instant a Telegram-origin prompt (see [`PromptOrigin::Telegram`])
    /// actually dispatches — immediately in [`Self::send_prompt_text_inner`], or
    /// later in [`Self::drain_next_queued_prompt`] if it had to wait behind an
    /// in-flight turn. `None` means no phone-triggered turn is currently being
    /// watched: either none was ever armed, or it already resolved (a
    /// qualifying chat item landed), was consumed by a permission request, or
    /// the turn settled. Cleared by whichever of those happens first; the
    /// `agent_chat_connect_ops.rs` event pump and the periodic Telegram flush pump
    /// (`Workspace::flush_telegram_first_response_fallbacks`) are the only
    /// consumers. Runtime-only; never serialized.
    telegram_first_response_watch: Option<FirstResponseWatch>,
    /// Diff editors, mermaid diagrams, and tool-output images built async
    /// from the conversation's content — see [`AssetCache`].
    pub(in crate::workspace) assets: AssetCache,
    /// The pane's activity bookkeeping — see [`ActivityTracker`]. Read and
    /// mutated together by `is_busy` / `reconcile_activity` /
    /// `reconcile_post_turn` / `activity_elapsed`, which also need `queue.turn`
    /// and `items` (neither moved here) so those methods stay `impl
    /// AgentChatView` rather than `impl ActivityTracker`.
    pub(in crate::workspace) activity: ActivityTracker,
    /// Per-conversation fold state — which blocks the user has explicitly
    /// expanded / collapsed. Transient / session-only; never serialized.
    pub(in crate::workspace) fold: FoldState,
    /// Virtualized conversation list state (gpui `list`). Renders only the
    /// visible items + a small overdraw, so the per-scroll / per-frame draw cost
    /// is bounded by what's on screen rather than the whole conversation. Set to
    /// [`FollowMode::Tail`] so it auto-scrolls with streaming output while the
    /// user is at the bottom and re-engages when they scroll back down (this
    /// replaces the old `stick_to_bottom` flag). Item count is kept in sync with
    /// `items` via [`Self::sync_list_after`]. Transient / session-only.
    pub(in crate::workspace) list_state: ListState,
    /// Render-row projection of `items` under `fold` (turns / tool groups /
    /// items, with `hidden` flags), recomputed by [`Self::rebuild_rows`] on
    /// every items/fold change. The virtualized `list` indexes over this, and
    /// the render processor reads `rows[ix]`. Derived cache — single rebuild
    /// site. Transient / session-only.
    pub(in crate::workspace) rows: Vec<RenderRow>,
    /// What the connected agent advertises about this session: its modes,
    /// select config options, and slash commands. Established at `Connected`,
    /// each part replaced wholesale by its own event, all cleared together on
    /// teardown. Runtime-only; never serialized.
    pub(in crate::workspace) session_config: SessionConfig,
    /// Which optional session methods the agent advertised at connect
    /// (`session/load` / `list` / `resume` / `close`). The active consumer today
    /// is resume gating, applied in the connection core (`resolve_resume` in
    /// `daruda_acp`) before this event — a resume against a non-`load` agent
    /// downgrades to a fresh session. Held here as the app-layer source of truth
    /// for capability-gated UI (resume / session-list affordances) added later,
    /// mirroring `available_commands`. Runtime-only; never serialized (re-read
    /// each connect from `initialize`). Default = baseline agent (nothing extra).
    pub(in crate::workspace) session_capabilities: SessionCapabilitiesView,
    /// Live context-window / cost accounting from the agent's `UsageUpdate`
    /// notifications (`AcpEvent::UsageChanged`). `None` until the agent reports
    /// usage; drives the context meter in the chat chrome. Distinct from the
    /// cumulative Usage tab (this is the current context fill). Runtime-only;
    /// never serialized. Cleared on a fresh session.
    pub(in crate::workspace) session_usage: Option<UsageView>,
    /// The agent's live execution plan (`PlanChanged`); full-replaced each update.
    /// Runtime-only; never serialized.
    pub(in crate::workspace) plan: Vec<PlanEntryView>,
    /// Agent-provided session title (`SessionInfoChanged`); `None` = fallback label.
    pub(in crate::workspace) session_title: Option<String>,
    /// Agent-provided last-activity timestamp (`SessionInfoChanged`, ISO 8601);
    /// `None` = unknown. Shown as a tooltip on the activity-bar title. Runtime-only.
    pub(in crate::workspace) session_updated_at: Option<String>,
    /// Whether the bottom plan region is collapsed to its header. Transient /
    /// session-only; defaults to `false` (expanded) so a fresh plan shows its
    /// checklist. Toggled by the header click via [`Self::toggle_plan_collapsed`].
    pub(in crate::workspace) plan_collapsed: bool,
    /// Scroll position of the expanded plan checklist (a plain `Div`, not a
    /// `list()`), backing the region's 4px daruda thumb overlay. Runtime-only;
    /// never serialized.
    pub(in crate::workspace) plan_scroll: ScrollHandle,
    /// Window-space bounds of the scrolling list viewport, captured each paint
    /// by the container's `canvas` (the sanctioned layout-geometry cache; see
    /// CLAUDE §3). Read by the drag-selection autoscroll poll to decide when the
    /// cursor has left the viewport. `None` until the first paint. Transient.
    pub(in crate::workspace) list_bounds: Option<Bounds<Pixels>>,
    /// Drag-selection autoscroll poll task (mirrors `TerminalView::autoscroll_task`).
    /// Spawned on a left mouse-down in the list, self-terminates when the drag
    /// ends. Replace-and-cancel on each new drag; `None` when idle. Transient.
    pub(in crate::workspace) autoscroll_task: Option<Task<()>>,
    /// App-owned "a drag-selection is in progress" signal (mirrors the
    /// terminal's `MouseDragState::TextSelection`). Set on the always-painted
    /// list container's mouse-down, cleared through the single `end_selection_drag`
    /// end point. This is the *primary* autoscroll-loop termination authority:
    /// it is independent of the selected block's paint lifetime, so the poll
    /// still stops on mouse-release even when the block unmounts from the
    /// virtualized list mid-drag (which would leave the vendored `TextView`'s
    /// paint-registered mouse-up / outside-clear handlers unable to null the
    /// selection slot). Transient / session-only.
    pub(in crate::workspace) selection_drag_active: bool,
    /// Inactive-pane dim amount in `[0.0, 1.0]`: `0.0` = focused / full color,
    /// `> 0.0` = unfocused leaf of a split, blended toward gray by this factor
    /// (see [`Self::dim`]). The single write site is `Workspace::refresh_pane_dimming`
    /// (MVU one-way flow), mirroring `TerminalView::set_dim_amount`. Transient /
    /// session-only.
    pub(in crate::workspace) dim_amount: f32,
    /// Observes the host `DarudaTheme` global so a light/dark toggle
    /// re-rasterizes cached mermaid diagrams for the new appearance. The cache
    /// is keyed by `(source, dark)` (see `mermaid_key`), so without this a
    /// toggle would leave the old-appearance raster missing until the next ACP
    /// event triggered a reconcile. Held for the view's lifetime; dropped with it.
    _theme_observer: Subscription,
    /// Test-only render counter, asserted by the cache-scoping regression test
    /// (`notify_rerenders_cached_agent_view`) to confirm a `cx.notify()` on this
    /// view actually re-renders it.
    #[cfg(test)]
    pub(in crate::workspace) render_count: std::cell::Cell<u32>,
}

mod activity_ops;
mod apply_event;
mod queue_ops;
mod session_ops;

impl AgentChatView {
    /// Build a fresh view. The session is *not* started here — the Workspace
    /// connects it lazily on first focus (`maybe_connect_agent_chat`), so cold
    /// restore doesn't spin up an agent process per pane. `status` is decided
    /// by the caller (Idle when a cwd is present, Error otherwise).
    #[allow(clippy::too_many_arguments)] // Restore/create seed values — bundling them into a struct only wraps callers.
    pub(in crate::workspace) fn new(
        pane_id: PaneId,
        window_handle: AnyWindowHandle,
        cwd: Option<PaneCwd>,
        status: AgentSessionStatus,
        session_id: Option<String>,
        mode_id: Option<String>,
        agent_id: String,
        agent_name: String,
        title: Option<String>,
        cx: &mut Context<Self>,
    ) -> Self {
        // Re-rasterize mermaid diagrams for the new appearance when the host
        // theme toggles. `apply_ui_theme` replaces the `DarudaTheme` global, so
        // this fires on every light/dark swap; `reconcile_mermaid` then fills the
        // `(source, dark)` cache keys the render hook now looks up.
        let theme_observer = cx.observe_global::<crate::ui::theme::DarudaTheme>(|this, cx| {
            let dark = Self::host_is_dark(cx);
            this.reconcile_mermaid(dark, cx);
            this.reconcile_tool_images(cx);
        });
        Self {
            pane_id,
            window_handle,
            focus_handle: cx.focus_handle(),
            cwd,
            status,
            session_id,
            last_known_mode_id: mode_id,
            agent_id,
            agent_name,
            // A restored pane connects lazily; the resume decision (and the
            // `restoring` flag that coalesces the replay) is made at connect
            // time by `maybe_connect_agent_chat`, not here.
            restoring: false,
            items: Vec::new(),
            handle: None,
            queue: PromptQueue::default(),
            _event_pump: None,
            pending_permissions: HashSet::new(),
            telegram_first_response_watch: None,
            activity: ActivityTracker::default(),
            assets: AssetCache::default(),
            fold: FoldState::default(),
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
            plan_scroll: ScrollHandle::new(),
            list_bounds: None,
            autoscroll_task: None,
            selection_drag_active: false,
            dim_amount: 0.0,
            _theme_observer: theme_observer,
            #[cfg(test)]
            render_count: std::cell::Cell::new(0),
        }
    }

    /// Set the inactive-pane dim (`0.0` = focused / full color, `> 0.0` =
    /// unfocused split leaf). Single write site: `Workspace::refresh_pane_dimming`
    /// (MVU one-way flow). No `cx.notify` here — the caller notifies only the
    /// views whose amount changed, mirroring `TerminalView::set_dim_amount`.
    pub(in crate::workspace) fn set_dim_amount(&mut self, amount: f32) {
        self.dim_amount = amount.clamp(0.0, 1.0);
    }

    /// Enter the `Connecting` status and repaint. Self-notifying so the event
    /// pump can't advance the connection state without dirtying the pane.
    pub(in crate::workspace) fn set_connecting(&mut self, cx: &mut Context<Self>) {
        self.status = AgentSessionStatus::Connecting;
        cx.notify();
    }

    /// Enter the `Error` status carrying `message` and repaint. Self-notifying
    /// so the event pump can't surface a failure without dirtying the pane.
    pub(in crate::workspace) fn set_error(&mut self, message: String, cx: &mut Context<Self>) {
        self.status = AgentSessionStatus::Error(message);
        cx.notify();
    }

    /// Enter the `PreparingRuntime` status at `phase` and repaint.
    /// Self-notifying so the runtime-progress drain can't advance the banner
    /// without dirtying the pane.
    pub(in crate::workspace) fn set_preparing(
        &mut self,
        phase: RuntimePrepPhase,
        cx: &mut Context<Self>,
    ) {
        self.status = AgentSessionStatus::PreparingRuntime(phase);
        cx.notify();
    }

    /// Enter the `Handshaking` status at `phase` and repaint. Self-notifying
    /// so the event pump can't advance the banner without dirtying the pane.
    /// Called from `apply_event` only while still `Connecting`/`Handshaking`
    /// (see that call site) — a `ConnectProgress` arriving after `Connected`
    /// or `Error` is stale and must not resurrect the connecting banner.
    pub(in crate::workspace) fn set_handshaking(
        &mut self,
        phase: ConnectPhase,
        cx: &mut Context<Self>,
    ) {
        self.status = AgentSessionStatus::Handshaking(phase);
        cx.notify();
    }

    /// Blend `c` toward gray by the current dim amount, alpha preserved. The
    /// render wraps every color it applies with this so an unfocused pane grays
    /// like an inactive terminal while keeping the window translucency (an
    /// overlay scrim would fill the see-through instead).
    pub(in crate::workspace) fn dim(&self, c: gpui::Hsla) -> gpui::Hsla {
        crate::ui::theme::dim_toward_gray(c, self.dim_amount)
    }

    /// Whether the host UI surface is currently dark, read from the theme
    /// global (defaults to dark before the global installs). Drives the mermaid
    /// diagram theme so edges stay visible against the host surface.
    fn host_is_dark(cx: &App) -> bool {
        cx.try_global::<crate::ui::theme::DarudaTheme>()
            .map(crate::ui::theme::DarudaTheme::is_dark)
            .unwrap_or(true)
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
mod tests;
