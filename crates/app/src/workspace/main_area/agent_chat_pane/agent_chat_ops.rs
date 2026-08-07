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
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::project::{LaneSessionHost, PaneCwd};
use gpui::{App, AppContext as _, Context, Entity, Window};
use std::path::{Path, PathBuf};

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
        .map(|a| a.name.trim())
        .filter(|name| !name.is_empty())
        .map(str::to_string)
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

/// Pure core of the agent-id choice for a freshly opened pane: keep `last`
/// when still in the catalog, else fall back to `agents[0]`. Factored out for
/// unit-testability without gpui.
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

/// Pure core of [`Workspace::resolve_new_pane_cwd`]: decide `Local` vs.
/// `Remote` cwd for a fresh pane, mirroring what its first connect will
/// resolve (`resolve_session_command` in `agent_chat_connect_ops.rs`) so a
/// freshly opened pane's `PaneCwd` — the value the rest of the app reads via
/// `PaneCwd::as_local`/`into_local` — already agrees with where the session
/// actually attaches.
///
/// The legacy `Raw` + `{{cwd}}` token escape hatch is the one shape that
/// still needs a non-blank `remote_cwd` up front — `Err(())` (nowhere to
/// attach, caller must not connect) when it has none, exactly as before this
/// axis moved to the lane. Every other launch shape (`Ssh`/`Docker`/a
/// host-agnostic `Raw`) now resolves through `session_host` and always
/// succeeds — `session_host` grew a session path or it didn't, and either
/// way there's a valid place to attach (remote path, or the lane's own
/// local one).
fn resolve_new_pane_cwd_core(
    launch: &AgentLaunch,
    local_cwd: Option<PathBuf>,
    remote_cwd: Option<String>,
    session_host: Option<&LaneSessionHost>,
    session_host_catalog: &[daruda_config::SessionHostEntry],
    session_host_tombstones: &[daruda_config::SessionHostTombstone],
) -> Result<Option<PaneCwd>, ()> {
    if matches!(launch, AgentLaunch::Raw(command) if command.contains(daruda_config::agent::CWD_TOKEN))
    {
        // A blank remote_cwd has nothing to substitute for the remote path
        // — treat it the same as `None` rather than letting it flow into
        // `AgentLaunch::wrap` and produce a broken `cd  && ...` command.
        return remote_cwd
            .filter(|cwd| !cwd.trim().is_empty())
            .map(PaneCwd::Remote)
            .map(Some)
            .ok_or(());
    }
    let Some(local_cwd) = local_cwd else {
        return Ok(None); // no active lane at all
    };
    let host = crate::lane::session_host::effective_session_host(
        session_host,
        remote_cwd.as_deref(),
        launch,
        session_host_catalog,
        session_host_tombstones,
    );
    Ok(Some(match host.session_path() {
        Some(remote_path) => PaneCwd::Remote(remote_path.to_string()),
        None => PaneCwd::Local(local_cwd),
    }))
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

#[derive(Clone, Debug, PartialEq, Eq)]
struct MarkdownFileLinkTarget {
    path: PathBuf,
    line: Option<usize>,
}

fn markdown_file_link_target(link: &str, cwd: Option<&Path>) -> Option<MarkdownFileLinkTarget> {
    let path = if let Some(path) = file_url_path(link) {
        path
    } else if is_external_url(link) {
        return None;
    } else {
        let path = PathBuf::from(link);
        if path.is_absolute() {
            path
        } else if link.starts_with("./")
            || link.starts_with("../")
            || link.contains('/')
            || cwd
                .map(|cwd| strip_markdown_line_suffix(cwd.join(&path)).path.is_file())
                .unwrap_or(false)
        {
            cwd?.join(path)
        } else {
            return None;
        }
    };

    Some(strip_markdown_line_suffix(path))
}

fn file_url_path(link: &str) -> Option<PathBuf> {
    let rest = link.strip_prefix("file://")?;
    let path = if let Some(path) = rest.strip_prefix("localhost/") {
        format!("/{path}")
    } else if rest.starts_with('/') {
        rest.to_string()
    } else {
        return None;
    };
    Some(PathBuf::from(percent_decode_path(&path)?))
}

fn percent_decode_path(path: &str) -> Option<String> {
    if !path.as_bytes().contains(&b'%') {
        return Some(path.to_string());
    }

    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hi = hex_value(*bytes.get(i + 1)?)?;
            let lo = hex_value(*bytes.get(i + 2)?)?;
            out.push((hi << 4) | lo);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn hex_value(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn is_external_url(link: &str) -> bool {
    link.contains("://")
        || link.starts_with("mailto:")
        || link.starts_with("tel:")
        || link.starts_with('#')
}

fn parse_numeric_suffix(suffix: &str) -> Option<Option<usize>> {
    if suffix.is_empty() || !suffix.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(suffix.parse::<usize>().ok().filter(|n| *n > 0))
}

fn strip_markdown_line_suffix(path: PathBuf) -> MarkdownFileLinkTarget {
    if path.is_file() {
        return MarkdownFileLinkTarget { path, line: None };
    }

    let Some(mut s) = path.to_str().map(str::to_owned) else {
        return MarkdownFileLinkTarget { path, line: None };
    };
    let mut line = None;
    for _ in 0..2 {
        let Some((prefix, suffix)) = s.rsplit_once(':') else {
            break;
        };
        let Some(parsed) = parse_numeric_suffix(suffix) else {
            break;
        };
        if let Some(n) = parsed {
            line = Some(n);
        }
        s = prefix.to_string();
        let stripped = PathBuf::from(&s);
        if stripped.is_file() {
            return MarkdownFileLinkTarget {
                path: stripped,
                line,
            };
        }
    }
    MarkdownFileLinkTarget {
        path: PathBuf::from(s),
        line,
    }
}

impl Workspace {
    /// Show a desktop notification `body` for `pane_id`, gated by `enabled`
    /// and the shared focus rule. A parked-lane pane never matches the
    /// focused-pane id, so its completion/wait always fires.
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

    /// Fire the "waiting for input" notification on a permission request.
    /// Called from the event pump BEFORE folding into the view — distinct
    /// from turn completion, which fires only via [`Self::fire_activity_completion`].
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

    /// Fire the "completed" notification. Called only from
    /// [`Self::fire_activity_completion`] on the busy→idle settle edge, not
    /// the raw `TurnEnded` (which may still have trailing subagents running).
    fn maybe_notify_agent_completed(&self, pane_id: PaneId, cx: &Context<Self>) {
        self.notify_agent_pane(
            pane_id,
            self.notifications.agent_completion_enabled,
            s::agent_notification_completed(),
            cx,
        );
    }

    /// Fire the completion signals once a pane's activity span settles: the
    /// "completed" notification (only for `Completed`) and the backing task's
    /// terminal reconcile. The single completion firing point.
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

    /// Construct an Agent chat `Pane` (no tab side-effects), parked `Idle` (or
    /// `Error` with no lane cwd). The live ACP session is *not* started here —
    /// `focus_pane` connects it lazily on first focus, so cold restore doesn't
    /// spin up an agent process per pane.
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
    /// and [`Self::create_new_agent_chat_pane`]: builds the view + wraps it in
    /// a `Pane` for an already-decided cwd `outcome`. Account defaults to the
    /// configured default for this agent's own auth domain; session restore
    /// overwrites that seed via `Pane::agent_chat_content_mut` right after.
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
        // Only this agent's own auth domain has a default that may apply
        // here — and only when this cwd is actually local: a managed
        // account's config dir is a local path, so it must never seed a
        // pane whose resolved cwd is remote (see `account_recipe`'s doc).
        let is_remote = matches!(cwd, Some(PaneCwd::Remote(_)));
        let account = self.default_account_selection_for_new_pane(
            self.agent_launch_for(&agent_id)
                .and_then(|l| l.account_recipe(is_remote)),
        );
        // `title` seeds the view's `session_title` below (restored dormant
        // panes show their persisted label before the session loads);
        // `Pane::title()` reads it live, so there's no separate cache to seed
        // here.
        let agent_name = agent_name_for(&self.agents, &agent_id);
        // The view owns its own `cwd` (for connect / persistence); the wrapper
        // caches a copy so `Pane::cwd()` stays cx-free.
        let view = cx.new({
            let cwd = cwd.clone();
            let agent_name = agent_name.clone();
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
            content: PaneContent::AgentChat(AgentChatContent { view, cwd, account }),
        }
    }

    /// Resolve `Local` vs. `Remote` cwd for a fresh pane under `agent_id`,
    /// via [`resolve_new_pane_cwd_core`]. An id no longer in the catalog falls
    /// back to an empty `AgentLaunch::Raw` (uses `local_cwd`).
    pub(in crate::workspace) fn resolve_new_pane_cwd(
        &self,
        agent_id: &str,
        local_cwd: Option<PathBuf>,
        remote_cwd: Option<String>,
        session_host: Option<&LaneSessionHost>,
    ) -> Result<Option<PaneCwd>, ()> {
        let launch = self
            .agent_launch_for(agent_id)
            .unwrap_or_else(|| AgentLaunch::Raw(String::new()));
        resolve_new_pane_cwd_core(
            &launch,
            local_cwd,
            remote_cwd,
            session_host,
            &self.session_hosts,
            &self.session_host_tombstones,
        )
    }

    /// Construct a fresh Agent chat pane for `agent_id`. The single
    /// fresh-creation entry point; `session_id`/`title` are always `None` — a
    /// fresh pane never resumes (only restore passes those, via
    /// [`Self::create_agent_chat_pane`] directly).
    pub(in crate::workspace) fn create_new_agent_chat_pane(
        &mut self,
        agent_id: String,
        local_cwd: Option<PathBuf>,
        remote_cwd: Option<String>,
        session_host: Option<LaneSessionHost>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Pane {
        let outcome = match self.resolve_new_pane_cwd(
            &agent_id,
            local_cwd,
            remote_cwd,
            session_host.as_ref(),
        ) {
            Ok(cwd) => PaneCwdOutcome::Ready(cwd),
            Err(()) => PaneCwdOutcome::Blocked(s::agent_chat_no_remote_cwd()),
        };
        self.build_agent_chat_pane(outcome, None, agent_id, None, window, cx)
    }

    /// The active lane's local path, legacy `remote_cwd`, and `session_host`
    /// — `(None, None, None)` when there is no active lane. Single source
    /// every fresh-pane cwd call site reads from.
    pub(in crate::workspace) fn active_lane_cwds(
        &self,
    ) -> (Option<PathBuf>, Option<String>, Option<LaneSessionHost>) {
        match self.active_lane() {
            Some(lane) => (
                Some(lane.path.clone()),
                lane.remote_cwd.clone(),
                lane.session_host.clone(),
            ),
            None => (None, None, None),
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
    /// persisted session id is still resumable. If the owning agent was
    /// removed from the catalog, fall back to the default and drop the id.
    pub(in crate::workspace) fn resolve_restored_agent(
        &self,
        persisted_agent_id: Option<String>,
    ) -> (String, bool /* keep session id */) {
        resolve_restored_agent(&self.agents, persisted_agent_id)
    }

    /// Open a fresh pane under the session's last-chosen agent (falling back
    /// to the catalog default). Thin wrapper over
    /// [`Self::open_agent_chat_pane_with_agent`].
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

    /// Open a fresh pane under `agent_id` at the active lane's cwd. Mirrors
    /// `open_task_edit_pane`'s tab-append + focus flow, and records
    /// `agent_id` as the session's last choice.
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
        let (local_cwd, remote_cwd, session_host) = self.active_lane_cwds();
        let pane = self.create_new_agent_chat_pane(
            agent_id,
            local_cwd,
            remote_cwd,
            session_host,
            window,
            cx,
        );
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

    /// Switch the active session mode. Shim for the mode chip: routes the
    /// chosen id into the view, which optimistically updates and sends
    /// `session/set_mode`. No-op when `pane_id` isn't an Agent chat pane.
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
        self.mutate_durable(cx, |_, _| {});
    }

    /// Change a config option (model / effort / a boolean toggle / …). Shim
    /// for the config chips: routes `(config_id, value)` into the view, which
    /// optimistically updates and sends `session/set_config_option`.
    pub(in crate::workspace) fn set_agent_config_option(
        &mut self,
        pane_id: PaneId,
        config_id: String,
        value: daruda_acp::ConfigValueView,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.agent_chat_view(pane_id).cloned() {
            view.update(cx, |v, cx| v.set_config_option(config_id, value, cx));
        }
    }

    /// Advance to the next advertised session mode, wrapping at the end. Backs
    /// the Shift+Tab shortcut. `false` (no switch) when not an Agent chat pane
    /// or fewer than two modes advertised — the caller then lets it outdent.
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
        self.mutate_durable(cx, |_, _| {});
        true
    }

    /// The (syntax theme, is-light) pair the Markdown/diff reconcilers read.
    /// `is_light` judges the pane's actual paint surface (`agent_chat_bg`),
    /// not the UI theme's light/dark bit, so diffs stay legible even when the
    /// terminal preset and UI theme disagree.
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
    /// panes, not just the active one — the event pump looks the view up on
    /// every ACP event, and a lane switch only re-points `self.active`; an
    /// active-lane-only lookup would break the pump's loop on a parked lane
    /// and drop that session's responses forever. Pane ids are
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

    /// Mutable counterpart of [`Self::agent_chat_view`]'s cross-lane scan —
    /// same reason (a parked lane's pane still needs reaching). Used to keep
    /// `Pane::agent_chat_content_mut().cwd` (the cx-free wrapper cache
    /// `Pane::cwd()` reads) in step with the view's own `cwd` when a connect
    /// resolves somewhere new.
    pub(in crate::workspace) fn pane_mut(&mut self, pane_id: PaneId) -> Option<&mut Pane> {
        self.main_area
            .runtimes
            .values_mut()
            .flat_map(|rt| rt.panes.iter_mut())
            .find(|p| p.id == pane_id)
    }

    /// The lane that owns `pane_id`, found by the same cross-lane scan as
    /// [`Self::agent_chat_view`] (a pane's owning lane never changes, but a
    /// lane switch only re-points `self.active`, so this must not be
    /// active-lane-only either). Lets a connect resolve *this* lane's session
    /// host even for a parked pane, rather than trusting a persisted
    /// `PaneCwd::Remote` that carries no host of its own — see
    /// `connect_agent_chat`.
    pub(in crate::workspace) fn lane_ref_for_pane(
        &self,
        pane_id: PaneId,
    ) -> Option<daruda_store::project::LaneRef> {
        self.main_area
            .runtimes
            .iter()
            .find(|(_, rt)| rt.panes.iter().any(|p| p.id == pane_id))
            .map(|(lane_ref, _)| *lane_ref)
    }

    /// True when `pane_id`'s session runs on a remote host (`PaneCwd::Remote`
    /// — an SSH/Docker session host, not "pane closed"/"unknown"). ACP paths
    /// the agent reports (`Diff.path`, tool `raw_input` paths, …) are on
    /// *that* host's filesystem, not this machine's, so any action that reads
    /// the path from local disk (the file viewer, an external editor) would
    /// either fail or — worse — silently show an unrelated local file that
    /// happens to share the same absolute path.
    fn diff_pane_is_remote(&self, pane_id: PaneId, cx: &App) -> bool {
        self.agent_chat_view(pane_id)
            .is_some_and(|view| matches!(view.read(cx).cwd, Some(PaneCwd::Remote(_))))
    }

    /// Open a diff block's file in the pane-area file viewer. Dispatched from
    /// the agent-chat diff header (`render/diff.rs`); `path` is ACP's
    /// `Diff.path`, which the spec guarantees absolute, so no lane-root join is
    /// needed. A no-op if `pane_id`'s lane can't be resolved (pane closed
    /// mid-click — the render that produced this callback is already gone).
    /// Toasts and returns instead if the pane's session is remote — see
    /// [`Self::diff_pane_is_remote`].
    pub(in crate::workspace) fn open_diff_in_file_view(
        &mut self,
        pane_id: PaneId,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(lane) = self.lane_ref_for_pane(pane_id) else {
            return;
        };
        if self.diff_pane_is_remote(pane_id, cx) {
            let report = ErrorReport::new(s::diff_remote_path_unsupported())
                .severity(ErrorSeverity::Warning)
                .at(file!(), line!())
                .dedup("agent_chat.diff.remote_path_unsupported")
                .build();
            self.report_error(report, cx);
            return;
        }
        self.open_pane_file_view(
            lane.lane,
            path,
            false,
            crate::workspace::main_area::file_view_pane::FileViewMode::Raw,
            window,
            cx,
        );
    }

    /// Open a file-shaped link from rendered agent-chat Markdown in the pane
    /// file viewer. Returns `false` for normal URLs so the caller can fall back
    /// to the platform URL opener. Handles the file-link shape this app emits
    /// in chat (`/abs/path:line`) by stripping the line suffix before opening.
    pub(in crate::workspace) fn open_agent_chat_markdown_file_link(
        &mut self,
        pane_id: PaneId,
        link: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let cwd = self.agent_chat_view(pane_id).and_then(|view| {
            let view = view.read(cx);
            match &view.cwd {
                Some(PaneCwd::Local(path)) => Some(path.clone()),
                Some(PaneCwd::Remote(_)) | None => None,
            }
        });
        let Some(target) = markdown_file_link_target(link, cwd.as_deref()) else {
            return false;
        };
        let Some(lane) = self.lane_ref_for_pane(pane_id) else {
            return true;
        };
        if self.diff_pane_is_remote(pane_id, cx) {
            let report = ErrorReport::new(s::diff_remote_path_unsupported())
                .severity(ErrorSeverity::Warning)
                .at(file!(), line!())
                .dedup("agent_chat.markdown_file_link.remote_path_unsupported")
                .build();
            self.report_error(report, cx);
            return true;
        }
        self.open_pane_file_view(
            lane.lane,
            target.path,
            false,
            crate::workspace::main_area::file_view_pane::FileViewMode::Raw,
            window,
            cx,
        );
        if let Some(line) = target.line {
            self.set_file_view_mode(
                crate::workspace::main_area::file_view_pane::FileViewMode::Raw,
                cx,
            );
            self.scroll_focused_file_viewer_to_line(line, window, cx);
        }
        true
    }

    /// Open a diff block's file externally — the user's preferred editor
    /// (Settings → External Editor), or the OS default handler when none is
    /// set. Dispatched from the agent-chat diff header, same shape as
    /// [`Self::open_diff_in_file_view`], including the no-op-on-missing-lane
    /// and remote-session guard.
    pub(in crate::workspace) fn open_diff_externally(
        &mut self,
        pane_id: PaneId,
        path: PathBuf,
        cx: &mut Context<Self>,
    ) {
        let Some(lane) = self.lane_ref_for_pane(pane_id) else {
            return;
        };
        if self.diff_pane_is_remote(pane_id, cx) {
            let report = ErrorReport::new(s::diff_remote_path_unsupported())
                .severity(ErrorSeverity::Warning)
                .at(file!(), line!())
                .dedup("agent_chat.diff.remote_path_unsupported")
                .build();
            self.report_error(report, cx);
            return;
        }
        self.open_file_externally(lane.lane, path, cx);
    }

    /// The AgentChat pane's `AccountSelection`. Same cross-lane scan as
    /// [`Self::agent_chat_view`], for the same reason. Falls back to
    /// `SystemDefault` when the pane is gone or isn't an AgentChat pane.
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
        MarkdownFileLinkTarget, markdown_file_link_target, resolve_new_pane_cwd_core,
        resolve_open_agent_id, resolve_restored_agent, should_notify_agent_event,
    };
    use daruda_config::{AgentDefinition, AgentLaunch};
    use daruda_store::project::{LaneSessionHost, PaneCwd};
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

    fn token_launch() -> AgentLaunch {
        AgentLaunch::Raw("ssh vm-work \"cd {{cwd}} && run\"".into())
    }

    #[test]
    fn resolve_new_pane_cwd_legacy_remote_combo_is_remote() {
        // The legacy pair (agent-side `Ssh`/`Docker` + the lane's own
        // `remote_cwd`, no `session_host` answered yet) still resolves
        // remotely — `effective_session_host`'s legacy fallback branch.
        for launch in [ssh_launch(), docker_launch()] {
            let result = resolve_new_pane_cwd_core(
                &launch,
                Some(PathBuf::from("/local/lane")),
                Some("/remote/lane".to_string()),
                None,
                &[],
                &[],
            );
            assert_eq!(
                result,
                Ok(Some(PaneCwd::Remote("/remote/lane".to_string()))),
                "launch {launch:?} should resolve to Remote"
            );
        }
    }

    #[test]
    fn resolve_new_pane_cwd_remote_launch_without_remote_cwd_falls_back_to_local() {
        // No `Err` case for `Ssh`/`Docker` anymore — with nothing on either
        // axis to name a host, the lane falls back to `Local` rather than
        // blocking the pane (mirrors `a_half_configured_legacy_pair_stays_local`
        // in `lane::session_host`).
        for launch in [ssh_launch(), docker_launch()] {
            let result = resolve_new_pane_cwd_core(
                &launch,
                Some(PathBuf::from("/local/lane")),
                None,
                None,
                &[],
                &[],
            );
            assert_eq!(
                result,
                Ok(Some(PaneCwd::Local(PathBuf::from("/local/lane")))),
                "launch {launch:?} should fall back to Local"
            );
        }
    }

    #[test]
    fn resolve_new_pane_cwd_remote_launch_with_blank_remote_cwd_falls_back_to_local() {
        // An empty or whitespace-only legacy `remote_cwd` has nothing to
        // substitute in — same fallback as no `remote_cwd` at all, not a
        // bogus `Remote("")`/`Remote("   ")`.
        for launch in [ssh_launch(), docker_launch()] {
            for blank in ["", "   ", "\t"] {
                let result = resolve_new_pane_cwd_core(
                    &launch,
                    Some(PathBuf::from("/local/lane")),
                    Some(blank.to_string()),
                    None,
                    &[],
                    &[],
                );
                assert_eq!(
                    result,
                    Ok(Some(PaneCwd::Local(PathBuf::from("/local/lane")))),
                    "launch {launch:?}, blank remote_cwd {blank:?} should fall back to Local"
                );
            }
        }
    }

    #[test]
    fn resolve_new_pane_cwd_session_host_overrides_a_local_agent_to_remote() {
        // The point of the lane axis: a plain local `Raw` agent still
        // attaches remotely when the lane itself answered `session_host`.
        let host = LaneSessionHost::Ssh {
            target: "vm-work".into(),
            session_path: "/srv/app".into(),
            registry_id: None,
        };
        let result = resolve_new_pane_cwd_core(
            &local_launch(),
            Some(PathBuf::from("/local/lane")),
            None,
            Some(&host),
            &[],
            &[],
        );
        assert_eq!(result, Ok(Some(PaneCwd::Remote("/srv/app".to_string()))));
    }

    #[test]
    fn resolve_new_pane_cwd_session_host_local_wins_over_legacy_remote_cwd() {
        // `Some(Local)` retires the legacy pair even for a fresh pane —
        // matches `answering_local_retires_the_legacy_pair`.
        let result = resolve_new_pane_cwd_core(
            &ssh_launch(),
            Some(PathBuf::from("/local/lane")),
            Some("/legacy/path".to_string()),
            Some(&LaneSessionHost::Local),
            &[],
            &[],
        );
        assert_eq!(
            result,
            Ok(Some(PaneCwd::Local(PathBuf::from("/local/lane"))))
        );
    }

    #[test]
    fn resolve_new_pane_cwd_local_launch_uses_local() {
        let result = resolve_new_pane_cwd_core(
            &local_launch(),
            Some(PathBuf::from("/local/lane")),
            Some("/remote/lane".to_string()),
            None,
            &[],
            &[],
        );
        assert_eq!(
            result,
            Ok(Some(PaneCwd::Local(PathBuf::from("/local/lane"))))
        );
    }

    #[test]
    fn resolve_new_pane_cwd_local_launch_no_local_is_none() {
        let result = resolve_new_pane_cwd_core(&local_launch(), None, None, None, &[], &[]);
        assert_eq!(result, Ok(None));
    }

    #[test]
    fn resolve_new_pane_cwd_token_escape_hatch_still_errs_without_remote_cwd() {
        // The one shape that keeps blocking: the legacy `{{cwd}}` token is
        // untouched by the lane session-host axis (see
        // `session_host::adapter_command`'s doc), so it still needs a
        // non-blank `remote_cwd` up front, `session_host` notwithstanding.
        let host = LaneSessionHost::Ssh {
            target: "other-host".into(),
            session_path: "/other/path".into(),
            registry_id: None,
        };
        let result = resolve_new_pane_cwd_core(
            &token_launch(),
            Some(PathBuf::from("/local/lane")),
            None,
            Some(&host),
            &[],
            &[],
        );
        assert_eq!(result, Err(()));
    }

    #[test]
    fn resolve_new_pane_cwd_token_escape_hatch_uses_its_own_remote_cwd() {
        let result = resolve_new_pane_cwd_core(
            &token_launch(),
            Some(PathBuf::from("/local/lane")),
            Some("/remote/lane".to_string()),
            None,
            &[],
            &[],
        );
        assert_eq!(
            result,
            Ok(Some(PaneCwd::Remote("/remote/lane".to_string())))
        );
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

    #[test]
    fn markdown_file_link_strips_absolute_line_suffix() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("diff.rs");
        std::fs::write(&path, "fn main() {}\n").unwrap();

        let link = format!("{}:75", path.display());
        assert_eq!(
            markdown_file_link_target(&link, None),
            Some(MarkdownFileLinkTarget {
                path,
                line: Some(75)
            })
        );
    }

    #[test]
    fn markdown_file_link_strips_line_and_column_suffix() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("diff.rs");
        std::fs::write(&path, "fn main() {}\n").unwrap();

        let link = format!("{}:75:9", path.display());
        assert_eq!(
            markdown_file_link_target(&link, None),
            Some(MarkdownFileLinkTarget {
                path,
                line: Some(75)
            })
        );
    }

    #[test]
    fn markdown_file_link_keeps_colon_filename_when_it_exists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("diff.rs:75");
        std::fs::write(&path, "literal colon filename\n").unwrap();

        assert_eq!(
            markdown_file_link_target(path.to_str().unwrap(), None),
            Some(MarkdownFileLinkTarget { path, line: None })
        );
    }

    #[test]
    fn markdown_file_link_resolves_relative_path_against_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let subdir = dir.path().join("crates/app/src");
        std::fs::create_dir_all(&subdir).unwrap();
        let path = subdir.join("diff.rs");
        std::fs::write(&path, "fn main() {}\n").unwrap();

        assert_eq!(
            markdown_file_link_target("crates/app/src/diff.rs:75", Some(dir.path())),
            Some(MarkdownFileLinkTarget {
                path,
                line: Some(75)
            })
        );
    }

    #[test]
    fn markdown_file_link_decodes_file_url() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("with space.rs");
        std::fs::write(&path, "fn main() {}\n").unwrap();
        let encoded = path.to_string_lossy().replace(' ', "%20");

        assert_eq!(
            markdown_file_link_target(&format!("file://{encoded}:75"), None),
            Some(MarkdownFileLinkTarget {
                path,
                line: Some(75)
            })
        );
    }

    #[test]
    fn markdown_file_link_ignores_external_urls() {
        assert_eq!(
            markdown_file_link_target("https://example.com/a.rs:75", None),
            None
        );
        assert_eq!(
            markdown_file_link_target("mailto:a@example.com", None),
            None
        );
        assert_eq!(markdown_file_link_target("#local-heading", None), None);
    }
}
