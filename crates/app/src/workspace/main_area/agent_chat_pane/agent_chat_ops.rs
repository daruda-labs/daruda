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
//!         │  connect_agent_session on bg executor → (handle, rx)
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

use daruda_acp::{NodeProgress, connect_agent_session};
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use futures::StreamExt as _;
use futures::channel::mpsc::unbounded;
use gpui::{AppContext as _, Context, Entity, Window};

use super::agent_chat_helpers::next_mode_id;
use super::slash_dispatch::{LocalSlashCommand, SlashDispatch, classify_slash};
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

/// The catalog's default agent id — the first entry, or the built-in Claude id
/// if the catalog is somehow empty (the config layer guarantees non-empty, so
/// the fallback is purely defensive).
fn catalog_default_id(agents: &[daruda_config::AgentDefinition]) -> String {
    agents
        .first()
        .map(|a| a.id.clone())
        .unwrap_or_else(|| daruda_config::AgentDefinition::claude_default().id)
}

/// Pure core of [`Workspace::resolve_restored_agent`] — decide the effective
/// agent for a restored pane and whether its persisted session id survives.
/// Factored out of the workspace so it is unit-testable without gpui.
fn resolve_restored_agent(
    agents: &[daruda_config::AgentDefinition],
    persisted_agent_id: Option<String>,
) -> (String, bool) {
    let owner =
        persisted_agent_id.unwrap_or_else(|| daruda_config::AgentDefinition::claude_default().id);
    if agents.iter().any(|a| a.id == owner) {
        (owner, true)
    } else {
        (catalog_default_id(agents), false)
    }
}

/// Pure core of the agent-id choice for a freshly opened pane: keep `last` when
/// it is still in the catalog, otherwise fall back to the catalog default
/// (`agents[0]`, or the built-in Claude id if the catalog is somehow empty).
/// Factored out of [`Workspace::open_agent_chat_pane`] so it is unit-testable
/// without gpui.
pub(in crate::workspace) fn resolve_open_agent_id(
    agents: &[daruda_config::AgentDefinition],
    last: Option<&str>,
) -> String {
    last.filter(|id| agents.iter().any(|a| a.id == *id))
        .map(str::to_owned)
        .unwrap_or_else(|| catalog_default_id(agents))
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
        session_id: Option<String>,
        agent_id: String,
        title: Option<String>,
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
        // Seed the tab title from the persisted session title (when present) so
        // a restored dormant pane shows its label before the session loads;
        // fall back to the static default for a freshly opened pane.
        let cached_title = match &title {
            Some(t) if !t.is_empty() => t.clone().into(),
            _ => s::agent_chat_tab_title().into(),
        };
        // The view owns its own `cwd` (for connect / persistence); the wrapper
        // caches a copy so `Pane::cwd()` stays cx-free.
        let view = cx.new({
            let cwd = cwd.clone();
            move |cx| {
                AgentChatView::new(
                    pane_id,
                    window_handle,
                    cwd,
                    status,
                    session_id,
                    agent_id,
                    title,
                    cx,
                )
            }
        });
        Pane {
            id: pane_id,
            content: PaneContent::AgentChat(AgentChatContent {
                view,
                cached_title,
                cwd,
            }),
        }
    }

    /// The launch command for `agent_id`, looked up in the catalog. `None` when
    /// the id is not in the catalog (e.g. a persisted id whose agent was removed).
    pub(in crate::workspace) fn agent_command_for(&self, agent_id: &str) -> Option<String> {
        self.agents
            .iter()
            .find(|a| a.id == agent_id)
            .map(|a| a.command.clone())
    }

    /// Resolve the agent a restored pane should launch under, and whether its
    /// persisted session id is still resumable. A pre-feature save (`agent_id`
    /// None) was created by the built-in Claude agent. If that owning agent is
    /// still in the catalog we relaunch it and keep the session id (resume works);
    /// if it was removed we fall back to the default agent and drop the session id
    /// (its session belongs to a now-absent agent — resuming it would be invalid).
    pub(in crate::workspace) fn resolve_restored_agent(
        &self,
        persisted_agent_id: Option<String>,
    ) -> (String, bool /* keep session id */) {
        resolve_restored_agent(&self.agents, persisted_agent_id)
    }

    /// Open a fresh Agent chat pane in a new tab under the session's last-chosen
    /// agent (falling back to the catalog default). Thin wrapper over
    /// [`Self::open_agent_chat_pane_with_agent`] so there is one construction
    /// path.
    pub(in crate::workspace) fn open_agent_chat_pane(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Default to the last agent the user opened (session-local), falling
        // back to the catalog default. A stale last id (agent removed from
        // config) also falls back.
        let agent_id = resolve_open_agent_id(&self.agents, self.last_agent_id.as_deref());
        self.open_agent_chat_pane_with_agent(agent_id, window, cx);
    }

    /// Open a fresh Agent chat pane in a new tab under `agent_id`, anchored at
    /// the active lane's working directory. Mirrors `open_task_edit_pane`'s
    /// tab-append + focus flow, and records `agent_id` as the session's last
    /// choice so the next fresh pane defaults to it.
    pub(in crate::workspace) fn open_agent_chat_pane_with_agent(
        &mut self,
        agent_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // An inaccessible active lane renders the empty-state; opening a pane
        // there would escape that state (mirrors `add_tab`). Guard first, before
        // recording the choice, so a rejected open does not mutate last_agent_id.
        if self.active_lane_is_inaccessible() {
            return;
        }
        self.last_agent_id = Some(agent_id.clone());
        let cwd = self.active_lane().map(|w| w.path.clone());
        let pane = self.create_agent_chat_pane(cwd, None, agent_id, None, window, cx);
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
        self.set_focused_pane(pane_id, window, cx);
        self.bump_activity(pane_id);
        self.focus_pane(pane_id, window, cx);
        self.resize_all_tabs(window, cx);
        cx.notify();
    }

    /// Handle the agent chip selecting a *different* agent: confirm, then open a
    /// new pane under that agent. The current pane + conversation are preserved
    /// — a backend swap means a fresh conversation (Zed's agent = thread model),
    /// so we never reuse the existing session. No-op when `target_agent_id` is
    /// not in the catalog, or when it already matches the focused pane's current
    /// agent (re-selecting the current agent must not open a fresh conversation
    /// — the chip guards this too, but enforce it at the op boundary regardless
    /// of the caller).
    pub(in crate::workspace) fn request_switch_agent(
        &mut self,
        target_agent_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Re-selecting the focused pane's current agent is a no-op.
        let focused = self.active_runtime().focused_pane_id;
        if self
            .agent_chat_view(focused)
            .is_some_and(|v| v.read(cx).agent_id == target_agent_id)
        {
            return;
        }
        // Look up the display name for the dialog body; ignore an unknown id.
        let Some(name) = self
            .agents
            .iter()
            .find(|a| a.id == target_agent_id)
            .map(|a| a.name.clone())
        else {
            return;
        };
        let weak = cx.weak_entity();
        crate::workspace::dialog_helpers::open_confirm_dialog(
            s::agent_chat_switch_confirm_title(),
            s::agent_chat_switch_confirm_body(&name),
            s::agent_chat_switch_confirm_ok(),
            // Neutral confirm — this opens a new chat, it is not destructive.
            crate::ui::ButtonVariant::Primary,
            move |_ev, window, app| {
                if let Some(ws) = weak.upgrade() {
                    let target = target_agent_id.clone();
                    ws.update(app, |ws, cx| {
                        ws.open_agent_chat_pane_with_agent(target, window, cx)
                    });
                }
            },
            window,
            cx,
        );
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
        let (cwd, resume) = {
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
            // A persisted session id (restored pane) resumes the prior
            // conversation via `session/load`; `None` starts a fresh session.
            (cwd, v.session_id.clone())
        };
        // Flip to `Connecting` before spawning so a second focus during the
        // handshake doesn't start a duplicate session. Mark `restoring` when a
        // resume is in flight so `apply_event` coalesces the load's replay.
        if let Some(view) = self.agent_chat_view(pane_id).cloned() {
            let resuming = resume.is_some();
            view.update(cx, |v, cx| {
                v.restoring = resuming;
                v.set_connecting(cx);
            });
            // Idle → Connecting is a dock-badge status change; the cached
            // docks need an explicit dirty (see `notify_status_docks`).
            self.notify_status_docks(cx);
        }
        self.connect_agent_chat(pane_id, cwd, resume, cx);
    }

    /// Resolve the launch command for `pane_id`'s agent, reconciling the view
    /// when its `agent_id` is stale. Happy path (allocation-free beyond the
    /// command clone): the view's `agent_id` is still in the catalog, so return
    /// its command unchanged. Miss: the persisted `agent_id` now points at an
    /// agent a live config reload removed/renamed, so it lies about which agent
    /// is running — rewrite the view to the session-sticky default, persist, and
    /// launch that agent instead. Returns `None` only when the pane is gone.
    fn resolve_pane_launch(&mut self, pane_id: PaneId, cx: &mut Context<Self>) -> Option<String> {
        let agent_id = self.agent_chat_view(pane_id)?.read(cx).agent_id.clone();
        // Happy path: the view's agent is still in the catalog.
        if let Some(command) = self.agent_command_for(&agent_id) {
            return Some(command);
        }
        // Stale id — reconcile so the chip / persisted state stop lying, then
        // launch the effective agent. `resolve_open_agent_id` yields a catalog
        // entry (or the built-in Claude id when the catalog is somehow empty).
        let effective_id = resolve_open_agent_id(&self.agents, self.last_agent_id.as_deref());
        if let Some(view) = self.agent_chat_view(pane_id).cloned() {
            let id = effective_id.clone();
            view.update(cx, |v, _| v.agent_id = id);
            self.mark_dirty_and_save(cx);
        }
        Some(
            self.agent_command_for(&effective_id)
                .unwrap_or_else(|| daruda_config::AgentDefinition::claude_default().command),
        )
    }

    /// Open the live ACP session for an already-pushed Agent chat pane and store
    /// the event-pump task on its view. Runs the (synchronous-to-parse, then
    /// async) connect on the background executor, then re-enters the workspace
    /// to store the handle and fold events through the view.
    ///
    /// The spawned task is stored in the view's `_event_pump`, so closing the
    /// pane drops it (ending the loop) in addition to dropping the session
    /// handle (which closes the connection).
    /// `resume` carries the persisted ACP session id when the pane is restoring
    /// a prior conversation: `Some` branches `session/load` (the adapter replays
    /// history before the `Connected` reply), `None` starts a fresh
    /// `session/new`. A failed *resume* retries once as a fresh session so the
    /// pane stays usable; the stale id is left persisted (a successful new
    /// session overwrites it via the `Connected` persist trigger).
    pub(in crate::workspace) fn connect_agent_chat(
        &mut self,
        pane_id: PaneId,
        cwd: std::path::PathBuf,
        resume: Option<String>,
        cx: &mut Context<Self>,
    ) {
        // A fresh session starts in the app's default permission mode; a resume
        // preserves whatever mode the loaded session already had (pass `None` so
        // `run_connection` skips the initial `set_mode`), so reattaching never
        // silently overrides a mode the user set in the prior session.
        let initial_mode = if resume.is_some() {
            None
        } else {
            Some(self.agent.default_permission_mode.mode_id().to_string())
        };
        let node_root = daruda_store::persistence::node_install_dir();

        // Resolve the pane's agent_id → launch command, reconciling the view
        // when its agent_id no longer names a catalog entry (a live config
        // reload removed/renamed that agent). Returns `None` only when the pane
        // is already gone — fall back to the catalog default command so the
        // (soon-to-be-dropped) task still has a valid command to parse.
        let command = self.resolve_pane_launch(pane_id, cx).unwrap_or_else(|| {
            self.agent_command_for(&catalog_default_id(&self.agents))
                .unwrap_or_else(|| daruda_config::AgentDefinition::claude_default().command)
        });

        // Runtime provisioning (see `connect_agent_session`) can download
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
                            v.set_preparing(phase, cx);
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

        // Kept for a fresh-session fallback if a resume fails (the resume attempt
        // moves its own clones into the background closure below).
        let retry_cwd = cwd.clone();
        let was_resume = resume.is_some();
        let pump = cx.spawn(async move |this, cx| {
            // `connect_agent_session` is synchronous (it provisions node,
            // parses the command, and spawns the connection task); run it on the
            // background executor so the download / smol `spawn` bind to a worker
            // thread rather than the main loop. The progress sender is moved in
            // and dropped when this closure returns, ending the drain above.
            let connected = cx
                .background_executor()
                .spawn(async move {
                    let mut progress = move |milestone| drop(progress_tx.unbounded_send(milestone));
                    // `Some` resumes the persisted session (`session/load`);
                    // `None` starts a fresh session (`session/new`).
                    connect_agent_session(
                        command,
                        node_root,
                        cwd,
                        initial_mode,
                        resume.map(daruda_acp::SessionId::new),
                        &mut progress,
                    )
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
                                    v.set_connecting(cx);
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
                    let mut connected_seen = false;
                    while let Some(event) = events.next().await {
                        // A rejected `session/load` (stale / expired / unknown
                        // persisted id) surfaces as `AcpEvent::Error` on the
                        // stream — `run_connection` runs detached, so the sync
                        // `Err` arm below never sees it. Before any `Connected`,
                        // treat a resume's error as a failed load and retry once
                        // as a fresh session. Bounded: the retry runs with
                        // `resume = None`, so its own error can't loop back here.
                        // The stale id is left persisted — a successful fresh
                        // session overwrites it via the `Connected` persist trigger.
                        if was_resume
                            && !connected_seen
                            && let daruda_acp::AcpEvent::Error(detail) = &event
                        {
                            let detail = detail.clone();
                            // SILENT-OK: workspace/window dropped before the resume retry could start
                            let _ = this.update(cx, |ws, cx| {
                                if let Some(view) = ws.agent_chat_view(pane_id).cloned() {
                                    view.update(cx, |v, cx| {
                                        // Release the replay gate and return to a
                                        // plain connecting state before the retry.
                                        v.restoring = false;
                                        v.set_connecting(cx);
                                    });
                                }
                                let report = ErrorReport::new(
                                    "ACP session/load resume failed; retrying fresh",
                                )
                                .severity(ErrorSeverity::Warning)
                                .with_context("detail", detail)
                                .at(file!(), line!())
                                .dedup("agent_chat.resume_fallback")
                                .build();
                                daruda_store::observability::log_writer::LogWriter::log(report);
                                // Fresh retry → `session/new`; spawns a new pump on
                                // the view, superseding this one.
                                ws.connect_agent_chat(pane_id, retry_cwd.clone(), None, cx);
                            });
                            return;
                        }
                        let is_connected = matches!(&event, daruda_acp::AcpEvent::Connected { .. });
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
                            // Capture the persisted session identity before the
                            // event: `Connected` establishes the live session id
                            // and `SessionInfoChanged` sets the title. Both are
                            // persisted (the id lets a later launch resume via
                            // `session/load`; the title names the tab), so a save
                            // is triggered below when either changes.
                            let session_id_before = view.read(cx).session_id.clone();
                            let title_before = view.read(cx).session_title.clone();
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
                            // Persist when the session id is newly established
                            // (or changed) or the title changed. Both change
                            // rarely — once at connect, then on the occasional
                            // `SessionInfoChanged` — so this never thrashes on
                            // token-streaming events.
                            {
                                let v = view.read(cx);
                                if v.session_id != session_id_before
                                    || v.session_title != title_before
                                {
                                    ws.mark_dirty_and_save(cx);
                                }
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
                        if is_connected {
                            connected_seen = true;
                        }
                        // Workspace/window gone (Err) or view gone (Ok(false)) —
                        // stop pumping.
                        if !matches!(cont, Ok(true)) {
                            break;
                        }
                    }
                    // The stream ended. If a resume was still replaying (no
                    // Connected/Error arrived — a misbehaving adapter), release
                    // the gate so the accumulated items render instead of the
                    // pane freezing mid-restore.
                    // SILENT-OK: view/window already gone at end-of-stream — nothing to release
                    let _ = this.update(cx, |ws, cx| {
                        if let Some(view) = ws.agent_chat_view(pane_id).cloned() {
                            view.update(cx, |v, cx| v.abort_restore(cx));
                        }
                    });
                }
                Err(err) if was_resume => {
                    // A failed *resume* (`session/load`) retries once as a fresh
                    // session so the pane stays usable. The persisted session id
                    // is intentionally left untouched: a successful new session
                    // overwrites it via the `Connected` persist trigger above,
                    // and a transient error must never wipe a still-valid id.
                    let message = format!("{err}");
                    // SILENT-OK: workspace/window dropped before the resume retry could start
                    let _ = this.update(cx, |ws, cx| {
                        if let Some(view) = ws.agent_chat_view(pane_id).cloned() {
                            view.update(cx, |v, cx| {
                                // No load will happen now — release the replay
                                // gate and return to a plain connecting state
                                // before the fresh retry.
                                v.restoring = false;
                                v.set_connecting(cx);
                            });
                        }
                        let report = ErrorReport::new("ACP session resume failed; retrying fresh")
                            .severity(ErrorSeverity::Warning)
                            .with_context("detail", message)
                            .at(file!(), line!())
                            .dedup("agent_chat.resume_fallback")
                            .build();
                        daruda_store::observability::log_writer::LogWriter::log(report);
                        // Re-enter with no resume → `session/new`. This spawns a
                        // fresh pump on the view; the current task then returns,
                        // dropping this (now superseded) pump.
                        ws.connect_agent_chat(pane_id, retry_cwd.clone(), None, cx);
                    });
                }
                Err(err) => {
                    let message = format!("{err}");
                    // workspace gone before the connect resolved — nothing left
                    // to surface the failure on.
                    // SILENT-OK: workspace/window dropped before connect resolved
                    let _ = this.update(cx, |ws, cx| {
                        if let Some(view) = ws.agent_chat_view(pane_id).cloned() {
                            view.update(cx, |v, cx| {
                                v.set_error(message.clone(), cx);
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

    /// Full local reset for the `/clear` slash command: wipe the conversation
    /// UI, tear down the live ACP session, clear the persisted session id, and
    /// start a fresh `session/new`. No-op when `pane_id` is gone or has no
    /// lane cwd (a cwd-less pane never had a session to reset).
    pub(in crate::workspace) fn reset_agent_chat_session(
        &mut self,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.agent_chat_view(pane_id).cloned() else {
            return;
        };
        let Some(cwd) = view.read(cx).cwd.clone() else {
            return; // no cwd → never had a session
        };
        view.update(cx, |v, cx| v.reset_for_new_session(cx));
        self.mark_dirty_and_save(cx);
        self.notify_status_docks(cx);
        self.connect_agent_chat(pane_id, cwd, None, cx);
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
        match classify_slash(&text) {
            SlashDispatch::Local(LocalSlashCommand::Clear) => {
                self.reset_agent_chat_session(pane_id, cx);
            }
            SlashDispatch::Forward => {
                if let Some(view) = self.agent_chat_view(pane_id).cloned() {
                    view.update(cx, |v, cx| v.send_prompt_text(text, cx));
                }
            }
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
        if !view.read(cx).turn.is_in_flight() {
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

#[cfg(test)]
mod tests {
    use super::{resolve_open_agent_id, resolve_restored_agent};
    use daruda_config::AgentDefinition;

    fn default_catalog() -> Vec<AgentDefinition> {
        vec![AgentDefinition::claude_default()]
    }

    #[test]
    fn restored_agent_kept_when_owner_present() {
        let claude = AgentDefinition::claude_default().id;
        let (agent, keep) = resolve_restored_agent(&default_catalog(), Some(claude.clone()));
        assert_eq!(agent, claude);
        assert!(
            keep,
            "session id resumable when owning agent is in the catalog"
        );
    }

    #[test]
    fn restored_none_falls_back_to_claude_and_keeps_session() {
        // Backward compat: a pre-feature save (agent_id None) was created by the
        // built-in Claude agent, which is always in the no-`[[agents]]` default,
        // so its session must still resume.
        let (agent, keep) = resolve_restored_agent(&default_catalog(), None);
        assert_eq!(agent, AgentDefinition::claude_default().id);
        assert!(keep, "pre-feature Claude session stays resumable");
    }

    #[test]
    fn restored_removed_agent_falls_back_and_drops_session() {
        // The persisted owner is no longer in the catalog — fall back to the
        // default agent and drop the session id (resuming it would be invalid).
        let (agent, keep) = resolve_restored_agent(&default_catalog(), Some("ghost".to_string()));
        assert_eq!(agent, AgentDefinition::claude_default().id);
        assert!(!keep, "session id dropped when owning agent was removed");
    }

    #[test]
    fn restored_selects_owner_among_multiple_agents() {
        let agents = vec![
            AgentDefinition {
                id: "other".to_string(),
                name: "Other".to_string(),
                command: "run-other".to_string(),
            },
            AgentDefinition::claude_default(),
        ];
        // The default (catalog[0]) is "other", but a persisted claude owner that
        // is still present is kept, not overridden by the default.
        let (agent, keep) =
            resolve_restored_agent(&agents, Some(AgentDefinition::claude_default().id));
        assert_eq!(agent, AgentDefinition::claude_default().id);
        assert!(keep);
        // A removed owner falls back to catalog[0] = "other".
        let (agent, keep) = resolve_restored_agent(&agents, Some("gone".to_string()));
        assert_eq!(agent, "other");
        assert!(!keep);
    }

    fn two_agent_catalog() -> Vec<AgentDefinition> {
        vec![
            AgentDefinition {
                id: "other".to_string(),
                name: "Other".to_string(),
                command: "run-other".to_string(),
            },
            AgentDefinition::claude_default(),
        ]
    }

    #[test]
    fn open_agent_keeps_valid_last_id() {
        // A last id still in the catalog is kept, not overridden by catalog[0].
        let claude = AgentDefinition::claude_default().id;
        let agents = two_agent_catalog();
        assert_eq!(resolve_open_agent_id(&agents, Some(&claude)), claude);
    }

    #[test]
    fn open_agent_stale_last_id_falls_back_to_catalog_head() {
        // A last id no longer in the catalog falls back to catalog[0] = "other".
        let agents = two_agent_catalog();
        assert_eq!(resolve_open_agent_id(&agents, Some("gone")), "other");
    }

    #[test]
    fn open_agent_none_falls_back_to_catalog_head() {
        // No prior choice falls back to catalog[0] = "other".
        let agents = two_agent_catalog();
        assert_eq!(resolve_open_agent_id(&agents, None), "other");
    }
}
