//! `impl Workspace` ops for the Agent chat pane: pane/tab construction, the
//! desktop-notification pipeline, mode/config switching, and misc pane
//! accessors. What stays here needs `Workspace` state (default permission
//! mode, per-event `syntax_theme`, `report_error`); per-event folding and
//! render-listener ops live on [`AgentChatView`].
//!
//! The ACP connection + event pump live in the sibling
//! [`super::agent_chat_connect_ops`]; the prompt-queue send/edit/cancel
//! routing lives in [`super::agent_chat_queue_ops`]. Telegram relay lives in
//! [`super::telegram_ops`]; this file only tees into it from
//! `maybe_notify_agent_event` and `fire_activity_completion`.

use daruda_config::AgentLaunch;
use daruda_store::project::PaneCwd;
use gpui::{AppContext as _, Context, Entity, Window};
use std::path::PathBuf;

use super::telegram_ops::DeferKind;
use super::view::{AgentChatView, AgentSessionStatus, TurnOutcome};
use crate::surface::strings as s;
use crate::workspace::Workspace;
use crate::workspace::main_area::pane::{AgentChatContent, Pane, PaneContent, TabEntry};
use crate::workspace::main_area::pane_tree::{PaneId, PaneLayout};

/// The catalog's default agent id — the first entry, or the built-in Claude id
/// if the catalog is somehow empty (the config layer guarantees non-empty, so
/// the fallback is purely defensive).
pub(super) fn catalog_default_id(agents: &[daruda_config::AgentDefinition]) -> String {
    agents
        .first()
        .map(|a| a.id.clone())
        .unwrap_or_else(|| daruda_config::AgentDefinition::claude_default().id)
}

pub(super) fn agent_name_for(agents: &[daruda_config::AgentDefinition], agent_id: &str) -> String {
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
/// derivation. One enum (not a `cwd` + separate error-reason pair that could
/// disagree) so an unattachable pane carries its own reason as its own variant.
enum PaneCwdOutcome {
    /// Normal resolution: `Some` parks the pane `Idle` with that cwd; `None`
    /// parks it in the generic "no working directory" error (cwd-less restore).
    Ready(Option<PaneCwd>),
    /// Cwd unresolvable — parks the pane in `Error` with the given
    /// (already-localized) reason, cwd `None`.
    Blocked(String),
}

/// Pure core of [`Workspace::resolve_new_pane_cwd`]: decide whether a fresh
/// Agent chat pane attaches a `Local` or `Remote` cwd, given the candidate
/// agent's `launch` and the active lane's local/remote paths.
///
/// - Needs a remote cwd (see [`AgentLaunch::needs_remote_cwd`]) and `remote_cwd`
///   is set → `Ok(Some(Remote))`.
/// - Needs a remote cwd but `remote_cwd` is `None`/blank → `Err(())`: nowhere
///   to attach, the caller must not connect.
/// - Doesn't need a remote cwd → `Ok(local_cwd.map(Local))` (`None` for no lane).
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
/// suppressed only when the app is foreground AND the firing pane is focused
/// (the user is already looking at it). Mirrors the hook-notification gate.
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
    /// by `enabled` and the shared focus rule. The single focus-gate + title
    /// lookup site.
    ///
    /// A pane is "focused" only when it is the active lane's focused pane; a
    /// parked-lane pane never matches this global id, so a background lane's
    /// completion / wait always fires (the user cannot be looking at it).
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

    /// The pane's display title for a notification: its live session title, or
    /// the static tab-title fallback before the session reports one. Stays
    /// `pub(in crate::workspace)` because `telegram_ops` also reads it.
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
    pub(super) fn maybe_notify_agent_event(
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
        // Reuse `apply_event`'s protocol→view-model conversion so this relay
        // never names a raw ACP type (daruda_acp's "host never touches protocol
        // types" boundary). `permission_item` always returns `Permission` for a
        // permission request; the `let else` is defensive. Passing the view's
        // current items lets it prefer an already-clean `raw_input` (from an
        // earlier tool_call for this id) over the request's own copy.
        let Some(view) = self.agent_chat_view(pane_id).cloned() else {
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
        // Task tracking matches `worktree_path` against a real path, so a
        // `PaneCwd::Remote` pane has nothing to match — `into_local` skips it.
        if let Some(cwd) = self
            .agent_chat_view(pane_id)
            .and_then(|v| v.read(cx).cwd.clone())
            .and_then(PaneCwd::into_local)
        {
            self.apply_agent_chat_task_ended(&cwd, reason, cx);
        }
    }

    /// Construct an Agent chat `Pane` (no tab side-effects), parking the session
    /// in `Idle` (or `Error` with no lane cwd). The live ACP session is *not*
    /// started here — [`Self::focus_pane`] connects it lazily on first focus (via
    /// [`Self::maybe_connect_agent_chat`]), so cold restore doesn't spin up an
    /// agent process per pane. `window` is captured only for the window handle
    /// the view stores for later diff-editor creation.
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
    /// and [`Self::create_new_agent_chat_pane`]: builds the view + wraps it in a
    /// `Pane` for an already-decided cwd `outcome`. `outcome` is one enum so a
    /// resolved-but-unattachable cwd goes through this single path carrying its
    /// own reason, distinct from the generic "no working directory" reason.
    ///
    /// `account_id` is seeded with the Claude provider default (`None` when
    /// unset) — same as every other default-inheriting pane-creation site.
    /// Session restore (`Workspace::rebuild_layout` in `persistence.rs`) is
    /// the only caller with a persisted override to honor instead, and it
    /// unconditionally overwrites this seed on the returned `Pane` via
    /// `Pane::agent_chat_content_mut`, so seeding here is a no-op for that
    /// path.
    fn build_agent_chat_pane(
        &mut self,
        outcome: PaneCwdOutcome,
        session_id: Option<String>,
        agent_id: String,
        title: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Pane {
        // No lane cwd → no directory to attach to: park in `Error` rather than a
        // dormant `Idle` that could never connect. The status banner re-adds the
        // error prefix, so carry the bare reason here.
        let (cwd, status) = match outcome {
            PaneCwdOutcome::Ready(Some(cwd)) => (Some(cwd), AgentSessionStatus::Idle),
            PaneCwdOutcome::Ready(None) => {
                (None, AgentSessionStatus::Error(s::agent_chat_no_lane_cwd()))
            }
            PaneCwdOutcome::Blocked(reason) => (None, AgentSessionStatus::Error(reason)),
        };
        let pane_id = self.alloc_id();
        let window_handle = window.window_handle();
        // Seed the tab title from the persisted session title so a restored
        // dormant pane shows its label before the session loads; else default.
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
                    // Session restore (`Workspace::rebuild_layout` in
                    // `persistence.rs`) patches the persisted mode id in
                    // afterward via `agent_chat_content_mut`, same as it does
                    // for `content.account` — see that call site.
                    None,
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
                account: self.default_account_selection_for_new_pane(),
            }),
        }
    }

    /// Resolve whether a fresh Agent chat pane for `agent_id` attaches a `Local`
    /// or `Remote` cwd: looks up the launch and defers to
    /// [`resolve_new_pane_cwd_core`]. An id no longer in the catalog falls back
    /// to an empty `AgentLaunch::Raw` (needs no remote cwd → uses `local_cwd`).
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
    /// Remote cwd via [`Self::resolve_new_pane_cwd`]. The single fresh-creation
    /// entry point, so the no-remote-cwd fallback lives in one place.
    /// `session_id`/`title` are always `None` — a fresh pane never resumes;
    /// only restore passes those, calling [`Self::create_agent_chat_pane`]
    /// directly with an already-resolved `PaneCwd` from disk.
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

    /// The active lane's local path and remote cwd, or `(None, None)` when there
    /// is no active lane. Single source both fresh-pane cwd call sites read from.
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

    /// Resolve the agent a restored pane launches under, and whether its
    /// persisted session id is still resumable. A `None` `agent_id` means the
    /// built-in Claude agent. If the owning agent is still in the catalog,
    /// relaunch it and keep the session id (resume works); if removed, fall back
    /// to the default agent and drop the session id (resuming it is invalid).
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
        // Default to the last agent opened (session-local), falling back to the
        // catalog default; a stale last id (agent removed) also falls back.
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
        // The live session connects lazily via `focus_pane` below. The prompt
        // input lives in the bottom dock; open the dock first so it's visible
        // before `focus_pane` activates the input panel and moves focus to it.
        // The focused *pane* stays this one, so input routes to its ACP session.
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
        // Persist the new `last_known_mode_id` so a resume after restart
        // reapplies it (see that field's doc).
        self.mark_dirty_and_save(cx);
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
        let Some(next) = view.read(cx).session_config.next_mode_id() else {
            return false;
        };
        view.update(cx, |v, cx| v.set_mode(next, cx));
        // The bottom-input placeholder includes the current mode name;
        // refresh it now that the mode has cycled.
        self.refresh_terminal_input_placeholder(cx);
        // Persist the new `last_known_mode_id` so a resume after restart
        // reapplies it (see that field's doc).
        self.mark_dirty_and_save(cx);
        true
    }

    /// The (syntax theme, is-light) pair the Markdown / diff reconcilers read.
    /// `is_light` is judged by [`crate::ui::theme::agent_chat_syntax_is_light`]
    /// — the agent-chat pane's actual paint surface (the terminal-preset
    /// background mirrored into `agent_chat_bg`), not the UI theme's own
    /// light/dark bit, so highlighted diffs and mermaid diagrams stay legible
    /// against the background they really render on even when the terminal
    /// preset and UI theme disagree on light vs dark. `agent_chat_bg` itself
    /// falls back to a dark default before any config load, so this needs no
    /// separate "theme global not installed" guard. The Workspace owns
    /// `syntax_theme` (config mirror), so it reads it here and passes it to
    /// the view per event.
    pub(in crate::workspace) fn agent_chat_theme_params(
        &self,
        cx: &Context<Self>,
    ) -> (String, bool) {
        let is_light = crate::ui::theme::agent_chat_syntax_is_light(cx);
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

    /// The AgentChat pane's [`AccountSelection`]. Same cross-lane scan as
    /// [`Self::agent_chat_view`] and for the same reason — `connect_agent_chat`
    /// resolves this at connect time, which can happen after a lane switch
    /// moved the pane out of the active runtime. Falls back to
    /// [`AccountSelection::SystemDefault`] when the pane is gone or isn't an
    /// AgentChat pane (a resolve-time no-op — same effect as no override).
    pub(in crate::workspace) fn agent_chat_account_selection(
        &self,
        pane_id: PaneId,
    ) -> daruda_store::accounts::AccountSelection {
        self.main_area
            .runtimes
            .values()
            .flat_map(|rt| rt.panes.iter())
            .find(|p| p.id == pane_id)
            .and_then(Pane::agent_chat_content)
            .map(|ac| ac.account)
            .unwrap_or(daruda_store::accounts::AccountSelection::SystemDefault)
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
                default_mode: None,
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
                default_mode: None,
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
