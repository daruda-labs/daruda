//! Agent-chat pane construction: agent and cwd resolution, building the pane,
//! and inserting or revealing it in a tab.

use super::{
    PaneCwdBlocked, PaneCwdOutcome, agent_identity, agent_name_for, resolve_new_pane_cwd_core,
    resolve_open_agent_id, resolve_restored_agent,
};
use crate::agent::launch_resolve::{AgentLaunchSpec, account_recipe_for_connect};
use crate::surface::strings as s;
use crate::workspace::Workspace;
use crate::workspace::main_area::pane::{AgentChatContent, Pane, PaneContent, TabEntry};
use crate::workspace::main_area::pane_tree::{PaneId, PaneLayout};
use daruda_chat::transcript_defaults::TranscriptDefaults;
use daruda_chat::view::{AgentChatView, AgentSessionStatus};
use daruda_config::AgentLaunch;
use daruda_store::project::{LaneSessionHost, PaneCwd};
use gpui::{AppContext as _, Context, Entity, Window};
use std::path::PathBuf;

impl Workspace {
    /// The agent a newly opened pane runs under: `requested` if given, else
    /// the window's last-used agent — whichever it is, only while it is still
    /// in the catalog, else the catalog default.
    pub(in crate::workspace) fn open_agent_id(&self, requested: Option<&str>) -> String {
        resolve_open_agent_id(
            &self.mirrors.agents,
            requested.or(self.last_agent_id.as_deref()),
        )
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
            // Neither of these has an in-app fix: the lane has no working
            // directory, or the remote path is blocked by configuration.
            PaneCwdOutcome::Ready(None) => (
                None,
                AgentSessionStatus::Error {
                    message: s::agent_chat::no_lane_cwd(),
                    remedy: daruda_acp::Remedy::NoneAvailable,
                },
            ),
            PaneCwdOutcome::Blocked(reason) => (
                None,
                AgentSessionStatus::Error {
                    message: reason,
                    remedy: daruda_acp::Remedy::NoneAvailable,
                },
            ),
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
                .and_then(|l| account_recipe_for_connect(&l.launch, is_remote)),
        );
        // `title` seeds the view's `session_title` below (restored dormant
        // panes show their persisted label before the session loads);
        // `Pane::title()` reads it live, so there's no separate cache to seed
        // here.
        let agent_name = agent_name_for(&self.mirrors.agents, &agent_id);
        // The view owns its own `cwd` (for connect / persistence); the wrapper
        // caches a copy so `Pane::cwd()` stays cx-free.
        // Seed from the resolved config; a restore overwrites it with the pane's
        // own persisted choice (see `rebuild_layout`), and a live config reload
        // re-applies these through `reseed_transcript_defaults`.
        let defaults = TranscriptDefaults::resolve(
            self.mirrors.agents.iter().find(|a| a.id == agent_id),
            self.mirrors.agent_reader_defaults,
        );
        // Seeded here so a pane can build diff embeds before its first event.
        let syntax_theme = self.mirrors.syntax_theme.clone();
        let view = cx.new({
            let cwd = cwd.clone();
            let agent_id = agent_id.clone();
            let agent_name = agent_name.clone();
            move |cx| {
                AgentChatView::new(
                    pane_id,
                    window_handle,
                    cwd,
                    status,
                    session_id,
                    agent_identity(agent_id, agent_name),
                    title,
                    defaults,
                    syntax_theme,
                    super::super::agent_default_writer::agent_default_writer(),
                    cx,
                )
            }
        });
        self.subscribe_agent_chat(pane_id, &view, window, cx);
        let preferred_editor = self.mirrors.preferred_editor.clone();
        view.update(cx, |view, _| view.set_preferred_editor(&preferred_editor));
        Pane {
            id: pane_id,
            content: PaneContent::AgentChat(AgentChatContent {
                view,
                cwd,
                account,
                agent_id,
            }),
        }
    }

    /// Resolve `Local` vs. `Remote` cwd for a fresh pane under `agent_id`,
    /// via [`resolve_new_pane_cwd_core`]. An id no longer in the catalog falls
    /// back to an empty `AgentLaunch::Raw` (uses `local_cwd`).
    pub(in crate::workspace::main_area::agent_chat_host) fn resolve_new_pane_cwd(
        &self,
        agent_id: &str,
        local_cwd: Option<PathBuf>,
        remote_cwd: Option<String>,
        session_host: Option<&LaneSessionHost>,
    ) -> Result<Option<PaneCwd>, PaneCwdBlocked> {
        let launch = self
            .agent_launch_for(agent_id)
            .map(|spec| spec.launch)
            .unwrap_or_else(|| AgentLaunch::Raw(String::new()));
        resolve_new_pane_cwd_core(
            &launch,
            local_cwd,
            remote_cwd,
            session_host,
            &self.mirrors.session_hosts,
            &self.mirrors.session_host_tombstones,
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
            Err(PaneCwdBlocked::NoRemotePath) => {
                PaneCwdOutcome::Blocked(s::agent_chat::no_remote_cwd())
            }
            Err(PaneCwdBlocked::UnusableSessionHost(reason)) => PaneCwdOutcome::Blocked(
                s::agent_chat::session_host_unusable(daruda_settings::labels::host_error(reason)),
            ),
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

    /// How `agent_id` launches — adapter command plus the environment its
    /// entry declares — looked up in the catalog. `None` when the id is not in
    /// the catalog (e.g. a persisted id whose agent was removed).
    pub(in crate::workspace) fn agent_launch_for(&self, agent_id: &str) -> Option<AgentLaunchSpec> {
        self.mirrors
            .agents
            .iter()
            .find(|a| a.id == agent_id)
            .map(AgentLaunchSpec::of)
    }

    /// Resolve the agent a restored pane launches under, and whether its
    /// persisted session id is still resumable. If the owning agent was
    /// removed from the catalog, fall back to the default and drop the id.
    pub(in crate::workspace) fn resolve_restored_agent(
        &self,
        persisted_agent_id: Option<String>,
    ) -> (String, bool /* keep session id */) {
        resolve_restored_agent(&self.mirrors.agents, persisted_agent_id)
    }

    pub(in crate::workspace) fn open_agent_chat_pane(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Default to the last agent opened (session-local), falling back to the
        // catalog default; a stale last id (agent removed) also falls back.
        let agent_id = self.open_agent_id(None);
        self.open_agent_chat_pane_with_agent(agent_id, window, cx);
    }

    /// Open a fresh pane under `agent_id` at the active lane's cwd, and
    /// record `agent_id` as the session's last choice.
    pub(in crate::workspace) fn open_agent_chat_pane_with_agent(
        &mut self,
        agent_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let cwds = self.active_lane_cwds();
        if let Some(pane_id) = self.insert_agent_chat_pane(agent_id, cwds, window, cx) {
            self.reveal_new_agent_chat_pane(pane_id, window, cx);
        }
    }

    /// Create the pane and register it in the active runtime's panes and tabs,
    /// stopping short of focusing it. Split from [`Self::reveal_new_agent_chat_pane`]
    /// because focusing is what triggers lazy connect, so anything that has to be
    /// true before a session could start goes between the two — that gap is the
    /// seam `open_agent_chat_pane_seeded` seeds through.
    ///
    /// `cwds` is the `(local, remote, session_host)` triple the pane roots at,
    /// passed in rather than read from the active lane: the orchestrator's
    /// window has no lane and roots its pane at a directory of its own, so it
    /// reuses this sequence instead of copying it.
    ///
    /// `None` when the active lane cannot host a pane.
    pub(in crate::workspace) fn insert_agent_chat_pane(
        &mut self,
        agent_id: String,
        cwds: (Option<PathBuf>, Option<String>, Option<LaneSessionHost>),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<PaneId> {
        // An inaccessible active lane renders the empty-state; opening a pane
        // there would escape that state (mirrors `add_tab`). Guard first, before
        // recording the choice, so a rejected open does not mutate last_agent_id.
        if self.active_lane_is_inaccessible() {
            return None;
        }
        self.last_agent_id = Some(agent_id.clone());
        let (local_cwd, remote_cwd, session_host) = cwds;
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
        if !self.docks.bottom.read(cx).is_open {
            self.docks.bottom.update(cx, |d, cx| {
                d.toggle();
                cx.notify();
            });
            self.main_area.pending_resize = true;
        }
        Some(pane_id)
    }

    /// Register a wrapper around an existing session without creating or connecting a view.
    pub(in crate::workspace) fn wrap_existing_agent_chat_pane(
        &mut self,
        pane_id: PaneId,
        view: Entity<AgentChatView>,
        cwd: Option<PaneCwd>,
        account: daruda_store::accounts::AccountSelection,
        agent_id: String,
    ) {
        let tab_id = self.alloc_id();
        let rt = self.active_runtime_mut();
        if !rt.tabs.is_empty() {
            rt.tab_history.push(rt.active_tab_index);
        }
        rt.panes.push(Pane {
            id: pane_id,
            content: PaneContent::AgentChat(AgentChatContent {
                view,
                cwd,
                account,
                agent_id,
            }),
        });
        rt.tabs.push(TabEntry {
            id: tab_id,
            layout: PaneLayout::Pane(pane_id),
            last_focused_pane: pane_id,
            user_label: None,
        });
        rt.active_tab_index = rt.tabs.len() - 1;
    }

    /// Focus a freshly inserted Agent chat pane and settle the layout around
    /// it. Focusing runs `maybe_connect_agent_chat`, so this is always the last
    /// step of an open.
    pub(in crate::workspace) fn reveal_new_agent_chat_pane(
        &mut self,
        pane_id: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_focused_pane(pane_id, window, cx);
        self.bump_activity(pane_id);
        self.focus_pane(pane_id, window, cx);
        self.resize_all_tabs(window, cx);
        cx.notify();
    }

    /// Open a pane whose transcript comes from `seed` rather than from a
    /// session — the single seeding entry point for `--screenshot` scenarios
    /// and `--replay-acp-log` alike.
    ///
    /// `seed` runs between insert and reveal, which is the whole point of the
    /// split: it parks the pane out of `Idle` before focus would otherwise send
    /// `maybe_connect_agent_chat` off to spawn a real adapter behind it.
    #[cfg(feature = "devtools")]
    pub(in crate::workspace) fn open_agent_chat_pane_seeded(
        &mut self,
        agent_id: Option<&str>,
        seed: impl FnOnce(&mut AgentChatView, &mut Window, &mut Context<AgentChatView>),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<PaneId> {
        // The pane's agent decides its chrome — name, tool-group label, usage
        // panel — so a caller replaying a capture wants the agent the capture
        // came from, not whatever was last opened. A requested id the catalog
        // does not have falls back exactly as a stale `last_agent_id` would.
        let agent_id = self.open_agent_id(agent_id);
        let cwds = self.active_lane_cwds();
        let pane_id = self.insert_agent_chat_pane(agent_id, cwds, window, cx)?;
        let view = self.agent_chat_view(pane_id).cloned()?;
        // The seed builds embed entities, which re-enter the window.
        view.update(cx, |v, cx| seed(v, window, cx));
        self.reveal_new_agent_chat_pane(pane_id, window, cx);
        Some(pane_id)
    }

    /// Open a pane holding `items` with no session behind it. The crate-root
    /// entry point (`--replay-acp-log`) reaches seeding through this, so the
    /// closure seam above stays inside `workspace`. `false` when there was no
    /// accessible lane to open a pane in.
    ///
    /// Gated on `replay` rather than `devtools`: its only caller is the
    /// crate-root replay entry point, so a screenshot-only build would see it
    /// as dead code.
    #[cfg(feature = "replay")]
    pub(crate) fn open_agent_chat_pane_with_transcript(
        &mut self,
        agent_id: Option<&str>,
        items: Vec<daruda_acp::ChatItem>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.open_agent_chat_pane_seeded(
            agent_id,
            |v, window, cx| v.seed_transcript(items, window, cx),
            window,
            cx,
        )
        .is_some()
    }
}
