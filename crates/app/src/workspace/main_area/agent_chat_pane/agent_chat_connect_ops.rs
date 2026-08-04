//! `Workspace` ops for the Agent chat pane's ACP connection lifecycle:
//! lazy-connect on first focus, manual retry from the Error banner, the
//! background connect + event pump itself, and the `/clear` full reset.
//! Split out of [`super::agent_chat_ops`] (which keeps notification, pane
//! construction, mode/config, and misc accessors) because the connect flow
//! is one large, self-contained concern with its own failure/retry paths.

use daruda_acp::{LaunchSpec, NodeProgress, connect_agent_session};
use daruda_config::{AccountEnv, AgentLaunch};
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::project::{LaneSessionHost, PaneCwd};
use futures::StreamExt as _;
use futures::channel::mpsc::unbounded;
use gpui::Context;
use std::path::{Path, PathBuf};

use super::agent_chat_ops::{agent_name_for, catalog_default_id, resolve_open_agent_id};
use super::view::{AgentSessionStatus, RuntimePrepPhase};
use crate::lane::Lane;
use crate::lane::session_host;
use crate::surface::strings as s;
use crate::workspace::Workspace;
use crate::workspace::main_area::pane::PreparedAccount;
use crate::workspace::main_area::pane_tree::PaneId;

/// Why [`resolve_session_command`] could not build a connect command —
/// distinct reasons because they need distinct messages (`connect_agent_chat`
/// maps each to its own status-line string; see that match).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConnectCommandError {
    /// The legacy `Raw` + `{{cwd}}` escape hatch has no usable remote path
    /// to substitute (`pane_cwd` isn't `Remote`, or its path is blank).
    NoRemotePath,
    /// `launch` is a JSON stdio config and the resolved host is remote —
    /// `session_host::wrap` can only fold a shell command line into
    /// `ssh`/`docker`, and raw JSON isn't one. See
    /// `AgentLaunch::is_json_stdio`'s doc.
    JsonStdioRemote,
}

/// Pure core of the `command`/cwd resolution inside
/// [`Workspace::connect_agent_chat`]. `owning_lane` is the pane's owning lane
/// (`None` when it could not be found — falls back to `Local`, never
/// promotes to remote; see the design's B′ note on restored remote panes).
///
/// Returns the resolved [`PaneCwd`] unconditionally (even on `Err`) — the
/// caller writes it back into the pane's own `cwd` so a pane whose lane's
/// host changed doesn't keep reporting the host it was created with (see
/// that call site's comment).
///
/// `pane_cwd` is consulted only for the legacy `Raw` + `{{cwd}}`-token escape
/// hatch (`AgentLaunch::CWD_TOKEN`), which predates the lane session-host
/// axis entirely and is untouched by it — see
/// [`session_host::adapter_command`]'s doc. Every other launch shape
/// resolves **both** the command and the cwd from the lane, not the
/// pane's own (possibly stale) persisted `cwd` — a restored remote pane
/// whose owning lane's host later changes follows the lane on its next
/// connect, not a frozen snapshot (the "의도된 행동 변화" the design calls
/// out for `edit_remote_cwd_hint`).
fn resolve_session_command(
    launch: &AgentLaunch,
    owning_lane: Option<&Lane>,
    pane_cwd: &PaneCwd,
    env: Option<&AccountEnv>,
    session_host_catalog: &[daruda_config::SessionHostEntry],
    session_host_tombstones: &[daruda_config::SessionHostTombstone],
) -> (Result<String, ConnectCommandError>, PaneCwd) {
    let is_raw_token = matches!(launch, AgentLaunch::Raw(command) if command.contains(daruda_config::agent::CWD_TOKEN));
    if is_raw_token {
        let (remote_path, resolved_cwd) = match pane_cwd {
            PaneCwd::Remote(remote_path) => (
                Some(remote_path.as_str()),
                PaneCwd::Remote(remote_path.clone()),
            ),
            PaneCwd::Local(path) => (None, PaneCwd::Local(path.clone())),
        };
        let wrapped = match env {
            Some(env) => launch.wrap_with_env(remote_path, env),
            None => launch.wrap(remote_path),
        }
        .map_err(|()| ConnectCommandError::NoRemotePath);
        return (wrapped, resolved_cwd);
    }

    let host = owning_lane
        .map(|lane| {
            lane.effective_session_host(launch, session_host_catalog, session_host_tombstones)
        })
        .unwrap_or(LaneSessionHost::Local);
    let resolved_cwd =
        match host.session_path() {
            Some(remote_path) => PaneCwd::Remote(remote_path.to_string()),
            // `Local`: the lane's own current path when it's known, else the
            // pane's own last-persisted local value (owning lane not found —
            // there is no better answer, see the module doc on that gate).
            None => PaneCwd::Local(owning_lane.map(|lane| lane.path.clone()).unwrap_or_else(
                || {
                    pane_cwd
                        .as_local()
                        .map(Path::to_path_buf)
                        .unwrap_or_default()
                },
            )),
        };
    // A JSON stdio config carries its command/args/env as structured
    // fields; `session_host::wrap` only knows how to splice a shell command
    // line into `ssh`/`docker`, so folding raw JSON in there would hand the
    // remote shell something it can't run. Local is unaffected — `wrap`
    // returns a JSON `Local` command unchanged, and `daruda_acp` parses it
    // exactly as it always has.
    if host.is_remote() && launch.is_json_stdio() {
        return (Err(ConnectCommandError::JsonStdioRemote), resolved_cwd);
    }
    let adapter = session_host::adapter_command(launch);
    let command = match env {
        Some(env) => session_host::wrap_with_env(&host, adapter, env),
        None => session_host::wrap(&host, adapter),
    };
    (Ok(command), resolved_cwd)
}

/// `host` (as [`Lane::effective_session_host`] resolved it) with its
/// `registry_id` corrected to the id the link currently resolves to —
/// [`session_host::resolved_registry_id`]'s value folded back onto the
/// host's own field. `effective_session_host` deliberately leaves a
/// tombstone-redirected id untouched (see its doc); this is the write-back
/// half `connect_agent_chat` persists so a future connect resolves directly
/// against the catalog instead of re-chasing the tombstone every time. A
/// no-op for `Local` or an unlinked/orphaned host.
fn sync_registry_id(
    host: LaneSessionHost,
    catalog: &[daruda_config::SessionHostEntry],
    tombstones: &[daruda_config::SessionHostTombstone],
) -> LaneSessionHost {
    let Some(fresh_id) = session_host::resolved_registry_id(&host, catalog, tombstones) else {
        return host;
    };
    match host {
        LaneSessionHost::Ssh {
            target,
            session_path,
            ..
        } => LaneSessionHost::Ssh {
            target,
            session_path,
            registry_id: Some(fresh_id),
        },
        LaneSessionHost::Docker {
            container,
            session_path,
            ..
        } => LaneSessionHost::Docker {
            container,
            session_path,
            registry_id: Some(fresh_id),
        },
        LaneSessionHost::Local => LaneSessionHost::Local,
    }
}

/// Pure core of `connect_agent_chat`'s session-host write-back: `Some(host)`
/// to persist onto the lane, `None` when there's nothing to sync.
///
/// `None` in two cases: `cached` is `None` — the lane never explicitly
/// answered the host question, so its resolution falls back to the legacy
/// `remote_cwd` pair, which must stay unanswered (see
/// `Lane::set_session_host`'s doc) rather than being silently promoted to an
/// explicit answer by this sync; or `resolved` is `None` — no owning lane
/// was found for this connect at all. Otherwise compares `cached` against
/// `resolved` corrected by [`sync_registry_id`] (folding in a tombstone
/// redirect's fresh id alongside the catalog's current `target`/`container`)
/// and returns the corrected value only when it actually differs.
fn session_host_write_back(
    cached: Option<&LaneSessionHost>,
    resolved: Option<&LaneSessionHost>,
    catalog: &[daruda_config::SessionHostEntry],
    tombstones: &[daruda_config::SessionHostTombstone],
) -> Option<LaneSessionHost> {
    let cached = cached?;
    let resolved = resolved?;
    let corrected = sync_registry_id(resolved.clone(), catalog, tombstones);
    (&corrected != cached).then_some(corrected)
}

/// The account recipe for `launch` on this connect, given `is_remote` (a
/// lane-verified locality this call site already has — see the doc on
/// `connect_agent_chat`'s `is_remote`). Handles the one case
/// [`AgentLaunch::account_recipe`] can't: a deprecated `Ssh`/`Docker` launch
/// this lane resolves to `Local` derives its recipe from `adapter_command`
/// directly, since `account_recipe` itself has no lane to verify locality
/// against and must stay conservative for its other three callers (see that
/// method's doc).
fn account_recipe_for_connect(
    launch: &AgentLaunch,
    is_remote: bool,
) -> Option<daruda_store::accounts::AccountRecipeId> {
    match launch {
        AgentLaunch::Ssh {
            adapter_command, ..
        }
        | AgentLaunch::Docker {
            adapter_command, ..
        } if !is_remote => daruda_config::account_recipe_for_local_command(adapter_command),
        _ => launch.account_recipe(is_remote),
    }
}

/// The `PathBuf` `connect_agent_session` needs for `cwd`, from a resolved
/// [`PaneCwd`]. For `Remote` this just re-wraps the opaque remote-path
/// string — the ACP wire protocol's `cwd` field is a plain path string
/// regardless of which machine it names, so this isn't a "real" local
/// `PathBuf`, only the shape the protocol call needs.
fn connect_wire_path(cwd: &PaneCwd) -> PathBuf {
    match cwd {
        PaneCwd::Local(path) => path.clone(),
        PaneCwd::Remote(remote_path) => PathBuf::from(remote_path),
    }
}

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

/// The session mode this agent's catalog entry asks for, when it sets one.
/// `None` for an agent that doesn't set one, or an id no longer in the catalog
/// — the global default then applies.
fn agent_default_mode<'a>(
    agents: &'a [daruda_config::AgentDefinition],
    agent_id: &str,
) -> Option<&'a str> {
    agents
        .iter()
        .find(|a| a.id == agent_id)?
        .default_mode
        .as_deref()
}

/// The ambient auth-override vars to unset for this pane's adapter, taken
/// from the resolved account's recipe. Empty for the System account, which
/// is defined by running under whatever the user's own environment says.
fn account_strip_env(prepared: Option<&PreparedAccount>) -> Vec<String> {
    prepared
        .map(|account| {
            account
                .env
                .strip
                .iter()
                .map(|name| (*name).to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// Abort a connect whose selected account's config dir could not be
/// prepared: park the pane in `Error` and toast. Never falls back to the
/// system account — the user picked this one, and connecting as another
/// silently is the failure this gate exists to prevent.
fn fail_connect_account_prepare(
    this: &gpui::WeakEntity<Workspace>,
    pane_id: PaneId,
    detail: String,
    cx: &mut gpui::AsyncApp,
) {
    let report = ErrorReport::new(s::agent_chat_account_prepare_failed())
        .message(detail)
        .severity(ErrorSeverity::Error)
        .at(file!(), line!())
        .dedup("agent_chat.account.prepare_failed")
        .build();
    let unreported = report.clone();
    match this.update(cx, |ws, cx| {
        if let Some(view) = ws.agent_chat_view(pane_id).cloned() {
            view.update(cx, |v, cx| {
                v.set_error(s::agent_chat_account_prepare_failed(), cx)
            });
            // Connecting → Error clears the badge; dirty the cached docks so
            // it doesn't linger stale.
            ws.notify_status_docks(cx);
        }
        ws.report_error(report, cx);
    }) {
        Ok(()) => {}
        // Window gone before the toast could land — keep the log record.
        Err(_) => daruda_store::observability::log_writer::LogWriter::log(unreported),
    }
}

impl Workspace {
    /// Lazy-connect entry point: start the ACP session for `pane_id` iff still
    /// parked in [`AgentSessionStatus::Idle`] with a cwd. Called from
    /// `focus_pane` so the session attaches on first focus and never twice.
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
            // Both `Local` and `Remote` panes are connectable; a cwd-less pane
            // was parked in `Error` at construction and never reaches `Idle`.
            let Some(cwd) = v.cwd.clone() else {
                return;
            };
            // A persisted session id resumes via `session/load`; `None` is fresh.
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
            // Idle → Connecting is a dock-badge status change; dirty the cached
            // docks explicitly (see `notify_status_docks`).
            self.notify_status_docks(cx);
        }
        self.connect_agent_chat(pane_id, cwd, resume, cx);
    }

    /// Manual retry for the Error banner: connect `pane_id` iff parked in
    /// [`AgentSessionStatus::Error`]. Like [`Self::maybe_connect_agent_chat`]
    /// but keeps `session_id` so it resumes via `session/load`.
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
            // Error → Connecting is a dock-badge status change; dirty the cached
            // docks explicitly (see `notify_status_docks`).
            self.notify_status_docks(cx);
        }
        self.connect_agent_chat(pane_id, cwd, resume, cx);
    }

    /// Resolve the launch spec for `pane_id`'s agent, reconciling the view when
    /// its `agent_id` is stale (a live config reload removed/renamed it) by
    /// rewriting to the session-sticky default. `None` only when the pane is gone.
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
        // launch the effective agent (a catalog entry, or the Claude id when the
        // catalog is somehow empty).
        let effective_id = resolve_open_agent_id(&self.agents, self.last_agent_id.as_deref());
        if let Some(view) = self.agent_chat_view(pane_id).cloned() {
            let id = effective_id.clone();
            let name = agent_name_for(&self.agents, &effective_id);
            view.update(cx, |v, _| {
                v.agent_id = id;
                v.agent_name = name;
            });
            self.mutate_durable(cx, |_, _| {});
        }
        Some(
            self.agent_launch_for(&effective_id)
                .unwrap_or_else(|| daruda_config::AgentDefinition::claude_default().launch),
        )
    }

    /// Open the live ACP session for an already-pushed pane and store the
    /// event-pump task on its view; closing the pane drops both. `resume`
    /// carries the persisted session id: `Some` branches `session/load`,
    /// `None` starts a fresh `session/new`. A failed resume retries once fresh.
    pub(in crate::workspace) fn connect_agent_chat(
        &mut self,
        pane_id: PaneId,
        cwd: PaneCwd,
        resume: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let node_root = daruda_store::persistence::node_install_dir();

        // Resolve the pane's agent_id → launch spec, reconciling a stale
        // agent_id. `None` only when the pane is gone — fall back to the catalog
        // default so the (soon-to-be-dropped) task still has a valid launch.
        let launch = self.resolve_pane_launch(pane_id, cx).unwrap_or_else(|| {
            self.agent_launch_for(&catalog_default_id(&self.agents))
                .unwrap_or_else(|| daruda_config::AgentDefinition::claude_default().launch)
        });
        // Read back after `resolve_pane_launch` so any stale-id reconcile it did
        // is picked up. Only keys the dev-build wire-tap file name — never
        // affects the launch itself.
        let agent_id = self
            .agent_chat_view(pane_id)
            .map(|v| v.read(cx).agent_id.clone())
            .unwrap_or_default();
        // Priority-ordered modes to try on a *fresh* session: this agent's own
        // `default_mode`, then the global default, then `auto`. Resolved after
        // the reconcile above so a pane whose agent_id was stale gets the mode
        // of the agent it actually launches. `run_connection` uses this only on
        // a fresh `session/new`; a real `session/load` uses `restore_mode`
        // below instead.
        let initial_modes = self
            .agent
            .connect_mode_priority(agent_default_mode(&self.agents, &agent_id));
        // The mode this pane's session was last known to be in — reapplied
        // after a resume (`session/load`) via `session/set_mode`.
        //
        // WORKAROUND: `session/load`'s response can in principle carry the
        // resumed session's real mode, but `claude-agent-acp` recomputes it
        // from `settings.json` on every process launch instead of the
        // session's actual last mode, so relying on that response alone loses
        // the mode across every app restart. Root cause is upstream
        // (`claude-agent-acp`'s `createSession`); the host tracks and
        // reapplies the mode itself until that's fixed there.
        let restore_mode = self
            .agent_chat_view(pane_id)
            .and_then(|v| v.read(cx).last_known_mode_id.clone());

        // Resolve the pane's owning lane so remote-ness is decided by the
        // lane's session host, not by whatever host (if any) `launch` itself
        // names — a restored `PaneCwd::Remote` pane whose agent has since
        // become host-agnostic must still attach to *this lane's* host
        // rather than silently falling back to local. This is the only place
        // a connect resolves the command/cwd pair to spawn, so a restored
        // remote pane (which skips `resolve_new_pane_cwd`) is fixed up here
        // on its lazy connect. See `resolve_session_command`'s doc.
        let owning_lane_ref = self.lane_ref_for_pane(pane_id);
        let owning_lane = owning_lane_ref.and_then(|lane_ref| self.lane_for(lane_ref));
        // Resolved once and reused below for `is_remote` and the registry
        // write-back — `effective_session_host` re-resolves `registry_id`
        // against the live catalog/tombstones on every call, so sharing one
        // result here avoids walking that twice for the same connect.
        let resolved_host = owning_lane.map(|lane| {
            lane.effective_session_host(&launch, &self.session_hosts, &self.session_host_tombstones)
        });
        // Cloned out now so it stays available after `owning_lane`'s borrow
        // of `self` ends (its last use is inside `resolve_session_command`
        // below, right before the write-back needs `&mut self`).
        let cached_host = owning_lane.and_then(|lane| lane.session_host.clone());
        // The pane's account must belong to the auth domain its own agent
        // launches under; an account from another domain is refused here
        // rather than injected under the wrong config-dir env var. Gated on
        // this connect's actual resolved host, not the launch's own shape —
        // a managed account's config dir is a local path, and injecting one
        // into a command that runs on another machine via `wrap_with_env`
        // would point the remote adapter at a directory that doesn't exist
        // there (see `account_recipe`'s doc).
        let is_remote = resolved_host
            .as_ref()
            .is_some_and(LaneSessionHost::is_remote);
        let selection = self.agent_chat_account_selection(pane_id);
        let domain = crate::workspace::main_area::pane::AccountDomain::for_agent(
            account_recipe_for_connect(&launch, is_remote),
        );
        let prepared = crate::workspace::main_area::pane::resolve_pane_account(
            &self.accounts,
            &self.data_dir,
            selection,
            domain,
        );
        if selection.account_id().is_some() && prepared.is_none() {
            // Not an error: the pane falls back to the system account. Log
            // only so a surprised user has ground truth for why.
            let report = ErrorReport::new(
                "Agent chat: pane account not usable for this agent; using system",
            )
            .severity(ErrorSeverity::Info)
            .with_context("agent_id", agent_id.clone())
            .at(file!(), line!())
            .dedup("agent_chat.account.unusable")
            .build();
            daruda_store::observability::log_writer::LogWriter::log(report);
        }
        let (wrapped, resolved_cwd) = resolve_session_command(
            &launch,
            owning_lane,
            &cwd,
            prepared.as_ref().map(|account| &account.env),
            &self.session_hosts,
            &self.session_host_tombstones,
        );
        // Sync the lane's cached session host with what this connect just
        // resolved — a registry `target`/`container` edit, or a tombstone
        // redirect landing on a new id, must persist onto the lane so a
        // future connect resolves it directly instead of re-deriving it
        // every time, and so anything reading `Lane::session_host` (e.g. a
        // future `SessionHostModal` display) sees the fresh value right
        // away rather than only after the next connect. Same idiom as the
        // `cwd_changed` sync below (codex review #3 on the prior Lane
        // session-host axis cycle), applied one layer up (Lane → Registry).
        if let Some(lane_ref) = owning_lane_ref
            && let Some(corrected) = session_host_write_back(
                cached_host.as_ref(),
                resolved_host.as_ref(),
                &self.session_hosts,
                &self.session_host_tombstones,
            )
        {
            self.set_lane_session_host(lane_ref, corrected, cx);
        }
        // Keep the pane's own cwd in step with what this connect actually
        // resolved. B′ (see `resolve_session_command`'s doc) means the live
        // host can diverge from what the pane was created or last connected
        // with; `AgentChatContent.cwd` is the cx-free cache `Pane::cwd()`,
        // the account-switcher, and persistence all read, and left unsynced
        // it would keep reporting a host this pane no longer attaches to.
        let cwd_changed = self
            .agent_chat_view(pane_id)
            .is_some_and(|view| view.read(cx).cwd.as_ref() != Some(&resolved_cwd));
        if cwd_changed {
            if let Some(view) = self.agent_chat_view(pane_id).cloned() {
                view.update(cx, |v, _| v.cwd = Some(resolved_cwd.clone()));
            }
            if let Some(content) = self
                .pane_mut(pane_id)
                .and_then(|p| p.agent_chat_content_mut())
            {
                content.cwd = Some(resolved_cwd.clone());
            }
            self.mutate_durable(cx, |_, _| {});
        }
        let connect_cwd = connect_wire_path(&resolved_cwd);
        // `wrap` fails only for the two `ConnectCommandError` reasons — see
        // that enum's doc. Never spawn a connection with a broken command:
        // park the pane in the matching error and bail out of this connect
        // attempt entirely.
        let command = match wrapped {
            Ok(command) => command,
            Err(err) => {
                let message = match err {
                    ConnectCommandError::NoRemotePath => s::agent_chat_no_remote_cwd(),
                    ConnectCommandError::JsonStdioRemote => {
                        s::agent_chat_json_stdio_remote_unsupported()
                    }
                };
                if let Some(view) = self.agent_chat_view(pane_id).cloned() {
                    view.update(cx, |v, cx| v.set_error(message, cx));
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

        // The strip list rides alongside the command all the way into launch
        // assembly; `wrap_with_env` above can only inject, since an
        // `/usr/bin/env` prefix this early would hide the `npx` launcher from
        // node detection.
        let launch_spec = LaunchSpec {
            command,
            strip_env: account_strip_env(prepared.as_ref()),
        };

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
            // Prep the managed account's config dir before anything spawns
            // (Claude mirrors shared MCP servers into it; Codex materializes
            // its home). The canonical sources can be multi-megabyte, so this
            // stays on the background executor. A failure aborts the connect:
            // the user picked this account, and silently continuing would run
            // the session as a different one.
            if let Some(account) = prepared {
                let prep = cx
                    .background_executor()
                    .spawn(async move {
                        daruda_agent::accounts::recipe_for(account.recipe)
                            .prepare_dir(&account.config_dir)
                            .map_err(|e| e.to_string())
                    })
                    .await;
                if let Err(detail) = prep {
                    fail_connect_account_prepare(&this, pane_id, detail, cx);
                    return;
                }
            }
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
                        launch_spec,
                        node_root,
                        connect_cwd,
                        initial_modes,
                        restore_mode,
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
                        });
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
                                        // Drop the stale handle too: until the
                                        // fresh connect reaches `Connected`, user
                                        // prompts must remain queued client-side
                                        // rather than entering the failed load's
                                        // closed command channel.
                                        v.handle = None;
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
                            // Also persisted (see `last_known_mode_id`'s doc) —
                            // reapplied on the next resume to work around
                            // `claude-agent-acp` not restoring it itself.
                            let mode_id_before = view.read(cx).last_known_mode_id.clone();
                            // Capture current mode before the event so we can
                            // detect `Connected` (modes arriving) and
                            // `ModeChanged` (current switching) and refresh the
                            // bottom-input placeholder when either fires.
                            let mode_before = view
                                .read(cx)
                                .session_config
                                .current_mode_id()
                                .map(str::to_string);
                            // Desktop notification for a permission wait, gated by
                            // focus. Must borrow `&event` before the move into
                            // `apply_event` below. Turn *completion* fires later,
                            // at the activity-settle edge (see the reconcile below).
                            ws.maybe_notify_agent_event(pane_id, &event, cx);
                            let telegram_first_response = view.update(cx, |v, cx| {
                                v.apply_event(event, &syntax_theme, is_light, cx)
                            });
                            ws.relay_telegram_first_response_effect(
                                pane_id,
                                telegram_first_response,
                                cx,
                            );
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
                                    || v.last_known_mode_id != mode_id_before
                                {
                                    ws.mutate_durable(cx, |_, _| {});
                                }
                            }
                            // Refresh placeholder when the active mode changed or
                            // modes became available (Connected). Only fires for
                            // the focused pane to avoid redundant work on parked
                            // lane views.
                            let mode_after = view
                                .read(cx)
                                .session_config
                                .current_mode_id()
                                .map(str::to_string);
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
                                let telegram_first_response = view.update(cx, |v, cx| {
                                    v.apply_event(
                                        daruda_acp::AcpEvent::Error(
                                            s::agent_chat_error_stream_ended(),
                                        ),
                                        &syntax_theme,
                                        is_light,
                                        cx,
                                    )
                                });
                                ws.relay_telegram_first_response_effect(
                                    pane_id,
                                    telegram_first_response,
                                    cx,
                                );
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

    /// Full local reset for `/clear`: wipe the conversation, tear down the ACP
    /// session, clear the persisted session id, start a fresh `session/new`.
    /// No-op when `pane_id` is gone or has no lane cwd.
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
        self.mutate_durable(cx, |_, _| {});
        self.notify_status_docks(cx);
        self.connect_agent_chat(pane_id, cwd, None, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ConnectCommandError, account_recipe_for_connect, account_strip_env, agent_default_mode,
        resolve_session_command, session_host_write_back, sync_registry_id,
    };
    use crate::lane::Lane;
    use daruda_config::{AccountEnv, AgentDefinition, AgentLaunch};
    use daruda_store::project::{LaneKind, LaneSessionHost, LaneStatus, PaneCwd};
    use std::path::PathBuf;

    /// A minimal lane at `path` for `resolve_session_command` tests — only
    /// `path` / `session_host` / `remote_cwd` matter to that function.
    fn lane_at(
        path: &str,
        session_host: Option<LaneSessionHost>,
        remote_cwd: Option<&str>,
    ) -> Lane {
        Lane {
            id: 1,
            kind: LaneKind::Default,
            path: PathBuf::from(path),
            name: None,
            tab_order: 0,
            is_unread: false,
            last_activity: 0,
            status: LaneStatus::Idle,
            is_main: false,
            base_ref: None,
            description: None,
            remote_cwd: remote_cwd.map(str::to_string),
            session_host,
            availability: crate::lane::availability::LaneAvailability::Present,
        }
    }

    fn raw(command: &str) -> AgentLaunch {
        AgentLaunch::Raw(command.to_string())
    }

    /// The rev2 design's B′ regression: a restored `PaneCwd::Remote` pane
    /// (a stale path with no host of its own) whose launch is a
    /// host-agnostic `Raw` command must still attach to its lane's `Ssh`
    /// host — not silently fall back to local because the launch no longer
    /// names a host itself.
    #[test]
    fn restored_remote_pane_follows_the_lanes_ssh_host_not_a_local_fallback() {
        let lane = lane_at(
            "/local/checkout",
            Some(LaneSessionHost::Ssh {
                target: "vm-work".into(),
                session_path: "/srv/app".into(),
                registry_id: None,
            }),
            None,
        );
        let stale_cwd = PaneCwd::Remote("stale-unrelated-path".into());
        let (command, connect_cwd) = resolve_session_command(
            &raw("npx acp-adapter"),
            Some(&lane),
            &stale_cwd,
            None,
            &[],
            &[],
        );
        assert_eq!(
            command,
            Ok("ssh vm-work sh -c 'cd \"/srv/app\" && npx acp-adapter'".to_string())
        );
        assert_eq!(
            connect_cwd,
            PaneCwd::Remote("/srv/app".to_string()),
            "connect cwd follows the lane's current session path, not the pane's stale one"
        );
    }

    /// The other half of B′: the lane answering `Local` (explicitly retiring
    /// a prior remote setup) must also override a stale `PaneCwd::Remote`.
    #[test]
    fn restored_remote_pane_follows_the_lane_back_to_local() {
        let lane = lane_at("/local/checkout", Some(LaneSessionHost::Local), None);
        let stale_cwd = PaneCwd::Remote("stale-unrelated-path".into());
        let (command, connect_cwd) = resolve_session_command(
            &raw("npx acp-adapter"),
            Some(&lane),
            &stale_cwd,
            None,
            &[],
            &[],
        );
        assert_eq!(command, Ok("npx acp-adapter".to_string()));
        assert_eq!(
            connect_cwd,
            PaneCwd::Local(PathBuf::from("/local/checkout"))
        );
    }

    /// Owning lane not found (pane outlived it) — falls back to local,
    /// never promoted to remote off a launch that names no host of its own.
    #[test]
    fn missing_owning_lane_falls_back_to_local_never_promotes_to_remote() {
        let local_cwd = PaneCwd::Local(PathBuf::from("/pane/own/local/path"));
        let (command, connect_cwd) =
            resolve_session_command(&raw("npx acp-adapter"), None, &local_cwd, None, &[], &[]);
        assert_eq!(command, Ok("npx acp-adapter".to_string()));
        assert_eq!(
            connect_cwd,
            PaneCwd::Local(PathBuf::from("/pane/own/local/path"))
        );
    }

    /// The legacy combo (lane never answered `session_host`, agent-side
    /// `Ssh`/`Docker` + `remote_cwd`) must keep assembling remotely, and the
    /// assembled string must stay byte-identical to what `AgentLaunch::wrap`
    /// alone produced before this axis moved to the lane.
    #[test]
    fn legacy_combo_still_assembles_remotely_byte_identical() {
        let lane = lane_at("/local/checkout", None, Some("/srv/legacy"));
        let launch = AgentLaunch::Ssh {
            adapter_command: "npx acp-adapter".into(),
            host: "old-box".into(),
        };
        let cwd = PaneCwd::Remote("/srv/legacy".into());
        let (command, connect_cwd) =
            resolve_session_command(&launch, Some(&lane), &cwd, None, &[], &[]);
        assert_eq!(
            command,
            launch
                .wrap(Some("/srv/legacy"))
                .map_err(|()| ConnectCommandError::NoRemotePath),
            "must match AgentLaunch::wrap's own assembly for the same inputs"
        );
        assert_eq!(connect_cwd, PaneCwd::Remote("/srv/legacy".to_string()));
    }

    /// The legacy `Raw` + `{{cwd}}` token escape hatch predates the lane
    /// session-host axis and must be untouched by it: even when the lane has
    /// its own `session_host` set to something else, the token path keeps
    /// resolving purely from the pane's own persisted `cwd`, exactly as
    /// `AgentLaunch::wrap` already did.
    #[test]
    fn raw_token_escape_hatch_ignores_the_lanes_session_host() {
        let lane = lane_at(
            "/local/checkout",
            Some(LaneSessionHost::Ssh {
                target: "other-host".into(),
                session_path: "/other/path".into(),
                registry_id: None,
            }),
            None,
        );
        let launch = raw("ssh vm-work \"cd {{cwd}} && run\"");
        let cwd = PaneCwd::Remote("/home/user/project".into());
        let (command, connect_cwd) =
            resolve_session_command(&launch, Some(&lane), &cwd, None, &[], &[]);
        assert_eq!(
            command,
            Ok("ssh vm-work \"cd /home/user/project && run\"".to_string())
        );
        assert_eq!(
            connect_cwd,
            PaneCwd::Remote("/home/user/project".to_string())
        );
    }

    fn json_stdio_launch() -> AgentLaunch {
        AgentLaunch::Raw(r#"{"command":"/opt/adapters/claude-agent-acp","args":[]}"#.to_string())
    }

    /// A JSON stdio config can't be folded into `ssh`/`docker` — `wrap` only
    /// knows how to splice a shell command line in there, and raw JSON isn't
    /// one. Must fail with the dedicated reason, not silently produce a
    /// broken command.
    #[test]
    fn json_stdio_launch_on_a_remote_lane_is_rejected() {
        let lane = lane_at(
            "/local/checkout",
            Some(LaneSessionHost::Ssh {
                target: "vm-work".into(),
                session_path: "/srv/app".into(),
                registry_id: None,
            }),
            None,
        );
        let cwd = PaneCwd::Local(PathBuf::from("/local/checkout"));
        let (command, resolved_cwd) =
            resolve_session_command(&json_stdio_launch(), Some(&lane), &cwd, None, &[], &[]);
        assert_eq!(command, Err(ConnectCommandError::JsonStdioRemote));
        // The resolved cwd is still reported (used to sync the pane even on
        // a rejected connect), even though the command itself failed.
        assert_eq!(resolved_cwd, PaneCwd::Remote("/srv/app".to_string()));
    }

    /// The same JSON stdio config on a lane that resolves `Local` is
    /// unaffected — `wrap` passes a `Local` command through unchanged, and
    /// `daruda_acp` parses it exactly as it always has.
    #[test]
    fn json_stdio_launch_on_a_local_lane_is_unaffected() {
        let lane = lane_at("/local/checkout", Some(LaneSessionHost::Local), None);
        let cwd = PaneCwd::Local(PathBuf::from("/local/checkout"));
        let (command, resolved_cwd) =
            resolve_session_command(&json_stdio_launch(), Some(&lane), &cwd, None, &[], &[]);
        assert_eq!(
            command,
            Ok(r#"{"command":"/opt/adapters/claude-agent-acp","args":[]}"#.to_string())
        );
        assert_eq!(
            resolved_cwd,
            PaneCwd::Local(PathBuf::from("/local/checkout"))
        );
    }

    /// Finding: a deprecated `Ssh`-typed launch that a lane now resolves to
    /// `Local` (the user picked Local in the Session Host dialog, retiring
    /// the launch's own embedded host) must still be eligible for a managed
    /// account, exactly like a `Raw` launch running the same command would.
    #[test]
    fn account_recipe_for_connect_recognizes_a_locally_resolved_ssh_launch() {
        let launch = AgentLaunch::Ssh {
            adapter_command: "npx -y @agentclientprotocol/claude-agent-acp@latest".into(),
            host: "old-box".into(),
        };
        assert_eq!(
            account_recipe_for_connect(&launch, false),
            Some(daruda_store::accounts::AccountRecipeId::Claude)
        );
        // Still excluded once it's actually remote.
        assert_eq!(account_recipe_for_connect(&launch, true), None);
    }

    /// Same for `Docker`.
    #[test]
    fn account_recipe_for_connect_recognizes_a_locally_resolved_docker_launch() {
        let launch = AgentLaunch::Docker {
            adapter_command: "npx -y @agentclientprotocol/codex-acp@latest".into(),
            container: "dev-1".into(),
        };
        assert_eq!(
            account_recipe_for_connect(&launch, false),
            Some(daruda_store::accounts::AccountRecipeId::Codex)
        );
    }

    /// A locally-resolved `Ssh`/`Docker` launch whose command is a JSON
    /// stdio config is still excluded — the shape that bars a managed
    /// account for `Raw` bars it here too.
    #[test]
    fn account_recipe_for_connect_still_excludes_json_stdio() {
        let launch = AgentLaunch::Ssh {
            adapter_command: r#"{"command":"x"}"#.into(),
            host: "old-box".into(),
        };
        assert_eq!(account_recipe_for_connect(&launch, false), None);
    }

    /// A plain `Raw` launch is unaffected by the new match arm — behaves
    /// exactly as `account_recipe` itself already does.
    #[test]
    fn account_recipe_for_connect_matches_account_recipe_for_raw_launches() {
        let launch = raw("npx -y @agentclientprotocol/claude-agent-acp@latest");
        assert_eq!(
            account_recipe_for_connect(&launch, false),
            launch.account_recipe(false)
        );
        assert_eq!(
            account_recipe_for_connect(&launch, true),
            launch.account_recipe(true)
        );
    }

    #[test]
    fn env_is_folded_in_for_the_lane_resolved_path() {
        let lane = lane_at(
            "/local/checkout",
            Some(LaneSessionHost::Ssh {
                target: "vm-work".into(),
                session_path: "/srv/app".into(),
                registry_id: None,
            }),
            None,
        );
        let env = AccountEnv {
            inject: vec![("CLAUDE_CONFIG_DIR".into(), "/remote/acc".into())],
            strip: vec!["ANTHROPIC_API_KEY"],
        };
        let cwd = PaneCwd::Local(PathBuf::from("/unused"));
        let (command, _) = resolve_session_command(
            &raw("npx acp-adapter"),
            Some(&lane),
            &cwd,
            Some(&env),
            &[],
            &[],
        );
        let command = command.unwrap();
        assert!(command.contains("export CLAUDE_CONFIG_DIR=\"/remote/acc\""));
        assert!(command.contains("unset ANTHROPIC_API_KEY"));
    }

    #[test]
    fn account_strip_env_carries_the_recipes_auth_overrides() {
        use crate::workspace::main_area::pane::PreparedAccount;
        use daruda_store::accounts::AccountRecipeId;
        use std::path::PathBuf;

        let recipe = daruda_agent::accounts::recipe_for(AccountRecipeId::Claude);
        let config_dir = PathBuf::from("/data/acc/alice");
        let prepared = PreparedAccount {
            recipe: AccountRecipeId::Claude,
            env: daruda_config::account_env(
                recipe.config_dir_env(),
                &config_dir,
                recipe.strip_env(),
            ),
            config_dir,
        };
        let strip = account_strip_env(Some(&prepared));
        assert_eq!(strip, recipe.strip_env());
        assert!(strip.iter().any(|n| n == "ANTHROPIC_API_KEY"));

        // Codex strips nothing, and the System account (no managed account)
        // runs under the user's own environment by definition.
        let codex = daruda_agent::accounts::recipe_for(AccountRecipeId::Codex);
        assert!(codex.strip_env().is_empty());
        assert!(account_strip_env(None).is_empty());
    }

    #[test]
    fn agent_default_mode_reads_the_matching_catalog_entry() {
        let agents = vec![
            AgentDefinition {
                id: "other".to_string(),
                name: "Other".to_string(),
                launch: AgentLaunch::Raw("run-other".to_string()),
                default_mode: Some("yolo".to_string()),
            },
            AgentDefinition::claude_default(),
        ];
        assert_eq!(agent_default_mode(&agents, "other"), Some("yolo"));
        assert_eq!(
            agent_default_mode(&agents, &AgentDefinition::claude_default().id),
            None,
            "an entry without an override leaves the global default to apply"
        );
        assert_eq!(
            agent_default_mode(&agents, "gone"),
            None,
            "an id no longer in the catalog is not an override"
        );
    }

    // Task 3: connect-path write-back — a registry `target`/`container` edit,
    // or a tombstone redirect, syncs back onto the lane's cached
    // `session_host` so a future connect (and any UI reading it) sees the
    // fresh value instead of re-deriving it every time.

    fn write_back_tombstone(
        old_id: daruda_store::project::SessionHostId,
        redirected_to: Option<daruda_store::project::SessionHostId>,
    ) -> daruda_config::SessionHostTombstone {
        daruda_config::SessionHostTombstone {
            old_id,
            kind: daruda_config::SessionHostKind::Ssh {
                target: "old-box".into(),
            },
            value: "old-box".into(),
            removed_at: 0,
            redirected_to,
        }
    }

    /// The Step 1 scenario: a catalog `target` edit surfaces through
    /// `effective_session_host` already (Task 2) — this asserts the
    /// write-back actually persists it onto the lane's cache.
    #[test]
    fn catalog_target_change_writes_back_the_lanes_cached_target() {
        let id = daruda_store::project::SessionHostId::new();
        let cached = LaneSessionHost::Ssh {
            target: "old-target".into(),
            session_path: "/srv/app".into(),
            registry_id: Some(id),
        };
        let catalog = vec![daruda_config::SessionHostEntry {
            id,
            label: "Build box".into(),
            kind: daruda_config::SessionHostKind::Ssh {
                target: "new-target".into(),
            },
        }];
        // Task 2's resolver already folded the catalog's current target
        // in — this is what `connect_agent_chat` would pass as `resolved`.
        let resolved = LaneSessionHost::Ssh {
            target: "new-target".into(),
            session_path: "/srv/app".into(),
            registry_id: Some(id),
        };
        let written = session_host_write_back(Some(&cached), Some(&resolved), &catalog, &[])
            .expect("target changed — must write back");
        assert_eq!(
            written,
            LaneSessionHost::Ssh {
                target: "new-target".into(),
                session_path: "/srv/app".into(),
                registry_id: Some(id),
            }
        );
    }

    /// A tombstone redirect also corrects the cached `registry_id` to the
    /// live id — not just `target` — so the next connect resolves directly
    /// against the catalog instead of re-chasing the tombstone.
    #[test]
    fn tombstone_redirect_writes_back_both_target_and_registry_id() {
        let old_id = daruda_store::project::SessionHostId::new();
        let new_id = daruda_store::project::SessionHostId::new();
        let cached = LaneSessionHost::Ssh {
            target: "old-box".into(),
            session_path: "/srv/app".into(),
            registry_id: Some(old_id),
        };
        let catalog = vec![daruda_config::SessionHostEntry {
            id: new_id,
            label: "Renamed box".into(),
            kind: daruda_config::SessionHostKind::Ssh {
                target: "merged-target".into(),
            },
        }];
        let tombstones = vec![write_back_tombstone(old_id, Some(new_id))];
        // `effective_session_host` (Task 2) resolves the target through the
        // redirect but leaves `registry_id` at the stale cached id — exactly
        // what `connect_agent_chat` would pass here.
        let resolved = LaneSessionHost::Ssh {
            target: "merged-target".into(),
            session_path: "/srv/app".into(),
            registry_id: Some(old_id),
        };
        let written =
            session_host_write_back(Some(&cached), Some(&resolved), &catalog, &tombstones)
                .expect("redirect resolved a new value — must write back");
        assert_eq!(
            written,
            LaneSessionHost::Ssh {
                target: "merged-target".into(),
                session_path: "/srv/app".into(),
                registry_id: Some(new_id),
            }
        );
    }

    /// No drift → no write, so a connect that changes nothing doesn't
    /// spuriously dirty + persist the workspace on every single connect.
    #[test]
    fn unchanged_value_writes_back_nothing() {
        let id = daruda_store::project::SessionHostId::new();
        let host = LaneSessionHost::Ssh {
            target: "vm-work".into(),
            session_path: "/srv/app".into(),
            registry_id: Some(id),
        };
        let catalog = vec![daruda_config::SessionHostEntry {
            id,
            label: "Build box".into(),
            kind: daruda_config::SessionHostKind::Ssh {
                target: "vm-work".into(),
            },
        }];
        assert_eq!(
            session_host_write_back(Some(&host), Some(&host), &catalog, &[]),
            None
        );
    }

    /// An unanswered lane (`session_host: None`) must never be silently
    /// promoted to an explicit answer by this sync — that's a distinct,
    /// user-driven action (`Lane::set_session_host`'s doc), not something a
    /// background resolve should do on its own.
    #[test]
    fn unanswered_lane_is_never_written_to() {
        let resolved = LaneSessionHost::Ssh {
            target: "vm-work".into(),
            session_path: "/srv/app".into(),
            registry_id: None,
        };
        assert_eq!(
            session_host_write_back(None, Some(&resolved), &[], &[]),
            None
        );
    }

    /// No owning lane at all (pane with no resolvable lane) — nothing to
    /// write back to.
    #[test]
    fn missing_owning_lane_writes_back_nothing() {
        let cached = LaneSessionHost::Local;
        assert_eq!(session_host_write_back(Some(&cached), None, &[], &[]), None);
    }

    /// `sync_registry_id` itself: a `Local` host has no registry link to
    /// correct — passed through unchanged.
    #[test]
    fn sync_registry_id_is_a_noop_for_local() {
        assert_eq!(
            sync_registry_id(LaneSessionHost::Local, &[], &[]),
            LaneSessionHost::Local
        );
    }

    /// An orphaned link (deleted, no redirect) keeps its stale id — there is
    /// nothing live to correct it to.
    #[test]
    fn sync_registry_id_keeps_an_orphaned_id() {
        let id = daruda_store::project::SessionHostId::new();
        let host = LaneSessionHost::Docker {
            container: "dev-1".into(),
            session_path: "/workspace".into(),
            registry_id: Some(id),
        };
        assert_eq!(sync_registry_id(host.clone(), &[], &[]), host);
    }
}
