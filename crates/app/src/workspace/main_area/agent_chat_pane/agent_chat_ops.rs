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

use daruda_acp::{NodeProgress, connect_session_with_node};
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use futures::StreamExt as _;
use futures::channel::mpsc::unbounded;
use gpui::{AppContext as _, Context, Entity, Window};

use super::agent_chat_helpers::next_mode_id;
use super::view::{AgentChatView, AgentSessionStatus, RuntimePrepPhase};
use crate::surface::strings as s;
use crate::workspace::Workspace;
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
