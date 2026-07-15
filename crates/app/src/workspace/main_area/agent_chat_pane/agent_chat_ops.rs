//! Workspace ops for the Agent chat pane — pane/tab construction, the live
//! ACP connection + event pump, the desktop-notification pipeline, the
//! bottom-dock prompt / cancel routing, plus the GPUI-free helpers the
//! view's reconcilers reuse.
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
//! The Telegram relay domain (outbound pings, inbound reply/permission
//! injection) lives in the sibling [`super::telegram_ops`] module; this file
//! keeps only the two tee points — `maybe_notify_agent_event` and
//! `fire_activity_completion` — that call into it.
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
use daruda_config::AgentLaunch;
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::project::PaneCwd;
use futures::StreamExt as _;
use futures::channel::mpsc::unbounded;
use gpui::{AppContext as _, Context, Entity, Window};
use std::path::PathBuf;

use super::agent_chat_helpers::next_mode_id;
use super::slash_dispatch::{LocalSlashCommand, SlashDispatch, classify_slash};
use super::telegram_ops::DeferKind;
use super::view::{AgentChatView, AgentSessionStatus, PromptId, RuntimePrepPhase, TurnOutcome};
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

fn agent_name_for(agents: &[daruda_config::AgentDefinition], agent_id: &str) -> String {
    agents
        .iter()
        .find(|a| a.id == agent_id)
        .map(|a| a.name.clone())
        .unwrap_or_else(|| agent_id.to_string())
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

/// Cwd outcome fed into [`Workspace::build_agent_chat_pane`]'s status
/// derivation. Kept as one enum — rather than a `cwd: Option<PaneCwd>` +
/// separate error-reason field, which could disagree — so an "unattachable"
/// pane (an agent whose command needs `{{cwd}}` but the lane has no
/// `remote_cwd`) is a variant of its own with its own reason, instead of
/// being represented as `Ready(None)` with an ambiguous or overridden reason.
enum PaneCwdOutcome {
    /// A cwd resolution that completed normally: `Some` parks the pane
    /// `Idle` with that cwd; `None` parks it in the generic "no working
    /// directory" error (restore's genuinely cwd-less case).
    Ready(Option<PaneCwd>),
    /// The cwd could not be resolved at all — parks the pane in `Error`
    /// with the given (already-localized) reason, cwd `None`.
    Blocked(String),
}

/// Pure core of [`Workspace::resolve_new_pane_cwd`] — decide whether a fresh
/// Agent chat pane should attach a `Local` or `Remote` cwd, given the
/// candidate agent's `launch` and the active lane's local/remote paths.
/// Factored out of the workspace so it is unit-testable without gpui.
///
/// - `launch` needs a remote cwd (see [`AgentLaunch::needs_remote_cwd`]) and
///   `remote_cwd` is set → `Ok(Some(Remote))`.
/// - `launch` needs a remote cwd but `remote_cwd` is `None` (or blank —
///   empty or whitespace-only, e.g. a lane whose remote path field was left
///   as spaces) → `Err(())`: the agent has nowhere to attach, and the caller
///   must not attempt a connect.
/// - `launch` does not need a remote cwd → `Ok(local_cwd.map(Local))`,
///   `None` when there is no active lane at all (mirrors the pre-existing
///   no-lane-cwd case).
fn resolve_new_pane_cwd_core(
    launch: &AgentLaunch,
    local_cwd: Option<PathBuf>,
    remote_cwd: Option<String>,
) -> Result<Option<PaneCwd>, ()> {
    if launch.needs_remote_cwd() {
        // A blank remote_cwd has nothing to substitute for the remote path
        // — treat it the same as `None` rather than letting it flow into
        // `AgentLaunch::wrap` and produce a broken `cd  && ...` command.
        remote_cwd
            .filter(|cwd| !cwd.trim().is_empty())
            .map(PaneCwd::Remote)
            .map(Some)
            .ok_or(())
    } else {
        Ok(local_cwd.map(PaneCwd::Local))
    }
}

/// Whether to raise the notification: the channel must be enabled, and it is
/// suppressed only when the app is foreground AND the firing pane is the
/// focused one (the user is already looking at it). Mirrors the hook-
/// notification focus gate.
fn should_notify_agent_event(
    enabled: bool,
    skip_focused_pane: bool,
    app_active: bool,
    is_focused_pane: bool,
) -> bool {
    enabled && !(skip_focused_pane && app_active && is_focused_pane)
}

impl Workspace {
    /// Show a desktop notification `body` for agent-chat pane `pane_id`, gated
    /// by `enabled` and the shared focus rule. The single place the focus gate +
    /// title lookup live.
    ///
    /// The firing pane is "focused" only when it is the active lane's focused
    /// pane. A pane in a parked (non-active) lane never matches this global pane
    /// id, so a background lane's completion / wait always fires — the desired
    /// behavior, since the user cannot be looking at it.
    fn notify_agent_pane(&self, pane_id: PaneId, enabled: bool, body: String, cx: &Context<Self>) {
        let is_focused = self.active_runtime().focused_pane_id == pane_id;
        if !should_notify_agent_event(
            enabled,
            self.notifications.skip_focused_pane,
            crate::platform::attention::is_app_active(),
            is_focused,
        ) {
            return;
        }
        crate::platform::notifications::show(&self.pane_title(pane_id, cx), &body);
    }

    /// The pane's display title for a notification: its live session title,
    /// or the static tab-title fallback before the session reports one.
    /// Also read by `telegram_ops::Workspace::telegram_ping_body`, which is
    /// why this stays `pub(in crate::workspace)` rather than private —
    /// both files are `impl Workspace` blocks under the same module tree.
    pub(in crate::workspace) fn pane_title(&self, pane_id: PaneId, cx: &Context<Self>) -> String {
        self.agent_chat_view(pane_id)
            .and_then(|v| v.read(cx).session_title.clone())
            .unwrap_or_else(s::agent_chat_tab_title)
    }

    /// Fire the "waiting for input" desktop notification when the agent requests
    /// a permission decision. Called from the event pump BEFORE the event is
    /// folded into the view. This is a wait signal, distinct from turn
    /// completion — completion fires at the activity-settle edge via
    /// [`Self::fire_activity_completion`], not from the raw event. A no-op for
    /// every other event and when gated out.
    fn maybe_notify_agent_event(
        &mut self,
        pane_id: PaneId,
        event: &daruda_acp::AcpEvent,
        cx: &Context<Self>,
    ) {
        let daruda_acp::AcpEvent::PermissionRequested { id, request } = event else {
            return;
        };
        self.notify_agent_pane(
            pane_id,
            self.notifications.agent_waiting_enabled,
            s::agent_notification_waiting(),
            cx,
        );
        // Reuse the same protocol→view-model conversion
        // `AgentChatView::apply_event` uses to render the in-app
        // permission card, so this relay never has to name the raw ACP
        // protocol type — `daruda_acp`'s boundary is "host never
        // touches protocol types" (see `daruda_acp::mapping` module
        // docs). `permission_item` always returns `ChatItem::Permission`
        // for a `RequestPermissionRequest`; the `let else` is defensive.
        // Passes the view's current items (this runs before the event is
        // folded in, but any earlier tool_call for this id is already
        // there) so `permission_item` can prefer an already-clean
        // `raw_input` over the request's own, possibly adapter-mangled,
        // copy.
        let Some(view) = self.agent_chat_view(pane_id) else {
            return;
        };
        let daruda_acp::ChatItem::Permission(card) =
            daruda_acp::permission_item(*id, request, &view.read(cx).items)
        else {
            return;
        };
        self.relay_permission_wait_to_telegram(
            pane_id,
            *id,
            &card.options,
            card.tool_title.as_deref(),
            card.raw_input_summary.as_deref(),
            cx,
        );
    }

    /// Fire the "completed" desktop notification for a settled turn. Called only
    /// from [`Self::fire_activity_completion`] on a `Completed` outcome at the
    /// busy→idle activity-settle edge (not on the raw `TurnEnded` event, which
    /// may still have trailing subagents running).
    fn maybe_notify_agent_completed(&self, pane_id: PaneId, cx: &Context<Self>) {
        self.notify_agent_pane(
            pane_id,
            self.notifications.agent_completion_enabled,
            s::agent_notification_completed(),
            cx,
        );
    }

    /// Fire the completion signals for a pane whose activity span just settled:
    /// the "completed" desktop notification (only for `Completed`) and the
    /// backing task's terminal reconcile. The single completion firing point,
    /// driven by every [`AgentChatView::reconcile_activity`] caller when it
    /// returns `Some` on the busy→idle edge.
    pub(in crate::workspace) fn fire_activity_completion(
        &mut self,
        pane_id: PaneId,
        outcome: TurnOutcome,
        cx: &mut Context<Self>,
    ) {
        if matches!(outcome, TurnOutcome::Completed) {
            self.maybe_notify_agent_completed(pane_id, cx);
            let (header, tail) = self.telegram_completion_parts(pane_id, cx);
            self.relay_or_defer_to_telegram(pane_id, DeferKind::Completion, header, tail, None, cx);
        }
        let reason = match outcome {
            TurnOutcome::Completed | TurnOutcome::Stopped => {
                daruda_store::tasks::SessionEndReason::Stop
            }
            TurnOutcome::Errored => daruda_store::tasks::SessionEndReason::Error,
        };
        // Task tracking is a local-filesystem concept (`worktree_path` is
        // matched against a real path) — a `PaneCwd::Remote` pane has
        // nothing to match, so `into_local` skips it rather than passing a
        // remote id where a `Path` is expected.
        if let Some(cwd) = self
            .agent_chat_view(pane_id)
            .and_then(|v| v.read(cx).cwd.clone())
            .and_then(PaneCwd::into_local)
        {
            self.apply_agent_chat_task_ended(&cwd, reason, cx);
        }
    }

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
        cwd: Option<PaneCwd>,
        session_id: Option<String>,
        agent_id: String,
        title: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Pane {
        self.build_agent_chat_pane(
            PaneCwdOutcome::Ready(cwd),
            session_id,
            agent_id,
            title,
            window,
            cx,
        )
    }

    /// Shared pane-construction core behind [`Self::create_agent_chat_pane`]
    /// and [`Self::create_new_agent_chat_pane`]: builds the view + wraps it
    /// in a `Pane` for an already-decided cwd `outcome`. Factored out (and
    /// `outcome` kept as one enum rather than a `cwd` + separate error-reason
    /// pair) so a resolved-but-unattachable cwd (an agent whose command
    /// needs `{{cwd}}` but the lane has no `remote_cwd` configured) can still
    /// go through the single pane-construction path while carrying its own
    /// actionable error reason — distinct from the generic "no working
    /// directory" reason a genuinely cwd-less pane uses.
    fn build_agent_chat_pane(
        &mut self,
        outcome: PaneCwdOutcome,
        session_id: Option<String>,
        agent_id: String,
        title: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Pane {
        // The connection roots at the lane cwd; without one there is no working
        // directory to attach the agent to. Park such a pane in an error state
        // rather than a dormant `Idle` that could never connect. The cwd case
        // stays `Idle` until first focus. The status banner re-adds the error
        // prefix, so carry the bare reason here — not the prefix.
        let (cwd, status) = match outcome {
            PaneCwdOutcome::Ready(Some(cwd)) => (Some(cwd), AgentSessionStatus::Idle),
            PaneCwdOutcome::Ready(None) => {
                (None, AgentSessionStatus::Error(s::agent_chat_no_lane_cwd()))
            }
            PaneCwdOutcome::Blocked(reason) => (None, AgentSessionStatus::Error(reason)),
        };
        let pane_id = self.alloc_id();
        let window_handle = window.window_handle();
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
            let agent_name = agent_name_for(&self.agents, &agent_id);
            move |cx| {
                AgentChatView::new(
                    pane_id,
                    window_handle,
                    cwd,
                    status,
                    session_id,
                    agent_id,
                    agent_name,
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

    /// Resolve whether a freshly-created Agent chat pane for `agent_id`
    /// should attach a `Local` or `Remote` cwd. Looks up `agent_id`'s launch
    /// in the catalog and defers to [`resolve_new_pane_cwd_core`]; an id no
    /// longer in the catalog falls back to an empty `AgentLaunch::Raw`
    /// (no `{{cwd}}` token, so it behaves as a launch needing no remote
    /// cwd — falls back to `local_cwd`), matching `agent_launch_for`'s
    /// existing "id not found" handling elsewhere in this file.
    pub(in crate::workspace) fn resolve_new_pane_cwd(
        &self,
        agent_id: &str,
        local_cwd: Option<PathBuf>,
        remote_cwd: Option<String>,
    ) -> Result<Option<PaneCwd>, ()> {
        let launch = self
            .agent_launch_for(agent_id)
            .unwrap_or_else(|| AgentLaunch::Raw(String::new()));
        resolve_new_pane_cwd_core(&launch, local_cwd, remote_cwd)
    }

    /// Construct a fresh Agent chat pane for `agent_id`, resolving Local vs.
    /// Remote cwd via [`Self::resolve_new_pane_cwd`]. The single entry point
    /// every fresh-creation call site uses (`open_agent_chat_pane_with_agent`,
    /// `split_focused_pane_kind`, `finalize_create_lane`), so the
    /// no-remote-cwd fallback lives in one place instead of duplicated at
    /// each call site. `session_id`/`title` are always `None` here — a fresh
    /// pane never resumes a prior conversation; only restore
    /// (`persistence.rs`) passes those, and it calls
    /// [`Self::create_agent_chat_pane`] directly with an already-resolved
    /// `PaneCwd` from disk, skipping this resolution entirely (a persisted
    /// `Remote` pane's remote path was valid when saved and needs no
    /// re-derivation against a lane that may since have changed).
    pub(in crate::workspace) fn create_new_agent_chat_pane(
        &mut self,
        agent_id: String,
        local_cwd: Option<PathBuf>,
        remote_cwd: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Pane {
        let outcome = match self.resolve_new_pane_cwd(&agent_id, local_cwd, remote_cwd) {
            Ok(cwd) => PaneCwdOutcome::Ready(cwd),
            Err(()) => PaneCwdOutcome::Blocked(s::agent_chat_no_remote_cwd()),
        };
        self.build_agent_chat_pane(outcome, None, agent_id, None, window, cx)
    }

    /// The active lane's local path and remote cwd, or `(None, None)` when
    /// there is no active lane. The single source both fresh-pane cwd call
    /// sites (`open_agent_chat_pane_with_agent`, `split_focused_pane_kind`)
    /// read from, so they can't drift on how a lane-less workspace is
    /// represented.
    pub(in crate::workspace) fn active_lane_cwds(&self) -> (Option<PathBuf>, Option<String>) {
        match self.active_lane() {
            Some(lane) => (Some(lane.path.clone()), lane.remote_cwd.clone()),
            None => (None, None),
        }
    }

    /// The launch spec for `agent_id`, looked up in the catalog. `None` when
    /// the id is not in the catalog (e.g. a persisted id whose agent was removed).
    pub(in crate::workspace) fn agent_launch_for(&self, agent_id: &str) -> Option<AgentLaunch> {
        self.agents
            .iter()
            .find(|a| a.id == agent_id)
            .map(|a| a.launch.clone())
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
        let (local_cwd, remote_cwd) = self.active_lane_cwds();
        let pane = self.create_new_agent_chat_pane(agent_id, local_cwd, remote_cwd, window, cx);
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
            // Both `Local` and `Remote` panes are connectable: `connect_agent_chat`
            // substitutes the `{{cwd}}` token and rewrites the spawn cwd for a
            // `Remote` pane. A pane with no cwd at all was already parked in
            // `Error` at construction (`create_agent_chat_pane` /
            // `create_new_agent_chat_pane`) and never reaches `Idle`.
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

    /// Manual retry entry point for the Error banner's "Retry" button: start
    /// the ACP session for `pane_id` iff it is parked in
    /// [`AgentSessionStatus::Error`]. Mirrors [`Self::maybe_connect_agent_chat`]
    /// but gated on `Error` instead of `Idle`, and preserves the conversation:
    /// `AgentChatView::retry_for_reconnect` clears local `items` (so a
    /// resume's replay doesn't duplicate them) but keeps `session_id`, so this
    /// resumes the same session via `session/load` when one exists rather than
    /// starting over.
    pub(in crate::workspace) fn retry_agent_chat_connect(
        &mut self,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) {
        let (cwd, resume) = {
            let Some(view) = self.agent_chat_view(pane_id) else {
                return;
            };
            let v = view.read(cx);
            if !matches!(v.status, AgentSessionStatus::Error(_)) {
                return;
            }
            let Some(cwd) = v.cwd.clone() else {
                return;
            };
            (cwd, v.session_id.clone())
        };
        if let Some(view) = self.agent_chat_view(pane_id).cloned() {
            view.update(cx, |v, cx| v.retry_for_reconnect(cx));
            // Error → Connecting is a dock-badge status change; the cached
            // docks need an explicit dirty (see `notify_status_docks`).
            self.notify_status_docks(cx);
        }
        self.connect_agent_chat(pane_id, cwd, resume, cx);
    }

    /// Resolve the launch spec for `pane_id`'s agent, reconciling the view
    /// when its `agent_id` is stale. Happy path (allocation-free beyond the
    /// launch clone): the view's `agent_id` is still in the catalog, so return
    /// its launch unchanged. Miss: the persisted `agent_id` now points at an
    /// agent a live config reload removed/renamed, so it lies about which agent
    /// is running — rewrite the view to the session-sticky default, persist, and
    /// launch that agent instead. Returns `None` only when the pane is gone.
    fn resolve_pane_launch(
        &mut self,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) -> Option<AgentLaunch> {
        let agent_id = self.agent_chat_view(pane_id)?.read(cx).agent_id.clone();
        // Happy path: the view's agent is still in the catalog.
        if let Some(launch) = self.agent_launch_for(&agent_id) {
            return Some(launch);
        }
        // Stale id — reconcile so the chip / persisted state stop lying, then
        // launch the effective agent. `resolve_open_agent_id` yields a catalog
        // entry (or the built-in Claude id when the catalog is somehow empty).
        let effective_id = resolve_open_agent_id(&self.agents, self.last_agent_id.as_deref());
        if let Some(view) = self.agent_chat_view(pane_id).cloned() {
            let id = effective_id.clone();
            let name = agent_name_for(&self.agents, &effective_id);
            view.update(cx, |v, _| {
                v.agent_id = id;
                v.agent_name = name;
            });
            self.mark_dirty_and_save(cx);
        }
        Some(
            self.agent_launch_for(&effective_id)
                .unwrap_or_else(|| daruda_config::AgentDefinition::claude_default().launch),
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
        cwd: PaneCwd,
        resume: Option<String>,
        cx: &mut Context<Self>,
    ) {
        // Always offer the app's default permission mode; `run_connection`
        // applies it only when it creates a *fresh* session (no resume, or a
        // resume the agent can't `session/load` and so downgrades to
        // session/new) and skips it on a real load, which preserves the resumed
        // session's own mode. Passing it unconditionally is what lets a
        // downgraded resume still get the configured default (a `None` here would
        // leave the fresh session in the adapter's default).
        let initial_mode = Some(self.agent.default_permission_mode.mode_id().to_string());
        let node_root = daruda_store::persistence::node_install_dir();

        // Resolve the pane's agent_id → launch spec, reconciling the view
        // when its agent_id no longer names a catalog entry (a live config
        // reload removed/renamed that agent). Returns `None` only when the pane
        // is already gone — fall back to the catalog default launch so the
        // (soon-to-be-dropped) task still has a valid launch to wrap.
        let launch = self.resolve_pane_launch(pane_id, cx).unwrap_or_else(|| {
            self.agent_launch_for(&catalog_default_id(&self.agents))
                .unwrap_or_else(|| daruda_config::AgentDefinition::claude_default().launch)
        });
        // Read back after `resolve_pane_launch` (rather than threading its
        // internal read out) so a stale-id reconcile it performed is picked up
        // too. Only keys the dev-build wire-tap file name (see
        // `daruda_acp::connect_session`'s `agent_id` param) — never affects the
        // launch itself.
        let agent_id = self
            .agent_chat_view(pane_id)
            .map(|v| v.read(cx).agent_id.clone())
            .unwrap_or_default();

        // A `Remote` cwd's launch needs the lane's remote path assembled in
        // (`resolve_new_pane_cwd` only ever assigns `Remote` when
        // `AgentLaunch::needs_remote_cwd` is true) — `Local` needs no
        // remote path at all. This is the only place a connect resolves the
        // actual command/cwd pair to spawn, so a restored `Remote` pane
        // (which skips `resolve_new_pane_cwd`, see `persistence.rs`) still
        // gets wrapped here on its lazy connect.
        let (remote_path, connect_cwd): (Option<&str>, PathBuf) = match &cwd {
            PaneCwd::Remote(remote_path) => {
                (Some(remote_path.as_str()), PathBuf::from(remote_path))
            }
            PaneCwd::Local(path) => (None, path.clone()),
        };
        // `wrap` fails only when the pane's fixed `cwd` and the (possibly
        // just-reconciled) `launch`'s remote-cwd requirement now disagree —
        // e.g. a stale agent_id got swapped by `resolve_pane_launch` to a
        // catalog entry with different remote-cwd needs than this pane's
        // already-resolved `cwd`. A genuine edge case, not a "can't happen":
        // never spawn a connection with a broken command, park the pane in
        // the same "no remote cwd" error `PaneCwdOutcome::Blocked` uses, and
        // bail out of this connect attempt entirely.
        let command = match launch.wrap(remote_path) {
            Ok(command) => command,
            Err(()) => {
                if let Some(view) = self.agent_chat_view(pane_id).cloned() {
                    view.update(cx, |v, cx| v.set_error(s::agent_chat_no_remote_cwd(), cx));
                    // Connecting → Error clears the badge; dirty the cached
                    // docks so it doesn't linger stale (same pattern as every
                    // other Error transition in this function).
                    self.notify_status_docks(cx);
                }
                return;
            }
        };

        // DIAG: an ACP adapter spawn that fails with `os error 2` means the
        // launcher (`docker` / `npx` / `ssh`) was not on this process's PATH —
        // a GUI launch whose `hydrate_path_from_login_shell` was skipped or
        // bailed leaves only the minimal launchd PATH. Log the exact command +
        // effective PATH once per connect so the failure has ground truth
        // instead of a bare `-32603`. Info severity: no toast, NDJSON only.
        {
            let path = std::env::var("PATH").unwrap_or_else(|_| "<unset>".to_string());
            let report = daruda_store::observability::error_report::ErrorReport::new(
                "ACP connect: resolved launch command",
            )
            .severity(daruda_store::observability::error_report::ErrorSeverity::Info)
            .with_context("command", command.clone())
            .with_context("PATH", path)
            .at(file!(), line!())
            .dedup("agent_chat.connect.command")
            .build();
            daruda_store::observability::log_writer::LogWriter::log(report);
        }

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
                        connect_cwd,
                        initial_mode,
                        resume.map(daruda_acp::SessionId::new),
                        &agent_id,
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
                        let Some(view) = ws.agent_chat_view(pane_id).cloned() else {
                            return false;
                        };
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
                        // `pump_pending_prompt` may have flipped the turn
                        // Idle→InFlight. Reconcile immediately — exactly as the
                        // prompt-send driver does — so the idle→busy edge stamps
                        // `was_busy`. Without this, if the pane buffered its first
                        // prompt and the first ACP event is `TurnEnded`, the pump
                        // event's later reconcile would see `was_busy == false`,
                        // miss the busy→idle edge, and strand `pending_completion`
                        // forever (task stuck `Running`, no notification). A
                        // returned `Some` on this open edge is unexpected but
                        // harmless — firing it keeps the single completion point
                        // consistent.
                        let edge =
                            view.update(cx, |v, _| v.reconcile_activity(std::time::Instant::now()));
                        if let Some(outcome) = edge {
                            ws.fire_activity_completion(pane_id, outcome, cx);
                        }
                        true
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
                            // Desktop notification for a permission wait, gated by
                            // focus. Must borrow `&event` before the move into
                            // `apply_event` below. Turn *completion* fires later,
                            // at the activity-settle edge (see the reconcile below).
                            ws.maybe_notify_agent_event(pane_id, &event, cx);
                            view.update(cx, |v, cx| {
                                v.apply_event(event, &syntax_theme, is_light, cx)
                            });
                            // Advance the activity span now that the event folded
                            // in. When this event drove the last busy→idle
                            // transition (the turn ended and no subagent is still
                            // running), `reconcile_activity` returns the captured
                            // outcome and the completion signals fire exactly once.
                            // A still-running subagent leaves the pane busy, so the
                            // firing defers to the pulse tick that catches the
                            // quiescence settle. AgentChat-surfaced tasks reconcile
                            // off this edge (they never write the status-file hooks
                            // the Terminal surface uses).
                            let edge = view
                                .update(cx, |v, _| v.reconcile_activity(std::time::Instant::now()));
                            if let Some(outcome) = edge {
                                ws.fire_activity_completion(pane_id, outcome, cx);
                            }
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
                    // The stream ended — either the command channel closed (an
                    // intentional pane close dropped the handle) or the
                    // connection task ended without emitting a terminal event.
                    // Two independent safety nets fire here, both no-ops in the
                    // common already-terminal (Connected then closed) case:
                    //  - `abort_restore` releases a still-set replay gate so a
                    //    resume's partial replay renders instead of the pane
                    //    freezing mid-restore.
                    //  - a synthetic `AcpEvent::Error` resolves a status that
                    //    never reached `Connected`/`Error` — without this, a
                    //    connection task that exits silently before emitting
                    //    anything (its future dropped by an upstream bug
                    //    rather than returning `Err`) strands the pane on
                    //    "Connecting…" forever with no event left to move it
                    //    and no retry affordance (that requires `Error`). Fed
                    //    through `apply_event` (not a bespoke setter) so this
                    //    gets the exact same handling as any other terminal
                    //    error — turn settle, handle drop, pending-prompt
                    //    clear — instead of a partial hand-rolled duplicate
                    //    that would leave the turn/activity state stranded.
                    // SILENT-OK: view/window already gone at end-of-stream — nothing to release
                    let _ = this.update(cx, |ws, cx| {
                        if let Some(view) = ws.agent_chat_view(pane_id).cloned() {
                            let was_connecting = view.read(cx).is_still_connecting();
                            view.update(cx, |v, cx| v.abort_restore(cx));
                            if was_connecting {
                                let (syntax_theme, is_light) = ws.agent_chat_theme_params(cx);
                                view.update(cx, |v, cx| {
                                    v.apply_event(
                                        daruda_acp::AcpEvent::Error(
                                            s::agent_chat_error_stream_ended(),
                                        ),
                                        &syntax_theme,
                                        is_light,
                                        cx,
                                    )
                                });
                                // Connecting → Error clears the badge; dirty the
                                // cached docks so it doesn't linger stale.
                                ws.notify_status_docks(cx);
                                if let Some(cwd) =
                                    view.read(cx).cwd.clone().and_then(PaneCwd::into_local)
                                {
                                    ws.apply_agent_chat_task_ended(
                                        &cwd,
                                        daruda_store::tasks::SessionEndReason::Error,
                                        cx,
                                    );
                                }
                            }
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
                            if let Some(cwd) =
                                view.read(cx).cwd.clone().and_then(PaneCwd::into_local)
                            {
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
        // Same gate as `maybe_connect_agent_chat`: both `Local` and `Remote`
        // reconnect through `connect_agent_chat` here; only a genuinely
        // cwd-less pane (never had a session) no-ops.
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
                    // Flush a not-yet-quiesced post-turn follow-up before this new
                    // turn subsumes it (else its delta would be lost/merged).
                    if let Some(delta) = view.update(cx, |v, _| v.take_pending_post_turn()) {
                        self.relay_post_turn_to_telegram(pane_id, delta, cx);
                    }
                    view.update(cx, |v, cx| v.send_prompt_text(text, cx));
                    // Open the activity span on the idle→busy edge (stamps the
                    // working-indicator elapsed anchor at send). A returned
                    // `Some` is unexpected on open but harmless to fire — the
                    // single completion firing point stays consistent.
                    let edge =
                        view.update(cx, |v, _| v.reconcile_activity(std::time::Instant::now()));
                    if let Some(outcome) = edge {
                        self.fire_activity_completion(pane_id, outcome, cx);
                    }
                }
            }
        }
    }

    /// Remove a single queued prompt from an Agent chat pane. Shim for the
    /// bottom-dock queued-prompt strip's per-item × button: routes into the
    /// view, which drops the entry and notifies (one-way data flow). No-op when
    /// `pane_id` is gone or is not an Agent chat pane.
    pub(in crate::workspace) fn remove_queued_prompt(
        &mut self,
        pane_id: PaneId,
        id: PromptId,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.agent_chat_view(pane_id).cloned() {
            view.update(cx, |v, cx| v.remove_queued(id, cx));
        }
    }

    /// Clear every queued prompt from an Agent chat pane. Shim for the
    /// bottom-dock queued-prompt strip's "clear all" button: routes into the
    /// view, which empties the queue and notifies (one-way data flow). No-op
    /// when `pane_id` is gone or is not an Agent chat pane.
    pub(in crate::workspace) fn clear_queued_prompts(
        &mut self,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.agent_chat_view(pane_id).cloned() {
            view.update(cx, |v, cx| v.clear_queue(cx));
        }
    }

    /// Begin editing a queued prompt: pull its text into the bottom-dock
    /// composer (cursor at end) and mark the view editing that slot. Shim for
    /// the queued-prompt strip's ✎ button and for ↑ in an empty composer. On
    /// the next send, [`AgentChatView::send_prompt_text`] replaces the slot in
    /// place (order preserved). No-op when `pane_id` is gone, is not an Agent
    /// chat pane, or `id` is no longer queued.
    pub(in crate::workspace) fn begin_edit_queued_prompt(
        &mut self,
        pane_id: PaneId,
        id: PromptId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.agent_chat_view(pane_id).cloned() else {
            return;
        };
        // Reading the view entity here is safe — it is a different entity from
        // `self` (Workspace) and from `terminal_input`.
        let Some(text) = view
            .read(cx)
            .pending_prompts
            .iter()
            .find(|q| q.id == id)
            .map(|q| q.text.clone())
        else {
            return;
        };
        // Pull the text into the composer, cursor at end (mirrors
        // `do_history_navigate`). Separate `entity.update` from the view update
        // below — never nest two updates on entities in one call.
        self.terminal_input.update(cx, |s, cx_state| {
            s.set_value(&text, window, cx_state);
            s.move_cursor_to_end(cx_state);
        });
        view.update(cx, |v, cx| v.begin_edit(id, cx));
    }

    /// Cancel an in-progress queued-prompt edit: clear the view's editing flag
    /// and empty the composer. Shim for the strip's cancel (↩) button. No-op on
    /// the view side when `pane_id` is gone or is not an Agent chat pane; the
    /// composer is emptied regardless.
    pub(in crate::workspace) fn cancel_edit_queued_prompt(
        &mut self,
        pane_id: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.agent_chat_view(pane_id).cloned() {
            view.update(cx, |v, cx| v.cancel_edit(cx));
        }
        self.terminal_input
            .update(cx, |s, cx_state| s.set_value("", window, cx_state));
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
            // `cancel_turn` settles the turn locally, so this reconcile hits the
            // busy→idle edge and fires the stashed outcome (a live-turn Stop's
            // `Stopped`, or a trailing-subagent Stop's preserved completion) via
            // the single completion firing point.
            let edge = view.update(cx, |v, _| v.reconcile_activity(std::time::Instant::now()));
            if let Some(outcome) = edge {
                self.fire_activity_completion(pane_id, outcome, cx);
            }
        }
    }

    /// Cancel `pane_id`'s turn only when the pane is actually busy (a prompt turn
    /// in flight OR a background subagent still running). Backs the Escape
    /// shortcut (the keyboard counterpart of the "Stop" button): returns `true`
    /// when it cancelled, `false` when `pane_id` is not an Agent chat pane or is
    /// idle — in which case the caller lets Escape propagate as usual. Mirrors
    /// the `agent_stop_pane` snapshot condition that shows the Stop button.
    pub(in crate::workspace) fn cancel_agent_turn_if_active(
        &mut self,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(view) = self.agent_chat_view(pane_id).cloned() else {
            return false;
        };
        if !view.read(cx).is_busy() {
            return false;
        }
        view.update(cx, |v, cx| v.cancel_turn(cx));
        // Same settle-edge firing as `cancel_agent_turn`.
        let edge = view.update(cx, |v, _| v.reconcile_activity(std::time::Instant::now()));
        if let Some(outcome) = edge {
            self.fire_activity_completion(pane_id, outcome, cx);
        }
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
    use super::{
        resolve_new_pane_cwd_core, resolve_open_agent_id, resolve_restored_agent,
        should_notify_agent_event,
    };
    use daruda_config::{AgentDefinition, AgentLaunch};
    use daruda_store::project::PaneCwd;
    use std::path::PathBuf;

    fn ssh_launch() -> AgentLaunch {
        AgentLaunch::Ssh {
            adapter_command: "npx acp-adapter".into(),
            host: "vm-work".into(),
        }
    }

    fn docker_launch() -> AgentLaunch {
        AgentLaunch::Docker {
            adapter_command: "npx acp-adapter".into(),
            container: "devbox".into(),
        }
    }

    fn local_launch() -> AgentLaunch {
        AgentLaunch::Raw("npx acp-adapter".into())
    }

    #[test]
    fn resolve_new_pane_cwd_remote_launch_with_remote_cwd_is_remote() {
        // Ssh and Docker must behave identically here: both always need a
        // remote cwd (`AgentLaunch::needs_remote_cwd` is unconditionally true
        // for both), regardless of adapter_command contents.
        for launch in [ssh_launch(), docker_launch()] {
            let result = resolve_new_pane_cwd_core(
                &launch,
                Some(PathBuf::from("/local/lane")),
                Some("/remote/lane".to_string()),
            );
            assert_eq!(
                result,
                Ok(Some(PaneCwd::Remote("/remote/lane".to_string()))),
                "launch {launch:?} should resolve to Remote"
            );
        }
    }

    #[test]
    fn resolve_new_pane_cwd_remote_launch_without_remote_cwd_is_err() {
        for launch in [ssh_launch(), docker_launch()] {
            let result =
                resolve_new_pane_cwd_core(&launch, Some(PathBuf::from("/local/lane")), None);
            assert_eq!(result, Err(()), "launch {launch:?} should be Err");
        }
    }

    #[test]
    fn resolve_new_pane_cwd_remote_launch_with_blank_remote_cwd_is_err() {
        // An empty or whitespace-only `remote_cwd` (e.g. a lane whose remote
        // path field was set to just spaces) has nothing to substitute in —
        // it must behave exactly like `None`, not flow through as a bogus
        // `Remote("")`/`Remote("   ")`.
        for launch in [ssh_launch(), docker_launch()] {
            for blank in ["", "   ", "\t"] {
                let result = resolve_new_pane_cwd_core(
                    &launch,
                    Some(PathBuf::from("/local/lane")),
                    Some(blank.to_string()),
                );
                assert_eq!(
                    result,
                    Err(()),
                    "launch {launch:?}, blank remote_cwd {blank:?} should be Err"
                );
            }
        }
    }

    #[test]
    fn resolve_new_pane_cwd_local_launch_uses_local() {
        let result = resolve_new_pane_cwd_core(
            &local_launch(),
            Some(PathBuf::from("/local/lane")),
            Some("/remote/lane".to_string()),
        );
        assert_eq!(
            result,
            Ok(Some(PaneCwd::Local(PathBuf::from("/local/lane"))))
        );
    }

    #[test]
    fn resolve_new_pane_cwd_local_launch_no_local_is_none() {
        let result = resolve_new_pane_cwd_core(&local_launch(), None, None);
        assert_eq!(result, Ok(None));
    }

    #[test]
    fn should_notify_disabled_channel_never_fires() {
        assert!(!should_notify_agent_event(false, true, true, true));
        assert!(!should_notify_agent_event(false, false, false, false));
    }

    #[test]
    fn should_notify_foreground_focused_pane_is_suppressed() {
        assert!(!should_notify_agent_event(true, true, true, true));
    }

    #[test]
    fn should_notify_backgrounded_pane_fires() {
        // App is active but the firing pane is not the focused one.
        assert!(should_notify_agent_event(true, true, true, false));
    }

    #[test]
    fn should_notify_app_inactive_fires_even_for_focused_pane() {
        assert!(should_notify_agent_event(true, true, false, true));
    }

    #[test]
    fn should_notify_skip_focused_disabled_always_fires() {
        assert!(should_notify_agent_event(true, false, true, true));
    }

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
                launch: AgentLaunch::Raw("run-other".to_string()),
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
                launch: AgentLaunch::Raw("run-other".to_string()),
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
