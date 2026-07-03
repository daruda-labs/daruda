//! Workspace ops for the Agent chat pane — pane/tab construction, the live
//! ACP connection + event pump, the bottom-dock prompt / cancel routing, plus
//! the GPUI-free helpers the view's reconcilers reuse.
//!
//! The per-event folding and every render-listener op (`toggle_fold`,
//! `on_scroll`, `respond_permission`, `set_mode`, …) live on
//! [`AgentChatView`](super::view::AgentChatView); they operate on the view and
//! `cx.notify()` the view, so a scroll / fold dirties only that cached subtree.
//! What stays here are the parts that need `Workspace` state — the connection
//! reads `agent.default_permission_mode`; the pump reads `syntax_theme` per
//! event and owns the error pipeline (`report_error`) — and the construction
//! that mutates the pane/tab tree.
//!
//! ## Connection + pump shape
//!
//! ```text
//!   create_agent_chat_pane
//!         │  builds Pane (status = Idle, handle = None) — no session yet
//!         ▼
//!   focus_pane → maybe_connect_agent_chat
//!         │  first focus only: status Idle → Connecting
//!         ▼
//!   connect_agent_chat (cx.spawn, weak Workspace)
//!         │  connect_session on bg executor → (handle, rx)
//!         │  store handle on the view, fold events through view.apply_event
//!         ▼
//!   event pump: while rx.next().await:
//!           view.update(|v, cx| v.apply_event(event, &syntax_theme, is_light, cx))
//!         each event notifies the *view* (cached subtree), never the Workspace
//! ```
//!
//! Both the handle and the pump task live on the view, so closing the pane
//! drops them: the handle drop closes the command channel (the connection task
//! exits) and the pump-task drop ends the loop. No explicit teardown is needed.

use daruda_acp::{DiffView, NodeProgress, connect_session_with_node};
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use futures::StreamExt as _;
use futures::channel::mpsc::unbounded;
use gpui::{AnyWindowHandle, AppContext as _, Context, Entity, Window};

use super::fold::{FoldKey, FoldState};
use super::rows::{RowKind, project};
use super::view::{AgentChatView, AgentSessionStatus, RuntimePrepPhase};
use crate::path_ext::PathExt as _;
use crate::surface::strings as s;
use crate::workspace::Workspace;
use crate::workspace::main_area::file_view_pane::diff_editor::{
    DiffColors, DiffEditorModel, build_diff_editor_model,
};
use crate::workspace::main_area::pane::{AgentChatContent, Pane, PaneContent, TabEntry};
use crate::workspace::main_area::pane_tree::{PaneId, PaneLayout};

/// The banner phase for a runtime-provisioning milestone, or `None` for
/// milestones that shouldn't surface a banner (system node found, or a cache
/// probe — both instant, so the plain "Connecting…" banner already fits).
fn runtime_prep_phase(progress: NodeProgress) -> Option<RuntimePrepPhase> {
    match progress {
        NodeProgress::UsingSystemNode | NodeProgress::CheckingCache => None,
        NodeProgress::Downloading => Some(RuntimePrepPhase::Downloading),
        NodeProgress::Verifying => Some(RuntimePrepPhase::Verifying),
        NodeProgress::Extracting => Some(RuntimePrepPhase::Extracting),
    }
}

impl Workspace {
    /// Construct an Agent chat `Pane` (no tab side-effects). Allocates the pane
    /// id and builds the `Entity<AgentChatView>`, seeding the conversation as
    /// empty and parking the session in `Idle` (or `Error` when there is no
    /// lane cwd to attach to). The live ACP session is *not* started here —
    /// [`Self::focus_pane`] connects it lazily on first focus (via
    /// [`Self::maybe_connect_agent_chat`]), so cold restore doesn't spin up an
    /// agent process per pane. The prompt input is the shared bottom-dock
    /// input, not a per-pane field. The `window` is needed only to capture the
    /// window handle the view stores for later diff-editor creation.
    pub(in crate::workspace) fn create_agent_chat_pane(
        &mut self,
        cwd: Option<std::path::PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Pane {
        let pane_id = self.alloc_id();
        let window_handle = window.window_handle();
        // The connection roots at the lane cwd; without one there is no working
        // directory to attach the agent to. Park such a pane in an error state
        // rather than a dormant `Idle` that could never connect. The cwd case
        // stays `Idle` until first focus. The status banner re-adds the error
        // prefix, so carry the bare reason here — not the prefix.
        let status = match &cwd {
            Some(_) => AgentSessionStatus::Idle,
            None => AgentSessionStatus::Error(s::agent_chat_no_lane_cwd()),
        };
        // The view owns its own `cwd` (for connect / persistence); the wrapper
        // caches a copy so `Pane::cwd()` stays cx-free.
        let view = cx.new({
            let cwd = cwd.clone();
            move |cx| AgentChatView::new(pane_id, window_handle, cwd, status, cx)
        });
        Pane {
            id: pane_id,
            content: PaneContent::AgentChat(AgentChatContent {
                view,
                cached_title: s::agent_chat_tab_title().into(),
                cwd,
            }),
        }
    }

    /// Open a fresh Agent chat pane in a new tab, anchored at the active lane's
    /// working directory. Mirrors `open_task_edit_pane`'s tab-append + focus
    /// flow.
    pub(in crate::workspace) fn open_agent_chat_pane(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // An inaccessible active lane renders the empty-state; opening a pane
        // there would escape that state (mirrors `add_tab`).
        if self.active_lane_is_inaccessible() {
            return;
        }
        let cwd = self.active_lane().map(|w| w.path.clone());
        let pane = self.create_agent_chat_pane(cwd, window, cx);
        let pane_id = pane.id;
        let tab_id = self.alloc_id();
        self.active_runtime_mut().panes.push(pane);
        self.active_runtime_mut().tabs.push(TabEntry {
            id: tab_id,
            layout: PaneLayout::Pane(pane_id),
            last_focused_pane: pane_id,
            user_label: None,
        });
        let cur_tab = self.active_runtime().active_tab_index;
        self.active_runtime_mut().tab_history.push(cur_tab);
        let last_tab = self.active_runtime().tabs.len() - 1;
        self.active_runtime_mut().active_tab_index = last_tab;
        // The live session is not started here — `focus_pane` below connects it
        // lazily (`maybe_connect_agent_chat`), the same path a restored pane
        // takes on first focus. The prompt input lives in the bottom dock, not
        // the pane. Open the dock first so the input is visible before
        // `focus_pane` activates the input panel, syncs the placeholder, and
        // moves keyboard focus to it for AgentChat panes. The focused *pane*
        // stays this one, so `send_terminal_input` routes to its ACP session.
        if !self.bottom_dock.read(cx).is_open {
            self.bottom_dock.update(cx, |d, cx| {
                d.toggle();
                cx.notify();
            });
            self.main_area.pending_resize = true;
        }
        self.set_focused_pane(pane_id, cx);
        self.bump_activity(pane_id);
        self.focus_pane(pane_id, window, cx);
        self.resize_all_tabs(window, cx);
        cx.notify();
    }

    /// Lazy-connect entry point: start the ACP session for `pane_id` iff it is
    /// an Agent chat pane still parked in [`AgentSessionStatus::Idle`] with a
    /// working directory. Called from [`Self::focus_pane`] so the session
    /// attaches on first focus and never twice (the `Idle` guard short-circuits
    /// once a connect is in flight or has resolved). A no-cwd pane is already
    /// parked in `Error`, so it is skipped here.
    pub(in crate::workspace) fn maybe_connect_agent_chat(
        &mut self,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) {
        let cwd = {
            let Some(view) = self.agent_chat_view(pane_id) else {
                return;
            };
            let v = view.read(cx);
            if !matches!(v.status, AgentSessionStatus::Idle) {
                return;
            }
            let Some(cwd) = v.cwd.clone() else {
                return;
            };
            cwd
        };
        // Flip to `Connecting` before spawning so a second focus during the
        // handshake doesn't start a duplicate session.
        if let Some(view) = self.agent_chat_view(pane_id).cloned() {
            view.update(cx, |v, cx| {
                v.status = AgentSessionStatus::Connecting;
                cx.notify();
            });
            // Idle → Connecting is a dock-badge status change; the cached
            // docks need an explicit dirty (see `notify_status_docks`).
            self.notify_status_docks(cx);
        }
        self.connect_agent_chat(pane_id, cwd, cx);
    }

    /// Open the live ACP session for an already-pushed Agent chat pane and store
    /// the event-pump task on its view. Runs the (synchronous-to-parse, then
    /// async) connect on the background executor, then re-enters the workspace
    /// to store the handle and fold events through the view.
    ///
    /// The spawned task is stored in the view's `_event_pump`, so closing the
    /// pane drops it (ending the loop) in addition to dropping the session
    /// handle (which closes the connection).
    pub(in crate::workspace) fn connect_agent_chat(
        &mut self,
        pane_id: PaneId,
        cwd: std::path::PathBuf,
        cx: &mut Context<Self>,
    ) {
        let initial_mode = Some(self.agent.default_permission_mode.mode_id().to_string());
        let node_root = daruda_store::persistence::node_install_dir();

        // Runtime provisioning (see `connect_session_with_node`) can download
        // Node.js on the first run of a machine without a usable system install.
        // Milestones flow over this channel to a foreground drain that shows a
        // "preparing runtime…" banner, so a slow first-run download doesn't look
        // like a hang. The sender lives in the background task; when it finishes,
        // the sender drops and the drain ends.
        let (progress_tx, mut progress_rx) = unbounded::<NodeProgress>();
        cx.spawn(async move |this, cx| {
            while let Some(progress) = progress_rx.next().await {
                let Some(phase) = runtime_prep_phase(progress) else {
                    continue;
                };
                let cont = this.update(cx, |ws, cx| {
                    let Some(view) = ws.agent_chat_view(pane_id).cloned() else {
                        return false;
                    };
                    view.update(cx, |v, cx| {
                        // Only advance the banner while still in a connecting
                        // phase — never clobber a Connected/Error terminal state
                        // (the connect and drain tasks race to completion).
                        if matches!(
                            v.status,
                            AgentSessionStatus::Connecting
                                | AgentSessionStatus::PreparingRuntime(_)
                        ) {
                            v.status = AgentSessionStatus::PreparingRuntime(phase);
                            cx.notify();
                        }
                    });
                    true
                });
                if !matches!(cont, Ok(true)) {
                    break;
                }
            }
        })
        .detach();

        let pump = cx.spawn(async move |this, cx| {
            // `connect_session_with_node` is synchronous (it provisions node,
            // parses the command, and spawns the connection task); run it on the
            // background executor so the download / smol `spawn` bind to a worker
            // thread rather than the main loop. The progress sender is moved in
            // and dropped when this closure returns, ending the drain above.
            let connected = cx
                .background_executor()
                .spawn(async move {
                    let mut progress = move |milestone| drop(progress_tx.unbounded_send(milestone));
                    connect_session_with_node(node_root, cwd, initial_mode, &mut progress)
                })
                .await;

            match connected {
                Ok((handle, mut events)) => {
                    // Store the handle on the view and clear any lingering
                    // "preparing runtime" banner — the adapter is now spawning
                    // (handshake in flight), so the state is plain Connecting
                    // until the event pump reports it live. If the view/window is
                    // already gone, drop the handle (closing the session).
                    let stored = this.update(cx, |ws, cx| {
                        if let Some(view) = ws.agent_chat_view(pane_id).cloned() {
                            view.update(cx, |v, cx| {
                                v.handle = Some(handle);
                                if matches!(v.status, AgentSessionStatus::PreparingRuntime(_)) {
                                    v.status = AgentSessionStatus::Connecting;
                                    cx.notify();
                                }
                                // Forward the first prompt submitted before the
                                // handle existed (queued during the handshake or
                                // dispatched into the pane before it connected).
                                // One per turn: each `TurnEnded` pumps the next,
                                // so the view tracks a single live turn. No-op
                                // when nothing was buffered.
                                v.pump_pending_prompt(cx);
                            });
                            true
                        } else {
                            false
                        }
                    });
                    if !matches!(stored, Ok(true)) {
                        return;
                    }

                    // Pump the event stream until end-of-stream (handle dropped
                    // on pane close, or terminal protocol error). Each event is
                    // folded through the view, which notifies itself.
                    while let Some(event) = events.next().await {
                        let cont = this.update(cx, |ws, cx| {
                            let (syntax_theme, is_light) = ws.agent_chat_theme_params(cx);
                            let Some(view) = ws.agent_chat_view(pane_id).cloned() else {
                                return false;
                            };
                            // The displayed session status (Working / Idle /
                            // NeedsAttention / …) feeds the cached per-lane
                            // dock badge. `apply_event` only self-notifies the
                            // view, so dirty the docks when the status actually
                            // changes — including for a *parked* lane, whose
                            // badge would otherwise freeze at its last animating
                            // frame once the pulse stops. Gated on change so
                            // token-streaming events don't repaint the docks.
                            let before = view.read(cx).to_session_status();
                            // Capture current mode before the event so we can
                            // detect `Connected` (modes arriving) and
                            // `ModeChanged` (current switching) and refresh the
                            // bottom-input placeholder when either fires.
                            let mode_before =
                                view.read(cx).modes.as_ref().map(|m| m.current.clone());
                            // AgentChat-surfaced tasks reconcile off the ACP turn
                            // lifecycle (they never write the status-file hooks
                            // the Terminal surface uses). Capture the outcome
                            // before the event is consumed: a completed turn maps
                            // to Done (via `Stop`), a terminal error to Error.
                            let task_end_reason = match &event {
                                daruda_acp::AcpEvent::TurnEnded { .. } => {
                                    Some(daruda_store::tasks::SessionEndReason::Stop)
                                }
                                daruda_acp::AcpEvent::Error(_) => {
                                    Some(daruda_store::tasks::SessionEndReason::Error)
                                }
                                _ => None,
                            };
                            view.update(cx, |v, cx| {
                                v.apply_event(event, &syntax_theme, is_light, cx)
                            });
                            if view.read(cx).to_session_status() != before {
                                ws.notify_status_docks(cx);
                            }
                            // Reconcile the backing task (if any) keyed by the
                            // pane's lane cwd. A no-op when no `Running` task
                            // matches (plain agent-chat pane) or the cwd is
                            // absent.
                            if let Some(reason) = task_end_reason
                                && let Some(cwd) = view.read(cx).cwd.clone()
                            {
                                ws.apply_agent_chat_task_ended(&cwd, reason, cx);
                            }
                            // Refresh placeholder when the active mode changed or
                            // modes became available (Connected). Only fires for
                            // the focused pane to avoid redundant work on parked
                            // lane views.
                            let mode_after =
                                view.read(cx).modes.as_ref().map(|m| m.current.clone());
                            let focused_id = ws.active_runtime().focused_pane_id;
                            if mode_before != mode_after && focused_id == pane_id {
                                ws.refresh_terminal_input_placeholder(cx);
                            }
                            true
                        });
                        // Workspace/window gone (Err) or view gone (Ok(false)) —
                        // stop pumping.
                        if !matches!(cont, Ok(true)) {
                            break;
                        }
                    }
                }
                Err(err) => {
                    let message = format!("{err}");
                    // workspace gone before the connect resolved — nothing left
                    // to surface the failure on.
                    // SILENT-OK: workspace/window dropped before connect resolved
                    let _ = this.update(cx, |ws, cx| {
                        if let Some(view) = ws.agent_chat_view(pane_id).cloned() {
                            view.update(cx, |v, cx| {
                                v.status = AgentSessionStatus::Error(message.clone());
                                cx.notify();
                            });
                            // Connecting → Error clears the badge (maps to
                            // `None`); dirty the cached docks so the stale
                            // Connecting badge doesn't linger after the pulse
                            // stops.
                            ws.notify_status_docks(cx);
                            // A connect failure ends any AgentChat-surfaced task
                            // rooted at this lane in `Error` (it can never run),
                            // keyed by cwd since ACP writes no status-file hooks.
                            if let Some(cwd) = view.read(cx).cwd.clone() {
                                ws.apply_agent_chat_task_ended(
                                    &cwd,
                                    daruda_store::tasks::SessionEndReason::Error,
                                    cx,
                                );
                            }
                        }
                        let report = ErrorReport::new("ACP session connect failed")
                            .severity(ErrorSeverity::Error)
                            .with_context("detail", message)
                            .at(file!(), line!())
                            .dedup("agent_chat.connect")
                            .build();
                        ws.report_error(report, cx);
                    });
                }
            }
        });
        // Store the pump on the view so a pane close drops it (ending the loop)
        // on top of dropping the session handle.
        if let Some(view) = self.agent_chat_view(pane_id).cloned() {
            view.update(cx, |v, _| v._event_pump = Some(pump));
        }
    }

    /// Send `text` as a prompt to an Agent chat pane. Shim for the bottom-dock
    /// input: routes into the view, which echoes the prompt locally, forwards it
    /// over the session, and marks a turn in flight.
    pub(in crate::workspace) fn send_agent_prompt_text(
        &mut self,
        pane_id: PaneId,
        text: String,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.agent_chat_view(pane_id).cloned() {
            view.update(cx, |v, cx| v.send_prompt_text(text, cx));
        }
    }

    /// Request cancellation of the active turn. Shim for the bottom-dock "Stop"
    /// button: routes into the view, which sends `session/cancel` and drains any
    /// pending permission request.
    pub(in crate::workspace) fn cancel_agent_turn(
        &mut self,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.agent_chat_view(pane_id).cloned() {
            view.update(cx, |v, cx| v.cancel_turn(cx));
        }
    }

    /// Cancel `pane_id`'s turn only when one is actually in flight. Backs the
    /// Escape shortcut (the keyboard counterpart of the "Stop" button): returns
    /// `true` when it cancelled, `false` when `pane_id` is not an Agent chat
    /// pane or has no turn running — in which case the caller lets Escape
    /// propagate as usual. Mirrors the `agent_stop_pane` snapshot condition that
    /// shows the Stop button.
    pub(in crate::workspace) fn cancel_agent_turn_if_in_flight(
        &mut self,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(view) = self.agent_chat_view(pane_id).cloned() else {
            return false;
        };
        if !view.read(cx).turn_in_flight {
            return false;
        }
        view.update(cx, |v, cx| v.cancel_turn(cx));
        true
    }

    /// Switch the active session mode of an Agent chat pane. Shim for the
    /// bottom-dock mode chip: routes the chosen mode id into the focused pane's
    /// view, which optimistically updates the chip and sends `session/set_mode`.
    /// No-op when `pane_id` is gone or is not an Agent chat pane.
    pub(in crate::workspace) fn set_agent_mode(
        &mut self,
        pane_id: PaneId,
        mode_id: String,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.agent_chat_view(pane_id).cloned() {
            view.update(cx, |v, cx| v.set_mode(mode_id, cx));
        }
        // The bottom-input placeholder includes the current mode name;
        // refresh it now that the mode has changed.
        self.refresh_terminal_input_placeholder(cx);
    }

    /// Change a select config option (model / effort / …) of an Agent chat
    /// pane. Shim for the bottom-dock config chips: routes the chosen
    /// `(config_id, value)` into the focused pane's view, which optimistically
    /// updates the chip and sends `session/set_config_option`. No-op when
    /// `pane_id` is gone or is not an Agent chat pane.
    pub(in crate::workspace) fn set_agent_config_option(
        &mut self,
        pane_id: PaneId,
        config_id: String,
        value: String,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.agent_chat_view(pane_id).cloned() {
            view.update(cx, |v, cx| v.set_config_option(config_id, value, cx));
        }
    }

    /// Advance an Agent chat pane's session mode to the next advertised mode,
    /// wrapping at the end. Backs the bottom-input Shift+Tab shortcut (mirrors
    /// Claude Code's permission-mode cycle). Returns `true` when it switched the
    /// mode; `false` (no switch) when `pane_id` is not an Agent chat pane or it
    /// advertises fewer than two modes — the caller then lets Shift+Tab outdent.
    pub(in crate::workspace) fn cycle_agent_mode(
        &mut self,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(view) = self.agent_chat_view(pane_id).cloned() else {
            return false;
        };
        let Some(next) = view.read(cx).modes.as_ref().and_then(next_mode_id) else {
            return false;
        };
        view.update(cx, |v, cx| v.set_mode(next, cx));
        // The bottom-input placeholder includes the current mode name;
        // refresh it now that the mode has cycled.
        self.refresh_terminal_input_placeholder(cx);
        true
    }

    /// The (syntax theme, is-light) pair the Markdown / diff reconcilers read
    /// from the active theme. `is_light = !is_dark`, mirroring the file-viewer
    /// loader; defaults to dark (`is_light = false`) when the theme global is
    /// not yet installed. The Workspace owns `syntax_theme` (config mirror), so
    /// it reads it here and passes it to the view per event.
    pub(in crate::workspace) fn agent_chat_theme_params(
        &self,
        cx: &Context<Self>,
    ) -> (String, bool) {
        let is_light = cx
            .try_global::<crate::ui::theme::DarudaTheme>()
            .map(crate::ui::theme::DarudaTheme::is_dark)
            .map(|dark| !dark)
            .unwrap_or(false);
        (self.syntax_theme.clone(), is_light)
    }

    /// True when `pane_id` is an Agent chat pane — lets the bottom-dock input
    /// route prompts to the session instead of a PTY.
    pub(in crate::workspace) fn is_agent_chat_pane(&self, pane_id: PaneId) -> bool {
        self.active_runtime()
            .panes
            .iter()
            .any(|p| p.id == pane_id && p.agent_chat_view().is_some())
    }

    /// Look up an AgentChat pane's view entity by id across every lane's
    /// panes in the single `runtimes` map. Returns `None` when the pane is
    /// gone or is not an AgentChat pane.
    ///
    /// Scanning every lane is essential, not a convenience: the view's
    /// event pump looks the view up by id on every ACP event, and a lane
    /// switch only re-points `self.active` — the pane stays in its lane's
    /// runtime, which is no longer the active one. An active-lane-only
    /// lookup would then return `None`, the pump would treat it as "view
    /// gone" and break its loop, and the session's remaining responses
    /// would be dropped forever — even after switching back. Pane ids are
    /// workspace-global, so scanning every runtime is unambiguous.
    pub(in crate::workspace) fn agent_chat_view(
        &self,
        pane_id: PaneId,
    ) -> Option<&Entity<AgentChatView>> {
        self.main_area
            .runtimes
            .values()
            .flat_map(|rt| rt.panes.iter())
            .find(|p| p.id == pane_id)?
            .agent_chat_view()
    }
}

/// The id of the mode after `modes.current` in advertised order, wrapping at
/// the end. `None` when fewer than two modes are advertised (nothing to cycle).
/// If `current` is not in the list, cycling starts from the first mode. Pure
/// logic for `Workspace::cycle_agent_mode` (Shift+Tab).
fn next_mode_id(modes: &daruda_acp::ModeStateView) -> Option<String> {
    if modes.available.len() < 2 {
        return None;
    }
    let current = modes
        .available
        .iter()
        .position(|m| m.id == modes.current)
        .unwrap_or(0);
    let next = (current + 1) % modes.available.len();
    Some(modes.available[next].id.clone())
}

/// The visible foldable-key set for a conversation: each assistant / thinking
/// item by index, each tool call by id plus one `Diff` key per diff it carries
/// (the same `diff_editor_key` the renderer embeds with). User / permission /
/// error items are not foldable and contribute none. Single source of truth for
/// expand-all / collapse-all (`AgentChatView::set_all_folds`) and the coverage
/// test.
pub(in crate::workspace) fn collect_foldable_keys(items: &[daruda_acp::ChatItem]) -> Vec<FoldKey> {
    let mut keys: Vec<FoldKey> = Vec::new();
    // Structural fold levels (response / tool-group) come from the same row
    // projection the renderer uses, so expand/collapse-all covers exactly the
    // headers on screen. The fold state doesn't affect which headers exist, so
    // project against the default.
    // Fold coverage is independent of live progress, so project with no
    // in-flight turn (the working indicator carries no fold key).
    let rows = project(items, &FoldState::default(), false);
    // Assistant prose rendered under a response bar is inline (no per-block
    // header/fold — the response bar owns the speaker label), so its
    // `FoldKey::Assistant` would be a dead toggle. Such rows are `AgentItem`s at
    // indent > 0; skip their keys so the fold set matches the on-screen headers.
    let inline_assistant: std::collections::HashSet<usize> = rows
        .iter()
        .filter_map(|row| match row.kind {
            RowKind::AgentItem(ix) if row.indent > 0 => Some(ix),
            _ => None,
        })
        .collect();
    for row in &rows {
        match &row.kind {
            RowKind::ResponseHeader { anchor, .. } => keys.push(FoldKey::Response(*anchor)),
            RowKind::ToolGroupHeader { gid, .. } => keys.push(FoldKey::ToolGroup(gid.clone())),
            // The conclusion's own `FoldKey::Assistant` is added by the per-block
            // loop below (it is not in `inline_assistant`), so nothing to do here.
            RowKind::User(_)
            | RowKind::AgentItem(_)
            | RowKind::ConclusionItem(_)
            | RowKind::WorkingIndicator => {}
        }
    }
    // Per-block fold levels (assistant / thinking by index, tool + its diffs by
    // id).
    for (ix, item) in items.iter().enumerate() {
        match item {
            daruda_acp::ChatItem::AssistantText { .. } if inline_assistant.contains(&ix) => {}
            daruda_acp::ChatItem::AssistantText { .. } => keys.push(FoldKey::Assistant(ix)),
            daruda_acp::ChatItem::Thinking { .. } => keys.push(FoldKey::Thinking(ix)),
            daruda_acp::ChatItem::ToolCall(tc) => {
                keys.push(FoldKey::Tool(tc.id.clone()));
                for di in 0..tc.diffs.len() {
                    keys.push(FoldKey::Diff(diff_editor_key(&tc.id, di)));
                }
                // Mirror the renderer's raw-input gate (generic tool, no diffs,
                // has args) so expand/collapse-all covers the disclosure.
                if renders_raw_input(tc) {
                    keys.push(FoldKey::ToolRawInput(tc.id.clone()));
                }
            }
            daruda_acp::ChatItem::UserText(_)
            | daruda_acp::ChatItem::Permission(_)
            | daruda_acp::ChatItem::Error(_) => {}
        }
    }
    keys
}

/// Whether a tool card renders its raw-input (JSON args) disclosure: a generic
/// tool (not a terminal `Execute`, whose command is already the title) that
/// carries args and has no diffs (an edit shows the diff instead). Single
/// source shared by the renderer and [`collect_foldable_keys`], so the fold
/// coverage matches what is actually on screen.
pub(in crate::workspace) fn renders_raw_input(tc: &daruda_acp::ToolCallItem) -> bool {
    tc.raw_input.is_some()
        && tc.diffs.is_empty()
        && !matches!(tc.kind, daruda_acp::ToolKindView::Execute)
}

/// Cache key for a tool call's `di`-th diff editor: one editor per file. Shared
/// with the renderer so the embed lookup matches the insert key.
pub(in crate::workspace) fn diff_editor_key(tool_call_id: &str, di: usize) -> String {
    format!("{tool_call_id}#{di}")
}

/// Max glyphs the first-prompt fallback title keeps before ellipsizing, and the
/// head kept when it must (leaving room for the `…`). Mirrors Superset's
/// 72/69 first-message-title budget.
const FALLBACK_TITLE_MAX: usize = 72;
const FALLBACK_TITLE_HEAD: usize = 69;

/// The activity-bar title: the agent-supplied session title when set, else a
/// fallback derived from the first user prompt (whitespace-normalized and
/// glyph-truncated), else `None` for a still-empty session (the caller renders a
/// blank bar — no placeholder). Precedence mirrors Superset's session-selector
/// (`session title → first-message fallback → blank`); zed's constant-string
/// fallback is intentionally *not* copied.
pub(in crate::workspace) fn activity_bar_title(
    session_title: Option<&str>,
    items: &[daruda_acp::ChatItem],
) -> Option<String> {
    if let Some(title) = session_title.map(str::trim).filter(|s| !s.is_empty()) {
        return Some(title.to_string());
    }
    items.iter().find_map(|item| match item {
        daruda_acp::ChatItem::UserText(text) => {
            let title = normalize_prompt_title(text);
            (!title.is_empty()).then_some(title)
        }
        _ => None,
    })
}

/// Collapse a user prompt to a single-line title: trim, collapse internal
/// whitespace runs to one space, and glyph-truncate (never byte-slice, so a
/// multibyte prompt can't split a char). Empty when the prompt is whitespace.
fn normalize_prompt_title(text: &str) -> String {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() > FALLBACK_TITLE_MAX {
        let head: String = normalized.chars().take(FALLBACK_TITLE_HEAD).collect();
        format!("{}…", head.trim_end())
    } else {
        normalized
    }
}

/// The markdown body of a chat item that can carry a ` ```mermaid ` fence —
/// assistant / thinking / user text. Tool / permission / error items carry no
/// markdown body and contribute none. Drives the mermaid scan.
pub(in crate::workspace) fn chat_item_markdown(item: &daruda_acp::ChatItem) -> Option<&str> {
    match item {
        daruda_acp::ChatItem::AssistantText { text, .. }
        | daruda_acp::ChatItem::Thinking { text, .. } => Some(text),
        daruda_acp::ChatItem::UserText(text) => Some(text),
        daruda_acp::ChatItem::ToolCall(_)
        | daruda_acp::ChatItem::Permission(_)
        | daruda_acp::ChatItem::Error(_) => None,
    }
}

/// Stable cache key for a mermaid fence's source *at a given appearance*, shared
/// between the rasterizer (insert) and the renderer (lookup) so the embed
/// matches what was cached. `dark` is part of the key because the diagram is
/// themed to the host appearance (`mermaid_with_theme`): without it a cached
/// raster would keep its old colours after a light/dark toggle. `DefaultHasher`
/// is process-stable, which is all the in-memory cache needs.
pub(in crate::workspace) fn mermaid_key(source: &str, dark: bool) -> u64 {
    use std::hash::{Hash as _, Hasher as _};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut hasher);
    dark.hash(&mut hasher);
    hasher.finish()
}

/// Extract the source of every **closed** ` ```mermaid ` fence in `text`, in
/// document order. Only closed fences are returned: a still-streaming (never
/// terminated) trailing `mermaid` fence is skipped so a half-arrived diagram
/// isn't rasterized until it completes. Non-mermaid fences are ignored.
///
/// A mermaid fence opens on a line whose trimmed content is exactly ```` ```mermaid ````
/// (optionally with trailing spaces) and closes on the next line whose trimmed
/// content is ```` ``` ````. Leading indentation on the fence lines is tolerated;
/// the captured source keeps the lines between the fences verbatim.
pub(in crate::workspace) fn mermaid_sources(text: &str) -> Vec<String> {
    let mut sources = Vec::new();
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        if line.trim() != "```mermaid" {
            continue;
        }
        // Inside a mermaid fence — collect until the closing ``` line. If the
        // text ends first the fence is unterminated (still streaming): drop it.
        let mut body: Vec<&str> = Vec::new();
        let mut closed = false;
        for inner in lines.by_ref() {
            if inner.trim() == "```" {
                closed = true;
                break;
            }
            body.push(inner);
        }
        if closed {
            sources.push(body.join("\n"));
        }
    }
    sources
}

/// Added / removed line counts for one tool-call diff, used by the fold summary
/// (`+N −M`) shown when the diff editor is collapsed. Counted from the *same*
/// hunks that build the diff editor (see [`build_diff_view_model`]), so the
/// numbers match what the editor renders exactly. Cached alongside the editor
/// in `AgentChatView.diff_stats`, keyed by [`diff_editor_key`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::workspace) struct DiffStat {
    pub(in crate::workspace) added: usize,
    pub(in crate::workspace) removed: usize,
}

/// Language id for an editor's syntax tree, from the diff's file extension.
/// Empty when unknown (the editor falls back to `"text"`).
pub(in crate::workspace) fn diff_editor_language(diff: &DiffView) -> &'static str {
    match diff.path.extension_str() {
        "rs" => "rust",
        "js" | "mjs" | "cjs" => "javascript",
        "ts" | "mts" | "cts" => "typescript",
        "jsx" => "jsx",
        "tsx" => "tsx",
        "py" => "python",
        "go" => "go",
        "toml" => "toml",
        "json" | "jsonc" => "json",
        "yaml" | "yml" => "yaml",
        "md" | "markdown" => "markdown",
        "html" | "htm" => "html",
        "css" => "css",
        "sh" | "bash" | "zsh" => "bash",
        "c" | "h" => "c",
        "cpp" | "cc" | "cxx" | "hpp" | "hxx" => "cpp",
        _ => "",
    }
}

/// Convert a tool-call [`DiffView`] into the editor inputs the shared
/// diff-through-editor renderer consumes, plus the [`DiffStat`] for the same
/// diff. Pure / GPUI-free: builds the unified diff from `old_text`/`new_text`,
/// syntax-highlights and word-diffs the hunks exactly as the File viewer's
/// `load_diff` does, then folds them into a [`DiffEditorModel`].
///
/// The stat is counted from those *same* hunks (via [`diff_stat_from_hunks`]),
/// so it matches the rendered editor line-for-line.
///
/// Returns `None` when the two sides are identical (no hunks → nothing to
/// render), so the caller leaves the inline fallback in place and records no
/// stat entry (absent ≡ `0/0`).
pub(in crate::workspace) fn build_diff_view_model(
    diff: &DiffView,
    syntax_theme: &str,
    is_light: bool,
    colors: &DiffColors,
) -> Option<(DiffEditorModel, DiffStat)> {
    use crate::workspace::main_area::file_view_pane::highlighter::highlight_hunks;
    use crate::workspace::main_area::file_view_pane::line_diff::unified_diff_text;
    use crate::workspace::main_area::file_view_pane::word_diff::apply_word_diff;
    use crate::workspace::main_area::file_view_pane::{build_diff_rows, parse_diff_hunks};

    let old = diff.old_text.as_deref().unwrap_or("");
    let text = unified_diff_text(old, &diff.new_text);
    let mut hunks = parse_diff_hunks(&text);
    if hunks.is_empty() {
        return None;
    }
    // Count add/remove from the parsed hunks before they are highlighted /
    // word-diffed (those passes only annotate, never reclassify lines), so the
    // stat is from the exact same diff that builds the editor below.
    let stat = diff_stat_from_hunks(&hunks);
    let ext = diff.path.extension_str();
    highlight_hunks(&mut hunks, ext, syntax_theme, is_light);
    apply_word_diff(&mut hunks);
    let rows = build_diff_rows(&hunks, false);
    Some((build_diff_editor_model(&rows, colors), stat))
}

/// Tally a [`DiffStat`] from parsed diff hunks. Pure / GPUI-free wrapper over
/// the File viewer's `count_diff_stats`, which counts `DiffLine::Added` vs
/// `DiffLine::Removed` across the hunks — the same line classification the
/// editor rows are built from.
fn diff_stat_from_hunks(
    hunks: &[crate::workspace::main_area::file_view_pane::DiffHunk],
) -> DiffStat {
    let (added, removed) = crate::workspace::main_area::file_view_pane::count_diff_stats(hunks);
    DiffStat { added, removed }
}

/// Create + configure a read-only diff editor entity inside a single window
/// re-entry against the view's stored `window_handle`. Mirrors the File
/// viewer's editor construction (`multi_line` + `soft_wrap(false)` +
/// `code_editor`) and the diff-config it applies (`set_disabled(true)` for
/// read-only + decorations + injected highlight spans). Returns `None` if the
/// owning window is gone.
///
/// Uses the stored `window_handle` rather than
/// `WindowRegistry::handle_for_workspace(cx.entity_id())` because after the
/// pane became its own entity `cx.entity_id()` is the view, not the Workspace,
/// so the registry would no longer resolve the window.
pub(in crate::workspace) fn create_diff_editor(
    cx: &mut Context<AgentChatView>,
    window_handle: AnyWindowHandle,
    pane_id: PaneId,
    language: &str,
    model: DiffEditorModel,
) -> Option<Entity<gpui_component::input::InputState>> {
    let language = language.to_owned();
    match cx.update_window(window_handle, move |_, window, cx_w| {
        cx_w.new(|cx_state| {
            let mut state = gpui_component::input::InputState::new(window, cx_state)
                .multi_line(true)
                .soft_wrap(false);
            state = if language.is_empty() {
                state.code_editor("text")
            } else {
                state.code_editor(&language)
            };
            state.set_value(model.text, window, cx_state);
            state.set_disabled(true, cx_state);
            state.set_line_decorations(model.decorations, cx_state);
            state.set_highlight_override(Some(model.highlights), cx_state);
            state
        })
    }) {
        Ok(editor) => Some(editor),
        Err(e) => {
            // Window gone mid-stream — drop this editor; the inline fallback
            // renders. Logged so it isn't a silent no-op.
            daruda_store::observability::log_writer::LogWriter::log(
                ErrorReport::new("Failed to build agent-chat diff editor")
                    .severity(ErrorSeverity::Warning)
                    .at(file!(), line!())
                    .with_context("error", format!("{e}"))
                    .dedup(format!("agent_chat.diff_editor.window_gone.{pane_id}"))
                    .build(),
            );
            None
        }
    }
}

/// Whether a chat block is currently streaming / in progress — the `active`
/// input the fold derivation reads. A streaming text or thinking block, or a
/// tool call still `InProgress`, is active; everything else (settled text,
/// finished/failed tool calls, user / permission / error items) is not. Shared
/// by [`AgentChatView::toggle_fold`] and the renderer so both derive the same
/// effective fold state.
pub(in crate::workspace) fn is_active(item: &daruda_acp::ChatItem) -> bool {
    use daruda_acp::ChatItem;
    match item {
        ChatItem::AssistantText { streaming, .. } | ChatItem::Thinking { streaming, .. } => {
            *streaming
        }
        ChatItem::ToolCall(tc) => tc.status.is_live(),
        ChatItem::UserText(_) | ChatItem::Permission(_) | ChatItem::Error(_) => false,
    }
}

/// The trailing not-yet-resolved permission card in `items`, if any. The agent
/// keeps a single permission request outstanding at a time and it is always the
/// most recent, so reverse-scan for the first unresolved card.
pub(in crate::workspace) fn trailing_unresolved_permission(
    view: &mut AgentChatView,
) -> Option<&mut daruda_acp::PermissionItem> {
    view.items.iter_mut().rev().find_map(|item| match item {
        daruda_acp::ChatItem::Permission(card) if card.resolved.is_none() => Some(card),
        _ => None,
    })
}

/// Cancel-drain the view's pending permission request, if any: respond to the
/// agent with a `Cancelled` outcome and mark the trailing unresolved card
/// cancelled so its buttons disable. No-op when nothing is pending; idempotent.
/// ACP requires the client to resolve a pending permission with a cancelled
/// outcome on `session/cancel`; this also runs when a turn ends or errors before
/// the user decided, so no card is left stuck with live buttons.
pub(in crate::workspace) fn cancel_pending_permission(view: &mut AgentChatView) {
    let Some(id) = view.pending_permission.take() else {
        return;
    };
    if let Some(handle) = &view.handle {
        handle.respond_permission(id, daruda_acp::PermissionDecision::Cancelled);
    }
    if let Some(card) = trailing_unresolved_permission(view) {
        card.resolved = Some(daruda_acp::PermissionResolution::Cancelled);
    }
}

/// Apply one `SessionInfoUpdate` field change to a cached `Option<String>`
/// slot. `Unchanged` leaves the slot as-is (the update omitted the field);
/// `Cleared` resets it to `None`; `Set` overwrites it. Shared by the title and
/// last-activity fields so both honour the protocol's per-field tri-state.
pub(in crate::workspace) fn apply_info_field(
    slot: &mut Option<String>,
    change: daruda_acp::InfoFieldChange,
) {
    match change {
        daruda_acp::InfoFieldChange::Unchanged => {}
        daruda_acp::InfoFieldChange::Cleared => *slot = None,
        daruda_acp::InfoFieldChange::Set(value) => *slot = Some(value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use daruda_acp::{ChatItem, ModeStateView, SessionModeView, ToolCallItem};

    fn asst(text: &str) -> ChatItem {
        ChatItem::AssistantText {
            text: text.to_owned(),
            streaming: false,
            message_id: None,
        }
    }

    #[test]
    fn activity_bar_title_prefers_the_session_title() {
        let items = [ChatItem::UserText("run the tests".to_owned())];
        assert_eq!(
            activity_bar_title(Some("Refactor fold state"), &items).as_deref(),
            Some("Refactor fold state")
        );
    }

    #[test]
    fn activity_bar_title_falls_back_to_first_user_prompt() {
        // No session title yet (pre first turn-end): the first prompt stands in.
        let items = [
            ChatItem::UserText("  fix the   parser  ".to_owned()),
            asst("sure"),
            ChatItem::UserText("second".to_owned()),
        ];
        assert_eq!(
            activity_bar_title(None, &items).as_deref(),
            Some("fix the parser")
        );
    }

    #[test]
    fn activity_bar_title_ignores_blank_session_title_and_falls_back() {
        let items = [ChatItem::UserText("hello".to_owned())];
        assert_eq!(
            activity_bar_title(Some("   "), &items).as_deref(),
            Some("hello")
        );
    }

    #[test]
    fn activity_bar_title_is_none_for_an_empty_session() {
        // Neither a session title nor a user prompt → blank bar (no placeholder).
        assert_eq!(activity_bar_title(None, &[]), None);
        // Non-user leading items don't seed a title.
        assert_eq!(activity_bar_title(None, &[asst("greeting")]), None);
        // A whitespace-only prompt yields nothing.
        assert_eq!(
            activity_bar_title(None, &[ChatItem::UserText("   ".to_owned())]),
            None
        );
    }

    #[test]
    fn normalize_prompt_title_truncates_long_prompts_on_a_char_boundary() {
        let long = "가".repeat(100);
        let title = normalize_prompt_title(&long);
        // 69 kept glyphs + the ellipsis (never a split multibyte char).
        assert_eq!(title.chars().count(), FALLBACK_TITLE_HEAD + 1);
        assert!(title.ends_with('…'));
    }

    #[test]
    fn normalize_prompt_title_keeps_short_prompts_verbatim() {
        assert_eq!(normalize_prompt_title("short one"), "short one");
    }

    fn modes(ids: &[&str], current: &str) -> ModeStateView {
        ModeStateView {
            available: ids
                .iter()
                .map(|id| SessionModeView {
                    id: (*id).to_string(),
                    name: (*id).to_string(),
                    description: None,
                })
                .collect(),
            current: current.to_string(),
        }
    }

    #[test]
    fn next_mode_id_wraps_through_advertised_order() {
        let m = modes(&["default", "acceptEdits", "bypassPermissions"], "default");
        assert_eq!(next_mode_id(&m).as_deref(), Some("acceptEdits"));
        let m = modes(
            &["default", "acceptEdits", "bypassPermissions"],
            "acceptEdits",
        );
        assert_eq!(next_mode_id(&m).as_deref(), Some("bypassPermissions"));
        // Wrap: last → first.
        let m = modes(
            &["default", "acceptEdits", "bypassPermissions"],
            "bypassPermissions",
        );
        assert_eq!(next_mode_id(&m).as_deref(), Some("default"));
    }

    #[test]
    fn next_mode_id_none_when_not_cyclable() {
        // Zero or one advertised mode → nothing to cycle.
        assert_eq!(next_mode_id(&modes(&[], "")), None);
        assert_eq!(next_mode_id(&modes(&["default"], "default")), None);
    }

    #[test]
    fn next_mode_id_starts_from_first_when_current_unknown() {
        let m = modes(&["default", "acceptEdits"], "stale-id");
        assert_eq!(next_mode_id(&m).as_deref(), Some("acceptEdits"));
    }

    /// A syntax theme id every test reuses for the highlight passes.
    const TEST_SYNTAX_THEME: &str = "base16-ocean.dark";

    /// A flat `DiffColors` fixture so the pure model build is testable without a
    /// live theme.
    fn diff_colors() -> DiffColors {
        let c = |l: f32| gpui::Hsla {
            h: 0.,
            s: 0.,
            l,
            a: 1.,
        };
        DiffColors {
            add_bg: c(0.1),
            del_bg: c(0.11),
            hunk_bg: c(0.12),
            add_text: c(0.2),
            del_text: c(0.21),
            ctx_text: c(0.22),
            hunk_text: c(0.23),
            hunk_ctx_text: c(0.24),
            word_add_bg: c(0.3),
            word_del_bg: c(0.31),
        }
    }

    fn diff(old: Option<&str>, new: &str, path: &str) -> DiffView {
        DiffView {
            path: std::path::PathBuf::from(path),
            old_text: old.map(str::to_owned),
            new_text: new.to_owned(),
        }
    }

    /// `build_diff_view_model` turns a single-line modification into a
    /// `DiffEditorModel` whose synthetic buffer carries the hunk header plus
    /// both sides (no `+`/`-` markers — the kind is in the decorations) and
    /// whose per-row decorations include add/del backgrounds.
    #[test]
    fn diff_view_model_builds_rows_and_decorations() {
        let d = diff(
            Some("fn a() {}\nlet x = 1;\nfn b() {}\n"),
            "fn a() {}\nlet y = 2;\nfn b() {}\n",
            "src/lib.rs",
        );
        let (m, _) = build_diff_view_model(&d, TEST_SYNTAX_THEME, false, &diff_colors())
            .expect("a modified file produces hunks");
        // Hunk header row + content rows, no marker prefix on content.
        assert!(m.text.starts_with("@@"), "buffer leads with a hunk header");
        assert!(m.text.contains("let x = 1;"), "removed line present");
        assert!(m.text.contains("let y = 2;"), "added line present");
        // Some rows carry an add/del background (the changed pair).
        let with_bg = m
            .decorations
            .iter()
            .filter(|d| d.background.is_some())
            .count();
        assert!(with_bg >= 2, "at least the changed pair is tinted");
    }

    /// A newly created file (`old_text == None`) diffs against an empty old
    /// side — every line is an addition, so the model is built (non-empty).
    #[test]
    fn diff_view_model_handles_created_file() {
        let d = diff(None, "line one\nline two\n", "new.txt");
        let (m, _) = build_diff_view_model(&d, TEST_SYNTAX_THEME, false, &diff_colors())
            .expect("a created file produces an all-added hunk");
        assert!(m.text.contains("line one"));
        assert!(m.text.contains("line two"));
    }

    /// Identical sides yield no hunks, so the adapter returns `None` and the
    /// caller keeps the inline fallback.
    #[test]
    fn diff_view_model_none_when_unchanged() {
        let d = diff(Some("same\n"), "same\n", "same.txt");
        assert!(build_diff_view_model(&d, TEST_SYNTAX_THEME, false, &diff_colors()).is_none());
    }

    /// A simple one-line modification must report the *changed* line on each
    /// side — `added = 1, removed = 1` — not the file's total line counts.
    #[test]
    fn diff_stat_counts_changed_lines_not_totals() {
        let d = diff(
            Some("fn a() {}\nlet x = 1;\nfn b() {}\n"),
            "fn a() {}\nlet y = 2;\nfn b() {}\n",
            "src/lib.rs",
        );
        let (_, stat) = build_diff_view_model(&d, TEST_SYNTAX_THEME, false, &diff_colors())
            .expect("a modified file produces hunks");
        assert_eq!(
            stat,
            DiffStat {
                added: 1,
                removed: 1
            }
        );
    }

    /// A newly created file (`old_text == None`) diffs against an empty old
    /// side, so every line is an addition: `added = N, removed = 0`.
    #[test]
    fn diff_stat_new_file_is_all_added() {
        let d = diff(None, "line one\nline two\nline three\n", "new.txt");
        let (_, stat) = build_diff_view_model(&d, TEST_SYNTAX_THEME, false, &diff_colors())
            .expect("a created file produces an all-added hunk");
        assert_eq!(
            stat,
            DiffStat {
                added: 3,
                removed: 0
            }
        );
    }

    /// A pure deletion — the new side drops every line of the old — reports
    /// `added = 0, removed = N`, the mirror of the all-added created-file case.
    #[test]
    fn diff_stat_deleted_lines_are_all_removed() {
        let d = diff(Some("first\nsecond\n"), "", "old.rs");
        let (_, stat) = build_diff_view_model(&d, TEST_SYNTAX_THEME, false, &diff_colors())
            .expect("a fully-deleted file produces an all-removed hunk");
        assert_eq!(
            stat,
            DiffStat {
                added: 0,
                removed: 2
            }
        );
    }

    /// Identical sides produce no hunks → no editor and no stat. This pins the
    /// pure tally directly on empty hunks for clarity.
    #[test]
    fn diff_stat_unchanged_is_zero() {
        assert_eq!(diff_stat_from_hunks(&[]), DiffStat::default());
    }

    /// The cache key is per-(tool-call, diff index) so two files in one tool
    /// call get distinct editors.
    #[test]
    fn diff_editor_keys_are_per_file() {
        assert_eq!(diff_editor_key("call-1", 0), "call-1#0");
        assert_ne!(diff_editor_key("call-1", 0), diff_editor_key("call-1", 1));
        assert_ne!(diff_editor_key("call-1", 0), diff_editor_key("call-2", 0));
    }

    /// A tool-call item with a given status and diff list, for `is_active` and
    /// key-collection coverage.
    fn tool_call(id: &str, status: daruda_acp::ToolStatusView, diffs: usize) -> ToolCallItem {
        ToolCallItem {
            id: id.to_owned(),
            title: "t".to_owned(),
            kind: daruda_acp::ToolKindView::Edit,
            status,
            diffs: (0..diffs)
                .map(|i| DiffView {
                    path: std::path::PathBuf::from(format!("f{i}.rs")),
                    old_text: None,
                    new_text: "x".to_owned(),
                })
                .collect(),
            output: Vec::new(),
            raw_input: None,
        }
    }

    /// `is_active` is true while a block is streaming, or a tool call is live
    /// (`Pending` or `InProgress` — see [`ToolStatusView::is_live`]).
    #[test]
    fn is_active_matches_streaming_and_in_progress() {
        use daruda_acp::ToolStatusView::*;
        assert!(is_active(&ChatItem::AssistantText {
            text: "a".to_owned(),
            streaming: true,
            message_id: None,
        }));
        assert!(!is_active(&ChatItem::AssistantText {
            text: "a".to_owned(),
            streaming: false,
            message_id: None,
        }));
        assert!(is_active(&ChatItem::Thinking {
            text: "t".to_owned(),
            streaming: true,
            message_id: None,
        }));
        assert!(!is_active(&ChatItem::Thinking {
            text: "t".to_owned(),
            streaming: false,
            message_id: None,
        }));
        assert!(is_active(&ChatItem::ToolCall(tool_call(
            "c1", InProgress, 0
        ))));
        // A live `Pending` tool means an in-flight call in the active turn
        // (leftover `Pending` is settled to `Cancelled` at turn end), so it
        // reads as active — same as `InProgress`.
        assert!(is_active(&ChatItem::ToolCall(tool_call("c1", Pending, 0))));
        assert!(!is_active(&ChatItem::ToolCall(tool_call(
            "c1", Completed, 0
        ))));
        assert!(!is_active(&ChatItem::ToolCall(tool_call("c1", Failed, 0))));
        // Non-foldable / inactive items.
        assert!(!is_active(&ChatItem::UserText("u".to_owned())));
        assert!(!is_active(&ChatItem::Error("e".to_owned())));
    }

    /// A single closed mermaid fence yields its verbatim body.
    #[test]
    fn mermaid_sources_extracts_a_closed_fence() {
        let text = "intro\n```mermaid\ngraph TD\nA-->B\n```\noutro";
        assert_eq!(mermaid_sources(text), vec!["graph TD\nA-->B".to_string()]);
    }

    /// Multiple closed fences are returned in document order.
    #[test]
    fn mermaid_sources_extracts_multiple_fences() {
        let text = "```mermaid\nA\n```\nmid\n```mermaid\nB\n```";
        assert_eq!(
            mermaid_sources(text),
            vec!["A".to_string(), "B".to_string()]
        );
    }

    /// An unterminated trailing fence (still streaming) is skipped — only the
    /// already-closed fence before it is returned.
    #[test]
    fn mermaid_sources_skips_unterminated_trailing_fence() {
        let text = "```mermaid\nA\n```\n```mermaid\nstill streaming";
        assert_eq!(mermaid_sources(text), vec!["A".to_string()]);
        // A lone unterminated fence yields nothing.
        assert!(mermaid_sources("```mermaid\ngraph TD").is_empty());
    }

    /// Non-mermaid fences (other languages, or none) are ignored.
    #[test]
    fn mermaid_sources_ignores_non_mermaid_fences() {
        let text = "```rust\nfn main() {}\n```\n```\nplain\n```";
        assert!(mermaid_sources(text).is_empty());
    }

    /// The cache key is stable per (source, appearance) and distinct across
    /// sources *and* across the dark/light appearance — so a light/dark toggle
    /// re-rasterizes rather than reusing a stale-coloured diagram.
    #[test]
    fn mermaid_key_is_stable_and_distinct() {
        assert_eq!(
            mermaid_key("graph TD\nA-->B", true),
            mermaid_key("graph TD\nA-->B", true)
        );
        assert_ne!(
            mermaid_key("graph TD\nA-->B", true),
            mermaid_key("graph LR\nA-->B", true)
        );
        // Same source, different appearance → different key.
        assert_ne!(
            mermaid_key("graph TD\nA-->B", true),
            mermaid_key("graph TD\nA-->B", false)
        );
    }

    /// The visible foldable-key set the expand-all / collapse-all op builds.
    #[test]
    fn visible_fold_keys_cover_text_tools_and_diffs() {
        use daruda_acp::ToolStatusView::Completed;
        let items = [
            ChatItem::UserText("u".to_owned()),
            ChatItem::AssistantText {
                text: "a".to_owned(),
                streaming: false,
                message_id: None,
            },
            ChatItem::Thinking {
                text: "t".to_owned(),
                streaming: false,
                message_id: None,
            },
            ChatItem::ToolCall(tool_call("c1", Completed, 2)),
            ChatItem::Error("e".to_owned()),
        ];
        let keys = collect_foldable_keys(&items);
        // Structural header keys (the response — non-trivial run) first, then
        // the per-block keys. The single tool call is not a group (run < 2). The
        // assistant text (item 1) is the run's conclusion, which carries its own
        // fold toggle, so it contributes an `Assistant` key; thinking keeps its
        // own fold.
        assert_eq!(
            keys,
            vec![
                FoldKey::Response(0),
                FoldKey::Assistant(1),
                FoldKey::Thinking(2),
                FoldKey::Tool("c1".to_owned()),
                FoldKey::Diff("c1#0".to_owned()),
                FoldKey::Diff("c1#1".to_owned()),
            ]
        );
    }

    /// A trivial single-block reply has no response bar, so its assistant prose
    /// keeps the labeled, foldable block — its `Assistant` key is still
    /// collected. Guards the inline-vs-block split in `collect_foldable_keys`.
    #[test]
    fn trivial_reply_keeps_assistant_fold_key() {
        let items = [
            ChatItem::UserText("u".to_owned()),
            ChatItem::AssistantText {
                text: "a".to_owned(),
                streaming: false,
                message_id: None,
            },
        ];
        assert_eq!(collect_foldable_keys(&items), vec![FoldKey::Assistant(1)]);
    }

    /// A consecutive tool-call run (≥ 2) contributes a `ToolGroup` key on top
    /// of the per-tool keys, so expand/collapse-all reaches the group level.
    #[test]
    fn fold_keys_include_response_and_tool_group() {
        use daruda_acp::ToolStatusView::Completed;
        let items = [
            ChatItem::UserText("u".to_owned()),
            ChatItem::ToolCall(tool_call("c1", Completed, 0)),
            ChatItem::ToolCall(tool_call("c2", Completed, 0)),
        ];
        let keys = collect_foldable_keys(&items);
        assert_eq!(
            keys,
            vec![
                FoldKey::Response(0),
                FoldKey::ToolGroup("c1".to_owned()),
                FoldKey::Tool("c1".to_owned()),
                FoldKey::Tool("c2".to_owned()),
            ]
        );
    }

    /// `renders_raw_input` is the single gate shared by the renderer and
    /// `collect_foldable_keys`; pin both the predicate and the resulting fold
    /// coverage so a future edit can't break renderer↔fold sync silently.
    #[test]
    fn raw_input_disclosure_gate_and_fold_coverage() {
        use daruda_acp::{ChatItem, ToolKindView, ToolStatusView};
        let generic = ToolCallItem {
            id: "c1".to_owned(),
            title: "Grep".to_owned(),
            kind: ToolKindView::Search,
            status: ToolStatusView::Completed,
            diffs: Vec::new(),
            output: Vec::new(),
            raw_input: Some(serde_json::json!({ "pattern": "foo" })),
        };
        // Generic tool with args and no diffs → disclosure shown, and the fold
        // key is collected (expand/collapse-all reaches it).
        assert!(renders_raw_input(&generic));
        let keys = collect_foldable_keys(&[ChatItem::ToolCall(generic.clone())]);
        assert!(keys.contains(&FoldKey::ToolRawInput("c1".to_owned())));

        // Execute (terminal): the command is already the title → no disclosure,
        // and no fold key for it.
        let exec = ToolCallItem {
            kind: ToolKindView::Execute,
            ..generic.clone()
        };
        assert!(!renders_raw_input(&exec));
        let exec_keys = collect_foldable_keys(&[ChatItem::ToolCall(exec)]);
        assert!(
            !exec_keys
                .iter()
                .any(|k| matches!(k, FoldKey::ToolRawInput(_)))
        );

        // No args, or a diff present (an edit shows the diff) → nothing to show.
        assert!(!renders_raw_input(&ToolCallItem {
            raw_input: None,
            ..generic.clone()
        }));
        assert!(!renders_raw_input(&ToolCallItem {
            diffs: vec![DiffView {
                path: std::path::PathBuf::from("f.rs"),
                old_text: None,
                new_text: "x".to_owned(),
            }],
            ..generic
        }));
    }
}
