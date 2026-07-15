//! `AgentChatView` — the self-owned ACP chat pane entity.
//!
//! Mirrors `TerminalView`: a pane-content entity that owns its model
//! (conversation `items`, connection `status`, fold state, …) **and** its
//! UI/runtime state (diff editors, mermaid cache, scroll handle), renders
//! itself (`impl Render`), and `cx.notify()`s **itself** on every state
//! change. The pane walker embeds it via `AnyView::cached(..)`, so a scroll
//! or fold toggle dirties only this subtree — sibling docks / terminals keep
//! their cached paint instead of the whole window re-rendering (the
//! pre-extraction cost; see the module-level design note).
//!
//! ## What lives here vs. `agent_chat_ops`
//!
//! - **Here**: every op a render listener dispatches into (`toggle_fold`,
//!   `on_scroll`, `respond_permission`, `set_mode`, …) plus the per-event
//!   `apply_event` fold + the diff-editor / mermaid reconcilers. All operate
//!   on `self` and notify `self`.
//! - **`agent_chat_ops` (Workspace)**: pane/tab construction, the live ACP
//!   connection + event pump, and the bottom-dock prompt / cancel routing —
//!   the parts that need `Workspace` state (`syntax_theme`,
//!   `default_permission_mode`) or the error pipeline (`report_error`). The
//!   pump mutates this view via `view.update(.., cx.notify())`, so model
//!   changes still notify only this subtree.
//!
//! `apply_event` takes `syntax_theme` / `is_light` as parameters (passed by
//! the Workspace pump, which owns the config mirror) so the view holds no
//! mirrored copy — there is no second sync site to keep in step.
//!
//! ## SAFETY(MVU): self-contained pane entity
//!
//! This entity owns and mutates its own model, like `TerminalView` — it is
//! **not** a Workspace-owned model written through a one-way op. The MVU
//! "Views dispatch; only Workspace mutates Model" rule is about Workspace
//! state; a self-notifying pane entity (CLAUDE rule #10) is the sanctioned
//! exception, the same one `TerminalView` and `ToastLayer` take.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use daruda_acp::{
    AcpEvent, AcpSessionHandle, ChatItem, ConfigOptionView, ConnectPhase, ModeStateView,
    PermissionDecision, PermissionKindView, PlanEntryView, SessionCapabilitiesView, SlashCommand,
    UsageView, apply_update_with, cancel_pending_tools, finalize_streaming, permission_item,
    subagent_activity, touched_tool_id,
};
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::project::PaneCwd;
use gpui::{
    AnyWindowHandle, App, Bounds, Context, Entity, FocusHandle, Focusable, FollowMode,
    ListAlignment, ListState, Pixels, ScrollHandle, Subscription, Task, Window, prelude::*, px,
};

use super::agent_chat_helpers::{
    DiffStat, apply_info_field, cancel_pending_permission, collect_foldable_keys, fold_active,
    permission_card_mut,
};
use super::fold::{FoldKey, FoldState};
use super::rows::{RenderRow, RowKind, project};
use crate::surface::strings as s;
use crate::workspace::main_area::file_view_pane::render::CachedImage;
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Turn {
    Idle,
    InFlight { started_at: std::time::Instant },
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
    /// Prompts buffered because they could not be sent yet, in submission
    /// order. A prompt is buffered when the session is not connected (`handle`
    /// is still `None` — the lazy connect happens on first focus) **or** a turn
    /// is already in flight. The session runs one turn at a time, so exactly one
    /// buffered prompt is drained per turn-completion by
    /// [`Self::pump_pending_prompt`]: the connect site pumps the first, and each
    /// `TurnEnded` pumps the next, keeping only one turn tracked at a time.
    /// Cleared on a connect failure / terminal `Error` (there is no reconnect
    /// path, so buffered prompts can never be delivered); never serialized.
    pub(in crate::workspace) pending_prompts: Vec<String>,
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
    /// Whether a prompt turn is in flight (between submit and the matching
    /// `TurnEnded`). Drives the input affordance (Send ↔ Stop), disables
    /// re-submit while the agent is busy, and carries the turn's start instant
    /// (runtime-only, never persisted) for the elapsed-time display.
    ///
    /// Confined to this module: `Turn` is the prompt-queue sequencing state, not
    /// the pane's activity signal. Production code outside `view.rs` must never
    /// read it — "is the pane working / did it just finish" decisions go through
    /// [`Self::is_busy`] / [`Self::activity_state`] / [`Self::activity_elapsed`],
    /// and completion fires only via `fire_activity_completion` at the busy→idle
    /// edge. Tests reach it through the `#[cfg(test)]` hooks below.
    turn: Turn,
    /// Per-subagent last-activity timestamps, keyed by the parent tool id every
    /// child tool stamps in its `parent_tool_id`. Bumped in `apply_event` each
    /// time one of a subagent's child tools produces a tool-call event, and read
    /// by [`daruda_acp::subagent_activity`] to hold the subagent's run "active"
    /// across the gaps between its sequential child calls (see
    /// [`SUBAGENT_QUIESCENCE`]). Runtime-only; never serialized.
    pub(in crate::workspace) subagent_last_activity: HashMap<String, std::time::Instant>,
    /// Wall-clock start of the current busy activity span (turn + any trailing
    /// subagents), set on the idle→busy edge and cleared on busy→idle by
    /// [`Self::reconcile_activity`]. Anchors the working-indicator elapsed timer
    /// across the whole span rather than just the foreground turn. Runtime-only;
    /// never serialized.
    pub(in crate::workspace) activity_started_at: Option<std::time::Instant>,
    /// Whether the pane was busy at the last [`Self::reconcile_activity`] tick —
    /// the edge-detection memory that turns the `is_busy` level signal into
    /// idle→busy / busy→idle transitions. Runtime-only; never serialized.
    pub(in crate::workspace) was_busy: bool,
    /// The outcome captured when the turn/session ended, held until the pane
    /// actually settles busy→idle (which may trail `end_turn` while subagents
    /// finish). Taken and returned by [`Self::reconcile_activity`] on the
    /// busy→idle edge so the completion signal fires at the true settle point.
    /// Runtime-only; never serialized.
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
    /// Read-only diff editor entities for tool-call file modifications, keyed by
    /// `"{tool_call_id}#{diff_index}"` (one editor per file in a tool call).
    /// Built once per diff by `reconcile_diff_editors` — the same
    /// diff-through-editor renderer the File viewer uses. Entities are created
    /// in the reconcile op, never in `render` (which only embeds them).
    pub(in crate::workspace) diff_editors:
        HashMap<String, Entity<gpui_component::input::InputState>>,
    /// Added / removed line counts per tool-call diff, keyed by the same
    /// `"{tool_call_id}#{diff_index}"` as `diff_editors`. Runtime cache —
    /// never serialized (the conversation itself is not persisted, only `cwd`).
    pub(in crate::workspace) diff_stats: HashMap<String, DiffStat>,
    /// Rendered mermaid diagrams keyed by fence-source hash, filled async by
    /// `reconcile_mermaid`. Stored as a GPU-ready [`CachedImage`] (converted
    /// once at insert) so each render clones the same image — gpui's texture
    /// cache hits instead of re-uploading the bitmap every frame. Shared
    /// `Arc<Mutex<…>>` so the `code_block_render` hook reads the live cache.
    /// Runtime cache; never serialized.
    pub(in crate::workspace) mermaid_images: Arc<Mutex<HashMap<u64, CachedImage>>>,
    /// Source hashes with a rasterization currently spawned, so
    /// `reconcile_mermaid` doesn't re-spawn the same diagram while it is still
    /// rendering on the background executor. Runtime cache; never serialized.
    pub(in crate::workspace) mermaid_inflight: HashSet<u64>,
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
    /// Session-mode state advertised by the agent at `session/new` time and
    /// updated on `CurrentModeUpdate` notifications. `None` until the session
    /// connects, or when the agent does not advertise modes.
    pub(in crate::workspace) modes: Option<ModeStateView>,
    /// Select config options (model / effort / …) advertised by the agent at
    /// `session/new` time and replaced wholesale on `ConfigOptionsChanged`.
    /// Empty until the session connects or when the agent advertises none.
    /// `Mode`-category options are intentionally not kept here — mode is
    /// rendered through [`Self::modes`] and the existing mode chip.
    pub(in crate::workspace) config_options: Vec<ConfigOptionView>,
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
    /// Slash commands advertised by the agent via `AvailableCommandsChanged`.
    /// Runtime-only; never serialized. Consumed by the slash-command
    /// autocomplete provider (later task).
    pub(in crate::workspace) available_commands: Vec<SlashCommand>,
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

impl AgentChatView {
    /// Test-only hook: mark a prompt turn in flight (as `send_prompt_text` does).
    /// The `Turn` field is module-private so production code cannot read it; tests
    /// drive it through these sanctioned accessors instead of touching the field.
    #[cfg(test)]
    pub(in crate::workspace) fn set_turn_in_flight(&mut self) {
        self.turn = Turn::InFlight {
            started_at: std::time::Instant::now(),
        };
    }

    /// Test-only hook: return the turn to idle (as `settle_turn` does).
    #[cfg(test)]
    pub(in crate::workspace) fn set_turn_idle(&mut self) {
        self.turn = Turn::Idle;
    }

    /// Test-only hook: whether the turn is idle (no prompt in flight).
    #[cfg(test)]
    pub(in crate::workspace) fn turn_is_idle(&self) -> bool {
        !self.turn.is_in_flight()
    }

    /// Map the view's internal state to a [`daruda_claude::SessionStatus`] for
    /// the lane indicator aggregation. Returns `None` for states that should
    /// not contribute an indicator (dormant `Idle` — session not yet started —
    /// and `Error` — broken session).
    pub(in crate::workspace) fn to_session_status(&self) -> Option<daruda_claude::SessionStatus> {
        use daruda_claude::SessionStatus;
        match &self.status {
            AgentSessionStatus::Idle | AgentSessionStatus::Error(_) => None,
            // Runtime prep and the handshake are both connecting sub-phases —
            // same pulsing badge.
            AgentSessionStatus::PreparingRuntime(_)
            | AgentSessionStatus::Connecting
            | AgentSessionStatus::Handshaking(_) => Some(SessionStatus::Connecting),
            AgentSessionStatus::Connected => Some(match self.activity_state() {
                ActivityState::AwaitingPermission => SessionStatus::NeedsAttention,
                ActivityState::Working => SessionStatus::Working,
                ActivityState::Idle => SessionStatus::Idle,
            }),
        }
    }

    /// Whether the agent is doing work right now — the prompt turn is in flight
    /// **or** at least one background subagent is still inside its run span. A
    /// subagent's span (see [`daruda_acp::subagent_activity`]) stays active while
    /// it has a live child tool OR its last child event was within
    /// [`SUBAGENT_QUIESCENCE`], which bridges the gaps between a subagent's
    /// sequential child calls so this predicate does not flicker off in them.
    /// This is the animation-liveness predicate the status-pulse gate reads, kept
    /// distinct from [`Self::activity_state`]: a pending permission changes the
    /// badge *label* but must not stop a still-live subagent badge from animating.
    /// Cheap O(1) pre-check: could this pane possibly be busy? A prompt turn in
    /// flight, or at least one subagent has been seen this session (so the
    /// `subagent_last_activity` map is non-empty). When false, `is_busy()` is
    /// guaranteed false without scanning `items` — the gate the pulse uses to
    /// avoid an O(items) scan of idle/terminated conversations every tick.
    pub(in crate::workspace) fn maybe_active(&self) -> bool {
        self.turn.is_in_flight()
            || !self.subagent_last_activity.is_empty()
            || self.post_turn_dirty_at.is_some()
    }

    /// On the pulse tick: if a post-turn follow-up has quiesced (no in-flight turn
    /// and `POST_TURN_QUIESCENCE` elapsed since the last post-turn update), return
    /// the new assistant text to relay and advance the marker. `None` otherwise.
    pub(in crate::workspace) fn reconcile_post_turn(
        &mut self,
        now: std::time::Instant,
        quiescence: std::time::Duration,
    ) -> Option<String> {
        if self.turn.is_in_flight() {
            return None;
        }
        let dirty_at = self.post_turn_dirty_at?;
        if now.saturating_duration_since(dirty_at) < quiescence {
            return None;
        }
        self.post_turn_dirty_at = None;
        let (delta, new_count) =
            post_turn_delta(&self.items, self.post_turn_relayed_assistant_texts)?;
        self.post_turn_relayed_assistant_texts = new_count;
        Some(delta)
    }

    /// Force-flush a not-yet-quiesced post-turn follow-up (called when a new prompt
    /// is about to subsume it). `None` when nothing is pending.
    pub(in crate::workspace) fn take_pending_post_turn(&mut self) -> Option<String> {
        self.post_turn_dirty_at.take()?;
        let (delta, new_count) =
            post_turn_delta(&self.items, self.post_turn_relayed_assistant_texts)?;
        self.post_turn_relayed_assistant_texts = new_count;
        Some(delta)
    }

    /// Sync the post-turn baseline to the current `AssistantText` count and clear
    /// the dirty clock. Called wherever `items` is bulk-set to a known baseline
    /// (turn settle, restore/replay completion, transient teardown) so only
    /// messages arriving *after* that point count as a background follow-up —
    /// the single update site for this mirrored count.
    fn snap_post_turn_baseline(&mut self) {
        self.post_turn_relayed_assistant_texts = self
            .items
            .iter()
            .filter(|it| matches!(it, ChatItem::AssistantText { .. }))
            .count();
        self.post_turn_dirty_at = None;
    }

    pub(in crate::workspace) fn is_busy(&self) -> bool {
        self.turn.is_in_flight()
            || subagent_activity(
                &self.items,
                &self.subagent_last_activity,
                std::time::Instant::now(),
                SUBAGENT_QUIESCENCE,
            )
            .any_running
    }

    /// Advance the activity span for the current `now`, returning the pending
    /// completion outcome exactly on the busy→idle edge (else `None`). Drives the
    /// working-indicator elapsed anchor and the completion firing. Mutating
    /// (updates `was_busy`/`activity_started_at`); callers are the prompt-send
    /// path, the event-pump tail, the pulse tick, and the user-cancel path.
    pub(in crate::workspace) fn reconcile_activity(
        &mut self,
        now: std::time::Instant,
    ) -> Option<TurnOutcome> {
        let busy = self.turn.is_in_flight()
            || subagent_activity(
                &self.items,
                &self.subagent_last_activity,
                now,
                SUBAGENT_QUIESCENCE,
            )
            .any_running;
        let edge = match (self.was_busy, busy) {
            (false, true) => {
                self.activity_started_at = Some(now);
                None
            }
            (true, false) => {
                self.activity_started_at = None;
                // The run is over: the `subagent_last_activity` map is only
                // meaningful during an active run (the `subagent N/M` indicator
                // is hidden when idle, and a later subagent event re-populates
                // it), so clear it. This bounds the map and makes `maybe_active`
                // return false once the pane is truly idle, so `pulse_agent_chats`
                // stops re-scanning a finished-subagent pane every tick. Safe for
                // `is_busy`: this arm is only reached when `busy` is false — no
                // child is live — so clearing the timestamps cannot change
                // `any_running`.
                self.subagent_last_activity.clear();
                self.pending_completion.take()
            }
            _ => None,
        };
        self.was_busy = busy;
        edge
    }

    /// Elapsed time since the current activity span began (busy→…), or `None` when
    /// idle. Anchors the working-indicator timer to the whole activity span
    /// (turn + trailing subagents), replacing the turn-scoped `turn.started_at()`.
    pub(in crate::workspace) fn activity_elapsed(&self) -> Option<std::time::Duration> {
        self.activity_started_at.map(|t| t.elapsed())
    }

    /// Count of subagents running *right now* for the working-indicator label
    /// (`N subagents`); `None` when none are currently running. This is the live
    /// in-flight count (`total - settled`), not a cumulative "done of total ever"
    /// tally — the chip disappears the moment the last subagent settles. Uses the
    /// same span derivation as `is_busy`: a subagent counts as running while it
    /// has a live child tool or its last child activity was within
    /// `SUBAGENT_QUIESCENCE`.
    pub(in crate::workspace) fn subagent_progress(&self) -> Option<usize> {
        let a = subagent_activity(
            &self.items,
            &self.subagent_last_activity,
            std::time::Instant::now(),
            SUBAGENT_QUIESCENCE,
        );
        let running = a.total - a.settled;
        (running > 0).then_some(running)
    }

    /// Whether any permission card is still awaiting a host decision. O(1) —
    /// reads the outstanding-id index, so the render hot path (activity badge,
    /// working indicator) never rescans `items`.
    pub(in crate::workspace) fn has_pending_permission(&self) -> bool {
        !self.pending_permissions.is_empty()
    }

    /// Whether the permission request `id` is still outstanding (its card
    /// unresolved). Used by the Telegram relay to drop a phone decision for a
    /// request the user already answered in-app (or that was cancelled).
    pub(in crate::workspace) fn is_permission_outstanding(&self, id: u64) -> bool {
        self.pending_permissions.contains(&id)
    }

    /// The pane's derived activity — the single source of the badge label. A
    /// pending permission takes precedence (it needs the user, not the agent);
    /// otherwise the agent is [`ActivityState::Working`] while [`Self::is_busy`].
    pub(in crate::workspace) fn activity_state(&self) -> ActivityState {
        if self.has_pending_permission() {
            return ActivityState::AwaitingPermission;
        }
        if self.is_busy() {
            ActivityState::Working
        } else {
            ActivityState::Idle
        }
    }

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
        });
        Self {
            pane_id,
            window_handle,
            focus_handle: cx.focus_handle(),
            cwd,
            status,
            session_id,
            agent_id,
            agent_name,
            // A restored pane connects lazily; the resume decision (and the
            // `restoring` flag that coalesces the replay) is made at connect
            // time by `maybe_connect_agent_chat`, not here.
            restoring: false,
            items: Vec::new(),
            handle: None,
            pending_prompts: Vec::new(),
            _event_pump: None,
            pending_permissions: HashSet::new(),
            turn: Turn::Idle,
            subagent_last_activity: HashMap::new(),
            activity_started_at: None,
            was_busy: false,
            pending_completion: None,
            post_turn_relayed_assistant_texts: 0,
            post_turn_dirty_at: None,
            cancel_in_flight: false,
            diff_editors: HashMap::new(),
            diff_stats: HashMap::new(),
            mermaid_images: Arc::new(Mutex::new(HashMap::new())),
            mermaid_inflight: HashSet::new(),
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
            modes: None,
            config_options: Vec::new(),
            session_capabilities: SessionCapabilitiesView::default(),
            session_usage: None,
            available_commands: Vec::new(),
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

    /// Fold a single [`AcpEvent`] into the chat model + status, reconcile the
    /// diff editors / mermaid diagrams the new content needs, then notify so
    /// the (cached) pane subtree repaints. The Workspace pump calls this on the
    /// foreground for every event, passing the current `syntax_theme` /
    /// `is_light` (it owns the config mirror).
    pub(in crate::workspace) fn apply_event(
        &mut self,
        event: AcpEvent,
        syntax_theme: &str,
        is_light: bool,
        cx: &mut Context<Self>,
    ) {
        // Session errors surface inline in the status banner (the `Error` arm
        // below sets `status`), so this only records to the NDJSON log — no
        // toast. A toast here is pure noise: it duplicates the banner, and on
        // cold restore the auto-connect of any errored session would pop one
        // per pane on startup.
        if let AcpEvent::Error(message) = &event {
            let report = ErrorReport::new("ACP session error")
                .severity(ErrorSeverity::Error)
                .with_context("detail", message.clone())
                .at(file!(), line!())
                .dedup("agent_chat.session_error")
                .build();
            daruda_store::observability::log_writer::LogWriter::log(report);
        }
        // Non-fatal advisory: log at Warning severity; session stays live.
        if let AcpEvent::Notice(message) = &event {
            let report = ErrorReport::new("ACP session notice")
                .severity(ErrorSeverity::Warning)
                .with_context("detail", message.clone())
                .at(file!(), line!())
                .dedup(format!("agent_chat.notice.{}", self.pane_id))
                .build();
            daruda_store::observability::log_writer::LogWriter::log(report);
        }

        // What this event changed, to gate the expensive full-conversation
        // reconciles below. Only an `Update` carrying tool/text content sets
        // these; every other event leaves both false.
        let mut touched_tool = false;
        let mut touched_text = false;
        // Set only when a `session/load` replay just finished (the `Connected`
        // reply cleared `restoring`), so the tail runs the single catch-up.
        let mut finished_restore = false;
        // Set when a turn just settled (natural completion or a session error).
        // The turn's streamed rows may have changed height via the *trailing*
        // async markdown reparse — `TextView` re-parses ~one debounce after the
        // final chunk, off the outer list's per-chunk remeasure path — so their
        // cached heights can be stale. The tail forces a full remeasure to
        // re-derive them (defensive: prevents a stale streaming height from
        // lingering and inflating the scroll geometry into an oversized gap).
        let mut turn_settled = false;

        match event {
            AcpEvent::ConnectProgress(phase) => {
                // Guard against a stale progress event arriving after the
                // connect already resolved (Connected/Error) — same shape as
                // the NodeProgress drain's guard in `connect_agent_chat`.
                if matches!(
                    self.status,
                    AgentSessionStatus::Connecting | AgentSessionStatus::Handshaking(_)
                ) {
                    self.set_handshaking(phase, cx);
                }
            }
            AcpEvent::Connected {
                session_id,
                modes,
                config_options,
                capabilities,
            } => {
                self.status = AgentSessionStatus::Connected;
                self.modes = modes;
                self.config_options = config_options;
                self.session_capabilities = capabilities;
                // A real resume (`session/load`) returns the same id we asked to
                // load; a resume the agent couldn't load was downgraded to a fresh
                // `session/new` with a NEW id. `self.restoring` was set
                // optimistically at connect and can't tell those apart, so decide
                // by id match — otherwise a downgraded resume keeps the prior
                // session's title and skips fresh-session setup. Compare before
                // overwriting `session_id`.
                let resumed =
                    self.restoring && self.session_id.as_deref() == Some(session_id.as_str());
                self.restoring = false;
                // A resume's replayed `session/update`s already populated `items`
                // by this point (see the comment above) — sync the baseline now
                // so those replayed messages don't later look like a background
                // follow-up. A fresh session's `items` is still empty, so this is
                // a no-op there.
                self.snap_post_turn_baseline();
                // Record the live session id so it persists — and so a later
                // launch resumes this session instead of starting fresh.
                self.session_id = Some(session_id);
                if resumed {
                    // Resume (`session/load`): the replayed `session/update`s
                    // already populated the conversation, plan, and title before
                    // this reply — keep them. Let the tail run the single
                    // coalesced catch-up.
                    finished_restore = true;
                } else {
                    // Fresh session (`session/new`): clear stale plan/title so
                    // they don't flash before the new agent sends its first
                    // updates, and drop the prior session's subagent activity so
                    // its timestamps can't hold the new session's badge "busy".
                    self.plan.clear();
                    self.session_title = None;
                    self.session_updated_at = None;
                    self.plan_collapsed = false;
                    self.session_usage = None;
                    self.subagent_last_activity.clear();
                    // Reset the activity-span tracker so a prior session's edge
                    // state / captured outcome can't leak into the fresh one.
                    self.activity_started_at = None;
                    self.was_busy = false;
                    self.pending_completion = None;
                    self.cancel_in_flight = false;
                }
            }
            AcpEvent::ConfigOptionsChanged(options) => {
                self.config_options = options;
            }
            AcpEvent::UsageChanged(usage) => {
                self.session_usage = Some(usage);
            }
            AcpEvent::ModeChanged { mode_id } => {
                if let Some(m) = &mut self.modes {
                    if m.available.iter().any(|v| v.id == mode_id) {
                        m.current = mode_id;
                    } else {
                        daruda_store::observability::log_writer::LogWriter::log(
                            ErrorReport::new(
                                "ACP session: received ModeChanged for unknown mode id",
                            )
                            .severity(ErrorSeverity::Warning)
                            .at(file!(), line!())
                            .with_context("mode_id", mode_id)
                            .dedup(format!("agent_chat.unknown_mode.{}", self.pane_id))
                            .build(),
                        );
                    }
                }
            }
            AcpEvent::Update(update) => {
                // Fold protocol traffic through this pane's per-agent strategy
                // (selected from its catalog id) so vendor-specific `_meta` is
                // read the way that agent emits it. See `daruda_acp::adapter`.
                let adapter = daruda_acp::adapter::adapter_for(&self.agent_id);
                let effect = apply_update_with(&mut self.items, &update, adapter.as_ref());
                touched_tool = effect.touched_tool;
                touched_text = effect.touched_text;
                // Post-turn (background) activity: an update that touches text or a
                // tool while no turn is in flight and we're not replaying a load.
                // Stamp the quiescence clock; the pulse tick relays the settled
                // follow-up (Claude reports background completion here, with no
                // TurnEnded to trigger the normal completion relay).
                if (touched_text || touched_tool) && !self.turn.is_in_flight() && !self.restoring {
                    self.post_turn_dirty_at = Some(std::time::Instant::now());
                }
                // Bump the subagent (parent) whose child just produced this
                // tool-call event, so its run span stays "active" across the
                // gaps between the subagent's sequential child calls. Only child
                // tools carry a `parent_tool_id`; a top-level tool has none, so
                // nothing is bumped for the turn's own (foreground) work.
                if let Some(tool_id) = touched_tool_id(&update) {
                    let parent = self.items.iter().rev().find_map(|it| match it {
                        ChatItem::ToolCall(tc) if tc.id == tool_id => tc.parent_tool_id.clone(),
                        _ => None,
                    });
                    if let Some(parent) = parent {
                        self.subagent_last_activity
                            .insert(parent, std::time::Instant::now());
                    }
                }
            }
            AcpEvent::PermissionRequested { id, request } => {
                let item = permission_item(id, &request, &self.items);
                self.items.push(item);
                self.pending_permissions.insert(id);
            }
            AcpEvent::TurnEnded { .. } | AcpEvent::TurnFailed(_) if self.cancel_in_flight => {
                // The terminal signal (a `cancelled` `TurnEnded`, or a
                // `TurnFailed` if the prompt errored as the cancel raced it) for a
                // turn a Stop already settled locally — its `Stopped` fired at
                // cancel time. Don't re-settle, re-complete, or push an error
                // item: just close the cancel window and drain any re-prompt the
                // user buffered while it was open (as a fresh turn). A buffered
                // re-prompt was never put on the wire (see `send_prompt_text`'s
                // `cancel_in_flight` guard), so nothing raced this ack and a
                // second Stop could still have cleared it.
                self.cancel_in_flight = false;
                self.pump_pending_prompt(cx);
            }
            AcpEvent::TurnEnded {
                completed_normally, ..
            } => {
                // Settle the turn: finalize streaming, cancel any tool the agent
                // left non-terminal (e.g. a `Cancelled` stop reason), and drain a
                // still-pending permission so no card keeps live buttons.
                self.settle_turn();
                turn_settled = true;
                // Capture the outcome; it fires only when the pane settles
                // busy→idle (via `reconcile_activity`), which may trail this
                // `end_turn` while trailing subagents finish.
                self.pending_completion = Some(if completed_normally {
                    TurnOutcome::Completed
                } else {
                    TurnOutcome::Stopped
                });
                // Drain the next buffered prompt (if any) now that the turn
                // completed — one per `TurnEnded`, so the queue advances a single
                // turn at a time and `turn.is_in_flight()` keeps tracking exactly
                // one live turn. A no-op when nothing is buffered or on a cancelled
                // turn whose queue is empty. Pumping here (not inside
                // `settle_turn`) is deliberate: `settle_turn` is also the Stop /
                // `Error` teardown, and pumping from all three would double-drain
                // (a cancelled turn's later idempotent `TurnEnded` re-runs
                // settle) — only a natural completion should advance the queue.
                self.pump_pending_prompt(cx);
                // Auto-collapse the plan region so the completed checklist
                // recedes to a one-line summary. The next `PlanChanged` will
                // re-expand it (see below).
                if !self.plan.is_empty() {
                    self.plan_collapsed = true;
                }
            }
            AcpEvent::AvailableCommandsChanged(commands) => {
                self.available_commands = commands;
            }
            // Full-replace the plan. Auto-expand when the plan content actually
            // changes (new turn's plan arrived) so the user sees the fresh
            // checklist. Mid-turn re-deliveries of the same entries keep the
            // current collapsed/expanded state to avoid fighting the user.
            AcpEvent::PlanChanged(entries) => {
                if self.plan != entries {
                    self.plan_collapsed = false;
                }
                self.plan = entries;
            }
            AcpEvent::SessionInfoChanged { title, updated_at } => {
                apply_info_field(&mut self.session_title, title);
                apply_info_field(&mut self.session_updated_at, updated_at);
            }
            AcpEvent::Notice(_) => {
                // Logged above; no status change.
            }
            AcpEvent::TurnFailed(message) => {
                // A single `session/prompt` failed (e.g. the adapter hit a usage
                // / session limit → `-32603`), but the ACP connection is alive —
                // the error was a normal JSON-RPC response, not a transport
                // failure. So unlike the terminal `Error` arm below, DO NOT set
                // `status = Error` and DO NOT drop the handle: keep the session
                // Connected and usable so the user can re-prompt (e.g. once the
                // limit resets) without reconnecting.
                //
                // Surface the failure inline in the conversation, then settle the
                // turn exactly as a `TurnEnded` would — otherwise a streaming
                // block stays `streaming: true` and an `InProgress` tool stays
                // live, so the rollup glyph blinks forever and the footer reads
                // `Running` after the turn is already over.
                self.items.push(ChatItem::Error(message));
                // (A `TurnFailed` while `cancel_in_flight` is handled by the
                // guarded arm above; here the turn was not being cancelled.)
                self.settle_turn();
                turn_settled = true;
                // Capture the errored outcome; it fires (notification +
                // backing-task done) on the busy→idle settle edge that
                // `reconcile_activity` detects, same as a normal completion.
                self.pending_completion = Some(TurnOutcome::Errored);
                // Advance the prompt queue one turn like a natural completion, so
                // a prompt the user buffered while this turn ran still runs. No-op
                // when nothing is buffered (the common single-prompt case).
                self.pump_pending_prompt(cx);
            }
            AcpEvent::Error(message) => {
                let error_message = match &self.cwd {
                    Some(PaneCwd::Remote(_)) => {
                        format!(
                            "{}\n\n{}",
                            message,
                            s::agent_chat_remote_connect_error_hint()
                        )
                    }
                    _ => message,
                };
                self.status = AgentSessionStatus::Error(error_message);
                // A session-level error terminates every outstanding turn,
                // including any cancel we were still awaiting an ack for — close
                // the cancel window so a post-reconnect turn isn't misread.
                self.cancel_in_flight = false;
                // A load that fails mid-replay must still render whatever was
                // replayed — release the coalescing gate so the tail rebuilds.
                self.restoring = false;
                // Whatever replayed before the failure is now the baseline —
                // it was already delivered by the replay itself, not a
                // background follow-up.
                self.snap_post_turn_baseline();
                // A mid-turn failure must settle the turn like a Stop would —
                // otherwise a streaming block stays `streaming: true` and an
                // `InProgress` tool stays live, so the rollup glyph blinks
                // forever and the response bar reads `Running` after the session
                // is already dead.
                self.settle_turn();
                turn_settled = true;
                // Capture the failure outcome; it fires on the busy→idle settle
                // edge (via `reconcile_activity`), same as a normal completion.
                self.pending_completion = Some(TurnOutcome::Errored);
                // The session is dead with no reconnect path, so any buffered
                // prompts can never be delivered — drop them (they were already
                // echoed locally) rather than leaving them to be pumped.
                self.pending_prompts.clear();
                // Drop the now-dead handle. The connection task has ended (this
                // `Error` is its terminal signal), so its command channel is
                // closed — a lingering `Some(handle)` would let `send_prompt_text`
                // send into a dead channel (silently dropped) and mark a turn
                // in-flight that never ends, stranding the pane on a phantom
                // "Working". With `None`, a post-error prompt buffers instead of
                // stranding. (Distinct from `TurnFailed`, which keeps the handle:
                // there the connection is still alive.)
                self.handle = None;
            }
        }
        // Gate the full-conversation reconciles on what the event actually
        // changed: diff editors only when a tool call moved, mermaid raster only
        // when message text changed. Running both on every streamed chunk would
        // rescan the whole `items` vec per chunk — O(n²) over a long turn.
        if touched_tool {
            self.reconcile_diff_editors(syntax_theme, is_light, cx);
        }
        if touched_text {
            self.reconcile_mermaid(!is_light, cx);
        }
        // During a `session/load` replay the adapter streams the whole prior
        // conversation as many `session/update`s before the `Connected` reply.
        // The reconciles above still run per-event (so diff editors / mermaid
        // for replayed content are built as they arrive), but the row rebuild +
        // repaint is deferred until the `Connected` reply releases the gate — one
        // catch-up instead of a rebuild per replayed event. (The per-event
        // reconciles are unchanged, so replay cost is no better than live-
        // streaming the same events; this only removes the redundant rebuilds.)
        if self.restoring {
            return;
        }
        // Reproject rows + sync the virtualized list. `FollowMode::Tail` keeps
        // the bottom pinned while streaming — no manual scroll needed.
        self.rebuild_rows();
        // Re-measure after a structural settle so no row keeps a stale streaming
        // height. Two triggers, two anchor policies:
        // (a) a `session/load` replay just spliced many rows at once — force a
        //     full `remeasure()` so the list has heights for all of them before
        //     the paint. A cold restore anchors to the tail, so the proportional
        //     re-anchor `remeasure()` performs is irrelevant here.
        // (b) a turn just settled — its streamed rows may have changed height via
        //     the trailing async markdown reparse. Re-derive every row's height,
        //     but through the span API (`remeasure_items`, Absolute anchor) rather
        //     than `remeasure()` (Proportional): if the user has scrolled back to
        //     read history, a Proportional re-anchor shifts their viewport when the
        //     anchored row's height changes, whereas Absolute keeps it fixed.
        // Cheap: at most once per restore / turn.
        if finished_restore {
            self.list_state.remeasure();
        }
        if turn_settled {
            // Span is all rows and the count is unchanged, so `to` and `prev_rows`
            // both equal the current row count.
            let n = self.rows.len();
            self.list_state.remeasure_items(0..n);
            self.trace_list_sync("turn-settled", 0, n, n);
        }
        cx.notify();
    }

    /// Release a stuck replay gate. A `session/load` normally ends in either a
    /// `Connected` reply or an `Error`, both of which clear `restoring`; but if
    /// a misbehaving adapter closes the event stream mid-load without either,
    /// the gate would stay set and the accumulated items would never be
    /// projected — a pane frozen mid-restore. The pump calls this once its loop
    /// exits so whatever arrived still renders. No-op when not restoring.
    pub(in crate::workspace) fn abort_restore(&mut self, cx: &mut Context<Self>) {
        if self.restoring {
            self.restoring = false;
            // Whatever arrived before the stream closed is now the baseline —
            // it was already delivered by the (aborted) replay, not a
            // background follow-up.
            self.snap_post_turn_baseline();
            self.rebuild_rows();
            self.list_state.remeasure();
            cx.notify();
        }
    }

    /// End-of-stream safety-net predicate: true while `status` is a
    /// non-terminal connecting state (`PreparingRuntime`/`Connecting`/
    /// `Handshaking`). The event pump checks this once its loop exits
    /// (channel closed — handle dropped, or the connection task ended without
    /// emitting a terminal event). Normally the stream never closes before
    /// `Connected` or `Error` fires; both already resolve the status. But a
    /// connection task that panics, or a `run_connection` future dropped by an
    /// upstream bug before its `Err` path runs, closes the channel silently —
    /// with no event left to ever move `status`, the pane would otherwise be
    /// stuck on "Connecting…"/"Handshaking…" forever with no error and no
    /// retry affordance (`Workspace::retry_agent_chat_connect` requires
    /// `Error` to fire). The pump feeds a real `AcpEvent::Error` through
    /// `apply_event` when this is true, rather than setting `status` here
    /// directly, so the failure gets the exact same handling as any other
    /// terminal error (turn settle, handle drop, pending-prompt clear —
    /// see the `AcpEvent::Error` arm).
    pub(in crate::workspace) fn is_still_connecting(&self) -> bool {
        matches!(
            self.status,
            AgentSessionStatus::PreparingRuntime(_)
                | AgentSessionStatus::Connecting
                | AgentSessionStatus::Handshaking(_)
        )
    }

    /// Recompute the projected render rows from `items` + `fold` and sync the
    /// virtualized `list` to them. The single rebuild site; call after any
    /// `items` or `fold` mutation.
    ///
    /// Rows include synthetic headers (tool-group, later response), and folding
    /// flips `hidden` without changing the row set — so we diff old vs new by
    /// `same_slot`:
    /// - **structural** (slot divergence / count change = an append, or a run
    ///   becoming a group): `splice` from the first divergent slot — scroll
    ///   above it is preserved.
    /// - **same slots & count** (a fold toggle flipping `hidden`, or a streamed
    ///   chunk growing the tail): `remeasure_items` over just the changed span
    ///   with the `Absolute` anchor, so reading history during streaming never
    ///   drifts (a full `remeasure()` would re-anchor proportionally).
    fn rebuild_rows(&mut self) {
        let old = std::mem::take(&mut self.rows);
        // The inline working indicator means "answering" — suppress it while
        // blocked on a permission prompt (the card + footer already say so).
        let awaiting_response = matches!(self.activity_state(), ActivityState::Working);
        self.rows = project(&self.items, &self.fold, awaiting_response);

        if let Some(at) = old
            .iter()
            .zip(&self.rows)
            .position(|(a, b)| !a.same_slot(b))
        {
            self.list_state.splice(at..old.len(), self.rows.len() - at);
            self.trace_list_sync("splice-divergent", at, self.rows.len(), old.len());
            return;
        }
        if old.len() != self.rows.len() {
            let at = old.len().min(self.rows.len());
            self.list_state.splice(at..old.len(), self.rows.len() - at);
            self.trace_list_sync("splice-count", at, self.rows.len(), old.len());
            return;
        }
        // Same slots & count: only `hidden` flipped or item content grew.
        // Remeasure the span whose `hidden` changed; if none did, a streamed
        // tail grow → remeasure the last row.
        let (lo, hi) = old
            .iter()
            .zip(&self.rows)
            .enumerate()
            .filter(|(_, (a, b))| a.hidden != b.hidden)
            .fold((usize::MAX, 0usize), |(lo, hi), (i, _)| {
                (lo.min(i), hi.max(i))
            });
        if lo == usize::MAX {
            // No `hidden` flip: a streamed chunk grew a row's content in place.
            // During an active turn the *last* row is a fixed-height
            // `WorkingIndicator` (`rows::project` pins it to the tail), so the row
            // that actually grew is the last non-indicator row. Remeasuring only
            // `n-1` would re-measure the indicator and leave the grown content row
            // at its stale (shorter) cached height — which then inflates the
            // scroll geometry the moment that row scrolls into the overdraw zone
            // (the intermittent oversized-gap bug). Remeasure from the last
            // content row through the end so both the grown row and the indicator
            // are covered.
            let n = self.rows.len();
            if n > 0 {
                let start = self.rows[..n]
                    .iter()
                    .rposition(|r| !matches!(r.kind, RowKind::WorkingIndicator))
                    .unwrap_or(n - 1);
                self.list_state.remeasure_items(start..n);
                self.trace_list_sync("tail-grow", start, n, old.len());
            }
        } else {
            self.list_state.remeasure_items(lo..hi + 1);
            self.trace_list_sync("hidden-span", lo, hi + 1, old.len());
        }
    }

    /// Trace one list-sync decision — a splice or a remeasure — to the NDJSON
    /// log, silent unless `DARUDA_DEBUG_AGENT_LIST` is set in the environment.
    /// The intermittent oversized-gap ("~3 page") bug is a virtualized-list
    /// height-cache staleness issue: a row's height changes (async markdown
    /// reparse, fold, tool-result landing) without the matching row being
    /// re-measured, so its stale cached height inflates the scroll geometry.
    /// This stays compiled in (near-zero cost when off) so, on recurrence,
    /// flipping the env var captures the sync timeline — `branch` + the
    /// `[from, to)` span touched vs. the total row count — to confirm whether a
    /// content change slipped through without a remeasure.
    ///
    /// `prev_rows` is the row count *before* this sync; it equals the current
    /// count for a remeasure (which never changes the count) and differs only on
    /// a splice, so a count delta is visible in the trace.
    fn trace_list_sync(&self, branch: &str, from: usize, to: usize, prev_rows: usize) {
        if !debug_list_trace_enabled() {
            return;
        }
        daruda_store::observability::log_writer::LogWriter::log(
            ErrorReport::new("agent-chat list sync")
                .severity(ErrorSeverity::Info)
                .with_context("pane", self.pane_id.to_string())
                .with_context("branch", branch.to_string())
                .with_context("from", from.to_string())
                .with_context("to", to.to_string())
                .with_context("rows", self.rows.len().to_string())
                .with_context("prev_rows", prev_rows.to_string())
                .at(file!(), line!())
                .dedup("agent_chat.list_sync_trace")
                .build(),
        );
    }

    /// Send `text` as a prompt: echo it locally, forward it over the session,
    /// and mark a turn in flight. Driven by the bottom-dock input via the
    /// Workspace shim (`send_agent_prompt_text`).
    pub(in crate::workspace) fn send_prompt_text(&mut self, text: String, cx: &mut Context<Self>) {
        // Echo locally so the prompt shows immediately even before the agent
        // streams it back as a user-message chunk.
        self.items.push(ChatItem::UserText(text.clone()));
        if let Some(handle) = &self.handle
            && !self.turn.is_in_flight()
            && !self.cancel_in_flight
        {
            // Connected and idle: send now and mark the turn in flight.
            handle.send_prompt(text);
            self.turn = Turn::InFlight {
                started_at: std::time::Instant::now(),
            };
        } else {
            // Not connected yet (lazy connect happens on first focus), a turn is
            // already in flight, or a Stop's cancel is still outstanding
            // (`cancel_in_flight` — buffer client-side so a second Stop can clear
            // it and it can't race the cancel's ack onto the wire). Buffer in
            // submission order — the local echo above already shows it — and drain
            // it one-per-turn via `pump_pending_prompt` (at connect, on each
            // `TurnEnded`, and when the cancel window closes). Do *not* mark the
            // turn in flight: nothing new is on the wire yet.
            self.pending_prompts.push(text);
        }
        // There is no `ToolCall` at a prompt-echo, so the diff reconcile would
        // be a no-op here; diff editors are reconciled solely on the event-pump
        // path. The echoed `UserText` renders its markdown directly; a prompt
        // may carry a ` ```mermaid ` fence, so rasterize those (no-op when
        // there are none).
        let dark = Self::host_is_dark(cx);
        self.reconcile_mermaid(dark, cx);
        self.rebuild_rows();
        // Submitting a prompt jumps the view to the bottom so the user sees their
        // message and the streaming response. `scroll_to_end` only repositions
        // the viewport; `FollowMode::Tail` re-engages on the first layout pass
        // that lands at the bottom (gpui `list` re-arms following there), so the
        // streaming response keeps sticking — no manual stick flag needed.
        self.list_state.scroll_to_end();
        cx.notify();
    }

    /// Send the next single buffered prompt iff the session is connected and no
    /// turn is currently in flight. Pops the FRONT of the queue (FIFO), forwards
    /// it over the handle, marks the turn in flight, and notifies. No-op when
    /// there is no handle, a turn is already running, or the buffer is empty.
    ///
    /// This drains the queue one prompt per turn-completion (the connect site
    /// pumps the first; each `TurnEnded` pumps the next) so the view never
    /// tracks more than one turn at a time — the Stop / Send affordance then
    /// reflects the single live turn instead of clearing while later queued
    /// turns are still streaming. The echo was already appended when the prompt
    /// was buffered in `send_prompt_text`, so this does not re-echo.
    pub(in crate::workspace) fn pump_pending_prompt(&mut self, cx: &mut Context<Self>) {
        // Hold the queue while a cancel is still outstanding (`cancel_in_flight`):
        // the buffered re-prompt drains only once that window closes, via the
        // `TurnEnded` ack path that clears the flag and calls this.
        if self.turn.is_in_flight() || self.cancel_in_flight || self.pending_prompts.is_empty() {
            return;
        }
        let Some(handle) = self.handle.as_ref() else {
            return;
        };
        // FIFO: the front of the queue is the oldest buffered prompt.
        let text = self.pending_prompts.remove(0);
        handle.send_prompt(text);
        self.turn = Turn::InFlight {
            started_at: std::time::Instant::now(),
        };
        cx.notify();
    }

    /// Stop the active turn. Sends `session/cancel` *and* ends the turn locally,
    /// right now — it does not wait for the agent's stop reason.
    ///
    /// `session/cancel` is only cooperative: a hung or dead agent may never
    /// return a `Cancelled` stop reason, and `AcpEvent::TurnEnded` (the sole
    /// other place the in-flight turn clears) would then never arrive — leaving
    /// the turn pulsing forever with the input stuck on "Stop". So Stop is
    /// authoritative here: clear the in-flight state, settle streaming text and
    /// still-running tool calls, and drain any pending permission. A later
    /// `TurnEnded` for this turn is idempotent. Mirrors zed's
    /// `AcpThread::cancel`, which takes `running_turn` and marks pending tools
    /// cancelled without awaiting the agent.
    pub(in crate::workspace) fn cancel_turn(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = &self.handle {
            handle.cancel();
        }
        // Settle the turn *locally and immediately* — responsive and hung-safe: a
        // connected agent that never returns the `cancelled` `TurnEnded` (or
        // returns it much later) can't leave the pane stuck busy. When a live
        // foreground turn is being cancelled, stash its `Stopped` (fired at the
        // busy→idle edge by `cancel_agent_turn`'s reconcile) and open the cancel
        // window (`cancel_in_flight`): until the cancel is acked, a re-prompt
        // buffers *client-side* (see `send_prompt_text`) rather than racing onto
        // the wire, so it can't be misattributed to the cancelled turn's ack and
        // a second Stop can still clear it. A trailing-subagent Stop (turn already
        // idle) neither stashes nor opens the window: the foreground turn's
        // already-captured outcome is preserved and fires when the tools settle.
        if self.turn.is_in_flight() {
            self.pending_completion = Some(TurnOutcome::Stopped);
            self.cancel_in_flight = true;
        }
        self.settle_turn();
        // Stop halts everything queued *before* it: drop prompts buffered prior to
        // this Stop. A prompt the user types *after* Stop is pushed after this
        // clear and runs as a fresh turn.
        self.pending_prompts.clear();
        // `settle_turn` mutated items (streaming → done, running tools →
        // cancelled, pending card → resolved), changing fold visibility and row
        // heights, so reproject and remeasure before notifying.
        self.rebuild_rows();
        self.list_state.remeasure();
        cx.notify();
    }

    /// End the current turn *locally* and settle every still-live item: clear
    /// the in-flight flags, finalize streaming text, mark running tool calls
    /// cancelled, and drain any pending permission. Model-only (no rows /
    /// notify) so the three call paths — the Stop button (`cancel_turn`), a
    /// normal `TurnEnded`, and a terminal `Error` — share one settle sequence
    /// and can never drift (e.g. one leaving streaming/tools live so the rollup
    /// blinks forever). Idempotent: a later `TurnEnded` after a Stop is a no-op.
    fn settle_turn(&mut self) {
        self.turn = Turn::Idle;
        self.settle_items();
        // Everything the turn produced is delivered by the completion relay, so
        // reset the post-turn baseline to the current assistant-text count; only
        // messages that arrive *after* this settle count as a follow-up.
        self.snap_post_turn_baseline();
    }

    /// Settle every still-live *item* — finalize streaming text, mark running
    /// tool calls cancelled, drain any pending permission — without touching the
    /// prompt `turn`. [`Self::settle_turn`] is this plus `turn = Idle`;
    /// [`Self::cancel_turn`] calls this alone so the turn stays in-flight until
    /// its real (`cancelled`) `TurnEnded` drives the settle + queue drain.
    fn settle_items(&mut self) {
        finalize_streaming(&mut self.items);
        cancel_pending_tools(&mut self.items);
        cancel_pending_permission(self);
    }

    /// Resolve the permission request `request_id` with the chosen option.
    /// Marks *that* card resolved (found by id, not by position — several may be
    /// outstanding), sends the decision over the session, and drops the id from
    /// the outstanding index. No-op if the request was already answered or
    /// cancelled. `kind` selects Allow vs. Reject semantics.
    pub(in crate::workspace) fn respond_permission(
        &mut self,
        request_id: u64,
        option_id: String,
        kind: PermissionKindView,
        cx: &mut Context<Self>,
    ) {
        if !self.pending_permissions.remove(&request_id) {
            return;
        }
        // Mark the card that carries this request id resolved so its buttons
        // disable and the choice shows — the others stay live.
        if let Some(card) = permission_card_mut(self, request_id) {
            card.resolved = Some(daruda_acp::PermissionResolution::Chosen(option_id.clone()));
        }
        let decision = match kind {
            PermissionKindView::AllowOnce | PermissionKindView::AllowAlways => {
                PermissionDecision::Allow { option_id }
            }
            PermissionKindView::RejectOnce | PermissionKindView::RejectAlways => {
                PermissionDecision::Reject { option_id }
            }
        };
        if let Some(handle) = &self.handle {
            handle.respond_permission(request_id, decision);
        }
        // The card is now resolved, so it no longer force-stays-visible under a
        // collapsed response: reproject so it folds back into the process
        // immediately. It also renders shorter (buttons → outcome line), so
        // remeasure for the reflow (item count unchanged).
        self.rebuild_rows();
        self.list_state.remeasure();
        cx.notify();
    }

    /// Toggle the fold state of one block. Resolves the `active` flag the same
    /// way `render` derives it, so the first click flips the *visible* state
    /// rather than re-deriving from a stale default.
    pub(in crate::workspace) fn toggle_fold(&mut self, key: FoldKey, cx: &mut Context<Self>) {
        // Resolve `active` via the shared `fold_active` — the same source
        // `rows::project` uses to derive the default collapsed state — so the
        // first click flips the *visible* state rather than a stale re-derivation.
        let active = fold_active(&key, &self.items);
        self.fold.toggle(key, active);
        // A fold change flips row `hidden` flags (and may collapse a group):
        // reproject + reflow the affected span.
        self.rebuild_rows();
        cx.notify();
    }

    /// Expand or collapse every currently-visible foldable block at once (the
    /// fold toolbar's expand-all / collapse-all).
    pub(in crate::workspace) fn set_all_folds(&mut self, expanded: bool, cx: &mut Context<Self>) {
        let keys = collect_foldable_keys(&self.items);
        self.fold.set_all(keys, expanded);
        // Bulk expand/collapse flips many row `hidden` flags: reproject + reflow.
        self.rebuild_rows();
        cx.notify();
    }

    /// Collapse / expand the bottom plan region. The plan is a derived render
    /// of `plan` (full-replaced by the agent), so this only flips the local
    /// presentation flag and notifies — no model rebuild needed.
    pub(in crate::workspace) fn toggle_plan_collapsed(&mut self, cx: &mut Context<Self>) {
        self.plan_collapsed = !self.plan_collapsed;
        cx.notify();
    }

    /// Dismiss the plan region: drop the entries so `plan_region` renders
    /// nothing. The plan is a derived render of `plan` (full-replaced by the
    /// agent), so clearing it is a pure local presentation reset — a later
    /// turn's `PlanChanged` repopulates it (expanded, since `plan_collapsed` is
    /// reset here). Wired to the header's × button, shown only once every entry
    /// is completed, so the finished checklist no longer lingers as a collapsed
    /// header with no clear close point.
    pub(in crate::workspace) fn dismiss_plan(&mut self, cx: &mut Context<Self>) {
        self.plan.clear();
        self.plan_collapsed = false;
        cx.notify();
    }

    /// Switch the active session mode. Optimistically updates `modes.current`
    /// so the chip reflects the selection immediately; the adapter reconciles
    /// via a `ModeChanged` event if it disagrees. Sends `session/set_mode` over
    /// the live handle (no-op when the handle is absent).
    pub(in crate::workspace) fn set_mode(&mut self, mode_id: String, cx: &mut Context<Self>) {
        if let Some(m) = &mut self.modes {
            m.current = mode_id.clone();
        }
        if let Some(h) = &self.handle {
            h.set_mode(mode_id);
        }
        cx.notify();
    }

    /// Change a select config option (model / effort / …). Optimistically
    /// updates the option's `current_value` so the chip reflects the choice
    /// immediately; the adapter reconciles by replacing the whole option set via
    /// a `ConfigOptionsChanged` event. Sends `session/set_config_option` over
    /// the live handle (no-op when the handle is absent).
    pub(in crate::workspace) fn set_config_option(
        &mut self,
        config_id: String,
        value: String,
        cx: &mut Context<Self>,
    ) {
        if let Some(opt) = self.config_options.iter_mut().find(|o| o.id == config_id) {
            opt.current_value = value.clone();
        }
        if let Some(h) = &self.handle {
            h.set_config_option(config_id, value);
        }
        cx.notify();
    }

    /// Shared teardown for a full `/clear` reset and a post-`Error` retry:
    /// drop the live handle and event pump (closing the ACP command channel
    /// and ending the pump loop — the same teardown a pane close performs)
    /// and wipe the conversation model + every runtime cache. Does NOT touch
    /// `session_id`, `restoring`, or `status` — the two callers differ there:
    /// a `/clear` reset drops the session id for a brand-new conversation; a
    /// retry keeps it so the reconnect resumes the same one via
    /// `session/load` instead of losing history.
    fn teardown_transient_session_state(&mut self) {
        self.handle = None;
        self._event_pump = None;
        self.items.clear();
        self.pending_prompts.clear();
        self.pending_permissions.clear();
        self.turn = Turn::Idle;
        self.subagent_last_activity.clear();
        self.activity_started_at = None;
        self.was_busy = false;
        self.pending_completion = None;
        self.cancel_in_flight = false;
        self.session_usage = None;
        self.diff_editors.clear();
        self.diff_stats.clear();
        self.mermaid_inflight.clear();
        if let Ok(mut m) = self.mermaid_images.lock() {
            m.clear();
        }
        self.fold = FoldState::default();
        self.modes = None;
        self.config_options.clear();
        self.available_commands.clear();
        self.plan.clear();
        self.plan_collapsed = false;
        self.session_title = None;
        self.session_updated_at = None;
    }

    /// Full local reset for the `/clear` slash command: wipe the conversation
    /// model and every runtime cache via [`Self::teardown_transient_session_state`],
    /// then also drop the persisted session id. The caller
    /// (`Workspace::reset_agent_chat_session`) then calls `connect_agent_chat`
    /// to supersede this with a fresh `session/new`, so the view never sits
    /// handle-less for a render.
    pub(in crate::workspace) fn reset_for_new_session(&mut self, cx: &mut Context<Self>) {
        self.teardown_transient_session_state();
        // Clear the persisted id so a restart resumes the fresh session, not
        // the cleared conversation (Connected re-persists the new id).
        self.session_id = None;
        self.restoring = false;
        // `teardown_transient_session_state` already cleared `items`, so this
        // resets the baseline to 0 — a stray post-turn update queued before
        // teardown can't relay stale text into the fresh session.
        self.snap_post_turn_baseline();
        self.status = AgentSessionStatus::Connecting;
        self.rebuild_rows(); // diff-splices list_state down to 0 rows
        cx.notify();
    }

    /// Reconnect after a terminal `Error` without losing the conversation:
    /// same teardown as `reset_for_new_session` (clears local `items` so a
    /// resume's replay doesn't duplicate what was already rendered before the
    /// failure) but keeps `session_id`, so `connect_agent_chat` resumes via
    /// `session/load` and the adapter replays the exact same history back.
    /// Called by `Workspace::retry_agent_chat_connect`, which then
    /// re-invokes `connect_agent_chat` with `self.session_id` as the resume.
    pub(in crate::workspace) fn retry_for_reconnect(&mut self, cx: &mut Context<Self>) {
        self.teardown_transient_session_state();
        self.restoring = self.session_id.is_some();
        self.status = AgentSessionStatus::Connecting;
        self.rebuild_rows();
        cx.notify();
    }

    /// Jump the conversation list to the bottom and re-engage `Tail` follow.
    /// Backs the floating scroll-to-bottom button. The list internally re-arms
    /// tail-following once it lands at the end, so a fresh streaming chunk keeps
    /// sticking to the bottom.
    pub(in crate::workspace) fn scroll_to_bottom(&mut self, cx: &mut Context<Self>) {
        self.list_state.scroll_to_end();
        cx.notify();
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
mod tests {
    fn assistant_text_item(text: &str) -> daruda_acp::ChatItem {
        daruda_acp::ChatItem::AssistantText {
            text: text.to_string(),
            streaming: false,
            message_id: None,
        }
    }

    /// Build a minimal, offline `AgentChatView` (no cwd → `Idle` status, no
    /// adapter spawned) as its own window root, so the post-turn marker
    /// methods (`&mut self` + `Instant`/`Duration`, no `Workspace`) can be
    /// driven directly. Lighter than `workspace/tests/agent_chat.rs`'s
    /// `make_activity_view` (which goes through a full `Workspace` +
    /// `create_agent_chat_pane`): `AgentChatView::new` only needs a
    /// `Context<Self>`, not a `Workspace` at all.
    fn make_test_view(cx: &mut gpui::TestAppContext) -> gpui::WindowHandle<super::AgentChatView> {
        crate::test_support::init_gpui_component(cx);
        cx.add_window(|window, cx| {
            super::AgentChatView::new(
                0,
                window.window_handle(),
                None,
                super::AgentSessionStatus::Idle,
                None,
                "claude".to_string(),
                "Claude".to_string(),
                None,
                cx,
            )
        })
    }

    /// `reconcile_post_turn` withholds the delta until `quiescence` has
    /// elapsed since the dirty stamp, then relays it and advances the marker
    /// so a second call (still no new text) is a no-op.
    #[gpui::test]
    fn reconcile_post_turn_waits_for_quiescence_then_dedups(cx: &mut gpui::TestAppContext) {
        let window = make_test_view(cx);
        let quiescence = std::time::Duration::from_millis(100);
        let dirty_at = std::time::Instant::now();

        window
            .update(cx, |view, _window, _cx| {
                view.items.push(assistant_text_item("done"));
                view.post_turn_dirty_at = Some(dirty_at);
            })
            .unwrap();

        window
            .update(cx, |view, _window, _cx| {
                assert_eq!(
                    view.reconcile_post_turn(dirty_at, quiescence),
                    None,
                    "not yet quiesced"
                );
            })
            .unwrap();

        let settled = dirty_at + quiescence + std::time::Duration::from_millis(1);
        window
            .update(cx, |view, _window, _cx| {
                assert_eq!(
                    view.reconcile_post_turn(settled, quiescence),
                    Some("done".to_string()),
                    "quiesced: relays the delta and advances the marker"
                );
                assert_eq!(
                    view.reconcile_post_turn(settled, quiescence),
                    None,
                    "second call has nothing new to relay"
                );
            })
            .unwrap();
    }

    /// `take_pending_post_turn` force-flushes a not-yet-quiesced follow-up
    /// exactly once, then reports nothing pending.
    #[gpui::test]
    fn take_pending_post_turn_flushes_once(cx: &mut gpui::TestAppContext) {
        let window = make_test_view(cx);
        window
            .update(cx, |view, _window, _cx| {
                view.items.push(assistant_text_item("flushed"));
                view.post_turn_dirty_at = Some(std::time::Instant::now());
                assert_eq!(view.take_pending_post_turn(), Some("flushed".to_string()));
                assert_eq!(view.take_pending_post_turn(), None);
            })
            .unwrap();
    }

    /// Regression for the resume/replay baseline bug (Finding 1): after
    /// `snap_post_turn_baseline()` syncs the marker to a pre-populated
    /// history (as every `restoring = false` site now does), a stray
    /// post-turn update that arrives with no new assistant text must not
    /// resurrect the replayed conversation as a "background follow-up".
    /// Before the fix, the marker stayed at its constructor default (0)
    /// across a resume, so this same sequence would have relayed the whole
    /// two-item history.
    #[gpui::test]
    fn snap_post_turn_baseline_prevents_replay_from_being_relayed(cx: &mut gpui::TestAppContext) {
        let window = make_test_view(cx);
        window
            .update(cx, |view, _window, _cx| {
                view.items.push(assistant_text_item("history-1"));
                view.items.push(assistant_text_item("history-2"));
                view.snap_post_turn_baseline();
                assert_eq!(view.post_turn_relayed_assistant_texts, 2);

                view.post_turn_dirty_at =
                    Some(std::time::Instant::now() - std::time::Duration::from_secs(10));
                assert_eq!(
                    view.reconcile_post_turn(
                        std::time::Instant::now(),
                        super::POST_TURN_QUIESCENCE
                    ),
                    None,
                    "resumed baseline must not re-relay replayed history"
                );
            })
            .unwrap();
    }

    #[test]
    fn post_turn_delta_none_when_nothing_new() {
        let items = vec![assistant_text_item("promise")];
        assert_eq!(super::post_turn_delta(&items, 1), None);
    }

    #[test]
    fn post_turn_delta_returns_new_item_and_advances_count() {
        let items = vec![
            assistant_text_item("promise"),
            assistant_text_item("완료되었습니다"),
        ];
        assert_eq!(
            super::post_turn_delta(&items, 1),
            Some(("완료되었습니다".to_string(), 2))
        );
    }

    #[test]
    fn post_turn_delta_joins_multiple_new_items() {
        let items = vec![
            assistant_text_item("a"),
            assistant_text_item("b"),
            assistant_text_item("c"),
        ];
        assert_eq!(
            super::post_turn_delta(&items, 1),
            Some(("b\n\nc".to_string(), 3))
        );
    }

    #[test]
    fn post_turn_delta_ignores_whitespace_only_delta() {
        let items = vec![assistant_text_item("x"), assistant_text_item("   ")];
        assert_eq!(super::post_turn_delta(&items, 1), None);
    }
}
