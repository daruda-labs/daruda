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
    AcpEvent, AcpSessionHandle, ChatItem, ModeStateView, PermissionDecision, PermissionKindView,
    PlanEntryView, SlashCommand, apply_update, finalize_streaming, permission_item,
};
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use gpui::{
    AnyWindowHandle, App, Context, Entity, FocusHandle, Focusable, FollowMode, ListAlignment,
    ListState, Task, Window, prelude::*, px,
};

use super::agent_chat_ops::{
    DiffStat, build_diff_view_model, cancel_pending_permission, chat_item_markdown,
    collect_foldable_keys, create_diff_editor, diff_editor_key, diff_editor_language, is_active,
    mermaid_key, mermaid_sources, trailing_unresolved_permission,
};
use super::fold::{FoldKey, FoldState};
use super::rows::{RenderRow, project};
use crate::workspace::main_area::file_view_pane::diff_editor::{DiffColors, DiffEditorModel};
use crate::workspace::main_area::file_view_pane::markdown_viewer::mermaid_with_theme;
use crate::workspace::main_area::file_view_pane::render::CachedImage;
use crate::workspace::main_area::file_view_pane::visual;
use crate::workspace::main_area::pane_tree::PaneId;

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
    /// Slash commands advertised by the agent via `AvailableCommandsChanged`.
    /// Runtime-only; never serialized. Consumed by the slash-command
    /// autocomplete provider (later task).
    pub(in crate::workspace) available_commands: Vec<SlashCommand>,
    /// The agent's live execution plan (`PlanChanged`); full-replaced each update.
    /// Runtime-only; never serialized.
    pub(in crate::workspace) plan: Vec<PlanEntryView>,
    /// Agent-provided session title (`SessionTitleChanged`); `None` = fallback label.
    pub(in crate::workspace) session_title: Option<String>,
    /// Whether the bottom plan region is collapsed to its header. Transient /
    /// session-only; defaults to `false` (expanded) so a fresh plan shows its
    /// checklist. Toggled by the header click via [`Self::toggle_plan_collapsed`].
    pub(in crate::workspace) plan_collapsed: bool,
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
            AgentSessionStatus::Connecting => Some(SessionStatus::Connecting),
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
        Self {
            pane_id,
            window_handle,
            focus_handle: cx.focus_handle(),
            cwd,
            status,
            items: Vec::new(),
            handle: None,
            _event_pump: None,
            pending_permission: None,
            turn_in_flight: false,
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
            available_commands: Vec::new(),
            plan: Vec::new(),
            session_title: None,
            plan_collapsed: false,
            #[cfg(test)]
            render_count: std::cell::Cell::new(0),
        }
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

        match event {
            AcpEvent::Connected { modes } => {
                self.status = AgentSessionStatus::Connected;
                self.modes = modes;
                // Clear stale plan/title from a previous session so they don't
                // flash before the new agent sends its first updates.
                self.plan.clear();
                self.session_title = None;
                self.plan_collapsed = false;
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
            AcpEvent::Update(update) => apply_update(&mut self.items, &update),
            AcpEvent::PermissionRequested { id, request } => {
                self.items.push(permission_item(&request));
                self.pending_permission = Some(id);
            }
            AcpEvent::TurnEnded { .. } => {
                finalize_streaming(&mut self.items);
                self.turn_in_flight = false;
                // A turn only ends with a permission still pending when it was
                // cancelled / refused mid-request — drain it so no card is left
                // with live buttons (no-op on a normal turn).
                cancel_pending_permission(self);
            }
            AcpEvent::AvailableCommandsChanged(commands) => {
                self.available_commands = commands;
            }
            // Full-replace the plan; `plan_collapsed` is intentionally preserved — the
            // agent updates the plan frequently mid-turn, so auto-expanding on every
            // update would fight the user. First arrival shows expanded via the
            // `plan_collapsed: false` default.
            AcpEvent::PlanChanged(entries) => {
                self.plan = entries;
            }
            AcpEvent::SessionTitleChanged(title) => {
                self.session_title = title;
            }
            AcpEvent::Notice(_) => {
                // Logged above; no status change.
            }
            AcpEvent::Error(message) => {
                self.status = AgentSessionStatus::Error(message);
                self.turn_in_flight = false;
                cancel_pending_permission(self);
            }
        }
        self.reconcile_diff_editors(syntax_theme, is_light, cx);
        self.reconcile_mermaid(!is_light, cx);
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
        if let Some(handle) = &self.handle {
            handle.send_prompt(text);
            self.turn_in_flight = true;
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

    /// Request cancellation of the active turn. Sends `session/cancel` and, per
    /// ACP, resolves any permission request still awaiting the user with a
    /// cancelled outcome.
    pub(in crate::workspace) fn cancel_turn(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = &self.handle {
            handle.cancel();
        }
        cancel_pending_permission(self);
        // Resolving the card flips it from pending → resolved, so its row no
        // longer force-stays-visible under a collapsed response: reproject so it
        // folds back into the process immediately. A drained card also swaps its
        // buttons for a one-line outcome (height change), so remeasure too.
        self.rebuild_rows();
        self.list_state.remeasure();
        cx.notify();
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
        let active = match &key {
            FoldKey::Assistant(ix) | FoldKey::Thinking(ix) => {
                self.items.get(*ix).map(is_active).unwrap_or(false)
            }
            FoldKey::Tool(id) => self
                .items
                .iter()
                .find_map(|item| match item {
                    ChatItem::ToolCall(tc) if tc.id == *id => Some(is_active(item)),
                    _ => None,
                })
                .unwrap_or(false),
            // A response is active while it is the last turn or its run is
            // still streaming — matching `rows::project`'s default derivation.
            FoldKey::Response(anchor) => {
                let start = anchor + 1;
                let end = self
                    .items
                    .iter()
                    .skip(start)
                    .position(|it| matches!(it, ChatItem::UserText(_)))
                    .map(|off| start + off)
                    .unwrap_or(self.items.len());
                let is_last = end >= self.items.len();
                let streaming = self
                    .items
                    .get(start..end)
                    .is_some_and(|run| run.iter().any(is_active));
                is_last || streaming
            }
            // The group is the consecutive tool-call run beginning at `gid`;
            // active while any tool in it is still running.
            FoldKey::ToolGroup(gid) => self
                .items
                .iter()
                .position(|item| matches!(item, ChatItem::ToolCall(tc) if tc.id == *gid))
                .map(|s| {
                    self.items[s..]
                        .iter()
                        .take_while(|item| matches!(item, ChatItem::ToolCall(_)))
                        .any(is_active)
                })
                .unwrap_or(false),
            // Diff policy is DefaultCollapsed → derivation ignores `active`.
            FoldKey::Diff(_) => false,
        };
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

    /// Jump the conversation list to the bottom and re-engage `Tail` follow.
    /// Backs the floating scroll-to-bottom button. The list internally re-arms
    /// tail-following once it lands at the end, so a fresh streaming chunk keeps
    /// sticking to the bottom.
    pub(in crate::workspace) fn scroll_to_bottom(&mut self, cx: &mut Context<Self>) {
        self.list_state.scroll_to_end();
        cx.notify();
    }

    /// Build the read-only diff editor entity for every tool-call file
    /// modification that does not yet have one. Called from `apply_event` after
    /// `items` mutates, so the (cached) subtree shows the diff through the same
    /// editor the File viewer uses rather than the inline fallback.
    ///
    /// Keyed by `"{tool_call_id}#{diff_index}"` — one editor per file. A diff is
    /// converted to a `DiffEditorModel` purely (no GPUI), then the editor entity
    /// is created + configured inside a single window re-entry against the
    /// stored `window_handle`. Build-once: keys are only filled when absent.
    fn reconcile_diff_editors(
        &mut self,
        syntax_theme: &str,
        is_light: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(colors) = cx
            .try_global::<crate::ui::theme::DarudaTheme>()
            .map(DiffColors::from_theme)
        else {
            // Theme global not yet installed (transient cold-start) — skip
            // editor creation; every diff renders via the inline fallback.
            // Logged so the blanket fallback isn't a silent no-op.
            daruda_store::observability::log_writer::LogWriter::log(
                ErrorReport::new("Skipping agent-chat diff editors: theme global absent")
                    .severity(ErrorSeverity::Warning)
                    .at(file!(), line!())
                    .dedup(format!(
                        "agent_chat.diff_editor.theme_missing.{}",
                        self.pane_id
                    ))
                    .build(),
            );
            return;
        };

        // Collect the pure work first; entity creation re-enters the window,
        // which can't happen while the immutable `items` borrow is live.
        let mut pending: Vec<(String, String, DiffEditorModel, DiffStat)> = Vec::new();
        for item in &self.items {
            let ChatItem::ToolCall(tc) = item else {
                continue;
            };
            for (di, diff) in tc.diffs.iter().enumerate() {
                let key = diff_editor_key(&tc.id, di);
                if self.diff_editors.contains_key(&key) {
                    continue;
                }
                let Some((model, stat)) =
                    build_diff_view_model(diff, syntax_theme, is_light, &colors)
                else {
                    continue;
                };
                let language = diff_editor_language(diff).to_owned();
                pending.push((key, language, model, stat));
            }
        }
        if pending.is_empty() {
            return;
        }

        let window_handle = self.window_handle;
        let pane_id = self.pane_id;
        for (key, language, model, stat) in pending {
            if let Some(editor) = create_diff_editor(cx, window_handle, pane_id, &language, model) {
                // Cache the stat under the same key as the editor so the fold
                // summary (`+N −M`) reads it back via `diff_editor_key`. Stored
                // only when the editor builds — a no-change diff yields no
                // editor and no stat (absent ≡ `0/0`).
                self.diff_stats.insert(key.clone(), stat);
                self.diff_editors.insert(key, editor);
            }
        }
        // We only reach here when there was pending diff work, so a tool card
        // just grew (embedded editor, or the inline fallback's diff lines). The
        // tool call may sit mid-list (a `ToolCallUpdate` to an earlier call), so
        // a full remeasure is needed — `sync_list_after` only remeasures the
        // tail. Runs once per diff-bearing event, not per frame.
        self.list_state.remeasure();
    }

    /// Rasterize every ` ```mermaid ` fence in the conversation that does not
    /// yet have a cached bitmap (and isn't already being rendered). Collect the
    /// pure work first, then spawn each rasterization on the background executor
    /// (selkie is CPU-heavy and can panic), and re-enter the view to fill the
    /// cache + `cx.notify()` when it lands.
    ///
    /// `dark` matches the diagram theme to the host appearance so edges stay
    /// visible. Theme-switch staleness (a cached raster keeps its colour after a
    /// light/dark toggle) is out of scope; the cache is only ever added to.
    fn reconcile_mermaid(&mut self, dark: bool, cx: &mut Context<Self>) {
        // Collect the not-yet-cached, not-in-flight sources first; the spawn
        // re-enters the view, which can't happen while the `items` borrow is
        // live.
        let mut pending: Vec<(u64, String)> = Vec::new();
        for item in &self.items {
            let Some(text) = chat_item_markdown(item) else {
                continue;
            };
            for source in mermaid_sources(text) {
                let key = mermaid_key(&source);
                if self.mermaid_images.lock().unwrap().contains_key(&key)
                    || self.mermaid_inflight.contains(&key)
                    || pending.iter().any(|(k, _)| *k == key)
                {
                    continue;
                }
                pending.push((key, source));
            }
        }
        if pending.is_empty() {
            return;
        }

        // Mark all pending keys in-flight before spawning so a second event
        // arriving before any task resolves doesn't re-spawn the same source.
        for (key, _) in &pending {
            self.mermaid_inflight.insert(*key);
        }

        for (key, source) in pending {
            cx.spawn(async move |this, cx| {
                let raster = cx
                    .background_executor()
                    .spawn(async move {
                        let themed = mermaid_with_theme(&source, dark);
                        // selkie is a young reimplementation; guard against a
                        // panic on malformed input so one bad diagram can't take
                        // the executor down — on panic / error we drop it and the
                        // fence keeps the default code rendering.
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            selkie::render::render_text(&themed)
                                .ok()
                                .and_then(|svg| visual::rasterize_svg(&svg).ok())
                        }))
                        .ok()
                        .flatten()
                    })
                    .await;
                // SILENT-OK: view/window dropped before the raster resolved — nothing left to cache it on.
                let _ = this.update(cx, |view, cx| {
                    view.mermaid_inflight.remove(&key);
                    // Convert the raster to a GPU-ready image once, here, so the
                    // render hook clones the same `CachedImage` each frame and
                    // gpui reuses the uploaded texture.
                    if let Some(image) = raster.and_then(|r| CachedImage::from_raster(&r)) {
                        view.mermaid_images.lock().unwrap().insert(key, image);
                        // The fence's item grows from a code block to a diagram —
                        // its cached height in the virtualized list is now stale,
                        // so remeasure before repainting or the diagram clips /
                        // leaves a gap. (Index unknown here; a full remeasure is a
                        // one-shot when the raster lands, not a per-frame cost.)
                        view.list_state.remeasure();
                        cx.notify();
                    }
                });
            })
            .detach();
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
        #[cfg(test)]
        self.render_count.set(self.render_count.get() + 1);
        super::render::render(self, cx)
    }
}
