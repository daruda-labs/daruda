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
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use daruda_acp::{
    AcpEvent, AcpSessionHandle, ChatItem, ConfigOptionView, ModeStateView, PermissionDecision,
    PermissionKindView, PlanEntryView, SlashCommand, apply_update, cancel_pending_tools,
    finalize_streaming, permission_item,
};
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use gpui::{
    AnyWindowHandle, App, Context, Entity, FocusHandle, Focusable, FollowMode, ListAlignment,
    ListState, ScrollHandle, Subscription, Task, Window, prelude::*, px,
};

use super::agent_chat_helpers::{
    DiffStat, apply_info_field, cancel_pending_permission, collect_foldable_keys, fold_active,
    trailing_unresolved_permission,
};
use super::fold::{FoldKey, FoldState};
use super::rows::{RenderRow, project};
use crate::workspace::main_area::file_view_pane::render::CachedImage;
use crate::workspace::main_area::pane_tree::PaneId;

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
    /// for prompts (handshake + `session/new` in flight).
    Connecting,
    /// `initialize` + `session/new` succeeded — the session accepts prompts and
    /// the event pump is folding updates into `items`.
    Connected,
    /// The connection or protocol failed; the message is surfaced both here
    /// (status line) and through the error pipeline.
    Error(String),
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
    /// pane was opened without a resolvable lane cwd.
    pub(in crate::workspace) cwd: Option<PathBuf>,
    /// Connection lifecycle state. Drives the status line + input/cancel
    /// affordance.
    pub(in crate::workspace) status: AgentSessionStatus,
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
    /// `None` on a connect failure. Dropping it (pane close) closes the command
    /// channel and shuts the connection task down.
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
    /// The id of the single in-flight permission request awaiting a host
    /// decision, if any. MVP serialises permissions: a new request replaces the
    /// previous pending id (the agent only asks one at a time within a turn).
    /// Cleared once the user responds.
    pub(in crate::workspace) pending_permission: Option<u64>,
    /// `true` between submitting a prompt and the matching `TurnEnded`. Drives
    /// the input affordance (Send ↔ Stop) and disables re-submit while the
    /// agent is busy.
    pub(in crate::workspace) turn_in_flight: bool,
    /// Wall-clock instant when the current turn started; `None` when idle.
    /// Runtime-only — not persisted.
    pub(in crate::workspace) turn_started_at: Option<std::time::Instant>,
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
    /// Map the view's internal state to a [`daruda_claude::SessionStatus`] for
    /// the lane indicator aggregation. Returns `None` for states that should
    /// not contribute an indicator (dormant `Idle` — session not yet started —
    /// and `Error` — broken session).
    pub(in crate::workspace) fn to_session_status(&self) -> Option<daruda_claude::SessionStatus> {
        use daruda_claude::SessionStatus;
        match &self.status {
            AgentSessionStatus::Idle | AgentSessionStatus::Error(_) => None,
            // Runtime prep is a connecting sub-phase — same pulsing badge.
            AgentSessionStatus::PreparingRuntime(_) | AgentSessionStatus::Connecting => {
                Some(SessionStatus::Connecting)
            }
            AgentSessionStatus::Connected => {
                if self.turn_in_flight {
                    if self.pending_permission.is_some() {
                        Some(SessionStatus::NeedsAttention)
                    } else {
                        Some(SessionStatus::Working)
                    }
                } else {
                    Some(SessionStatus::Idle)
                }
            }
        }
    }

    /// Build a fresh view. The session is *not* started here — the Workspace
    /// connects it lazily on first focus (`maybe_connect_agent_chat`), so cold
    /// restore doesn't spin up an agent process per pane. `status` is decided
    /// by the caller (Idle when a cwd is present, Error otherwise).
    pub(in crate::workspace) fn new(
        pane_id: PaneId,
        window_handle: AnyWindowHandle,
        cwd: Option<PathBuf>,
        status: AgentSessionStatus,
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
            items: Vec::new(),
            handle: None,
            pending_prompts: Vec::new(),
            _event_pump: None,
            pending_permission: None,
            turn_in_flight: false,
            turn_started_at: None,
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
            available_commands: Vec::new(),
            plan: Vec::new(),
            session_title: None,
            session_updated_at: None,
            plan_collapsed: false,
            plan_scroll: ScrollHandle::new(),
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

        match event {
            AcpEvent::Connected {
                modes,
                config_options,
            } => {
                self.status = AgentSessionStatus::Connected;
                self.modes = modes;
                self.config_options = config_options;
                // Clear stale plan/title from a previous session so they don't
                // flash before the new agent sends its first updates.
                self.plan.clear();
                self.session_title = None;
                self.session_updated_at = None;
                self.plan_collapsed = false;
            }
            AcpEvent::ConfigOptionsChanged(options) => {
                self.config_options = options;
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
                let effect = apply_update(&mut self.items, &update);
                touched_tool = effect.touched_tool;
                touched_text = effect.touched_text;
            }
            AcpEvent::PermissionRequested { id, request } => {
                self.items.push(permission_item(&request));
                self.pending_permission = Some(id);
            }
            AcpEvent::TurnEnded { .. } => {
                // Settle the turn: finalize streaming, cancel any tool the agent
                // left non-terminal (e.g. a `Cancelled` stop reason), and drain a
                // still-pending permission so no card keeps live buttons.
                self.settle_turn();
                // Drain the next buffered prompt (if any) now that the turn
                // completed — one per `TurnEnded`, so the queue advances a single
                // turn at a time and `turn_in_flight` keeps tracking exactly one
                // live turn. A no-op when nothing is buffered or on a cancelled
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
            AcpEvent::Error(message) => {
                self.status = AgentSessionStatus::Error(message);
                // A mid-turn failure must settle the turn like a Stop would —
                // otherwise a streaming block stays `streaming: true` and an
                // `InProgress` tool stays live, so the rollup glyph blinks
                // forever and the response bar reads `Running` after the session
                // is already dead.
                self.settle_turn();
                // The session is dead with no reconnect path, so any buffered
                // prompts can never be delivered — drop them (they were already
                // echoed locally) rather than leaving them to be pumped.
                self.pending_prompts.clear();
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
        // Reproject rows + sync the virtualized list. `FollowMode::Tail` keeps
        // the bottom pinned while streaming — no manual scroll needed.
        self.rebuild_rows();
        cx.notify();
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
        let awaiting_response = self.turn_in_flight && self.pending_permission.is_none();
        self.rows = project(&self.items, &self.fold, awaiting_response);

        if let Some(at) = old
            .iter()
            .zip(&self.rows)
            .position(|(a, b)| !a.same_slot(b))
        {
            self.list_state.splice(at..old.len(), self.rows.len() - at);
            return;
        }
        if old.len() != self.rows.len() {
            let at = old.len().min(self.rows.len());
            self.list_state.splice(at..old.len(), self.rows.len() - at);
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
            let n = self.rows.len();
            if n > 0 {
                self.list_state.remeasure_items(n - 1..n);
            }
        } else {
            self.list_state.remeasure_items(lo..hi + 1);
        }
    }

    /// Send `text` as a prompt: echo it locally, forward it over the session,
    /// and mark a turn in flight. Driven by the bottom-dock input via the
    /// Workspace shim (`send_agent_prompt_text`).
    pub(in crate::workspace) fn send_prompt_text(&mut self, text: String, cx: &mut Context<Self>) {
        // Echo locally so the prompt shows immediately even before the agent
        // streams it back as a user-message chunk.
        self.items.push(ChatItem::UserText(text.clone()));
        if let Some(handle) = &self.handle
            && !self.turn_in_flight
        {
            // Connected and idle: send now and mark the turn in flight.
            handle.send_prompt(text);
            self.turn_in_flight = true;
            self.turn_started_at = Some(std::time::Instant::now());
        } else {
            // Not connected yet (lazy connect happens on first focus), or a turn
            // is already in flight. Buffer the prompt in submission order — the
            // local echo above already shows it — and drain it one-per-turn via
            // `pump_pending_prompt` (at connect, then on each `TurnEnded`). Do
            // *not* set `turn_in_flight`: nothing new is on the wire yet.
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
        if self.turn_in_flight || self.pending_prompts.is_empty() {
            return;
        }
        let Some(handle) = self.handle.as_ref() else {
            return;
        };
        // FIFO: the front of the queue is the oldest buffered prompt.
        let text = self.pending_prompts.remove(0);
        handle.send_prompt(text);
        self.turn_in_flight = true;
        self.turn_started_at = Some(std::time::Instant::now());
        cx.notify();
    }

    /// Stop the active turn. Sends `session/cancel` *and* ends the turn locally,
    /// right now — it does not wait for the agent's stop reason.
    ///
    /// `session/cancel` is only cooperative: a hung or dead agent may never
    /// return a `Cancelled` stop reason, and `AcpEvent::TurnEnded` (the sole
    /// other place `turn_in_flight` clears) would then never arrive — leaving
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
        self.settle_turn();
        // Stop halts everything queued, not just the live turn. Drop any
        // buffered prompts so the cancelled turn's later `TurnEnded` finds an
        // empty buffer and `pump_pending_prompt` no-ops — otherwise Stop would
        // silently auto-fire the next queued prompt (the bottom-dock input does
        // not gate Send on `turn_in_flight`). The clear runs now, before the
        // cancelled `TurnEnded` arrives over the event pump.
        self.pending_prompts.clear();
        // The settle above mutates items (streaming → done, running tools →
        // cancelled, pending card → resolved), changing both fold visibility and
        // row heights, so reproject and remeasure before notifying.
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
        self.turn_in_flight = false;
        self.turn_started_at = None;
        finalize_streaming(&mut self.items);
        cancel_pending_tools(&mut self.items);
        cancel_pending_permission(self);
    }

    /// Resolve the pending permission request with the chosen option. Marks the
    /// matching card resolved, sends the decision over the session, and clears
    /// the pending id. `kind` selects Allow vs. Reject semantics.
    pub(in crate::workspace) fn respond_permission(
        &mut self,
        option_id: String,
        kind: PermissionKindView,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.pending_permission.take() else {
            return;
        };
        // Mark the trailing unresolved permission card resolved so the buttons
        // disable and the choice shows.
        if let Some(card) = trailing_unresolved_permission(self) {
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
            handle.respond_permission(id, decision);
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
