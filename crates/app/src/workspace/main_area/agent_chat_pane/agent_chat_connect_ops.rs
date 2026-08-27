//! `Workspace` ops for the Agent chat pane's ACP connection lifecycle:
//! lazy-connect on first focus, manual retry from the Error banner, the
//! background connect + event pump itself, and the `/clear` full reset.
//! Split out of [`super::agent_chat_ops`] (which keeps notification, pane
//! construction, mode/config, and misc accessors) because the connect flow
//! is one large, self-contained concern with its own failure/retry paths.

use daruda_acp::{NodeProgress, connect_agent_session_with_model};
use daruda_config::AgentLaunch;
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::project::{LaneSessionHost, PaneCwd};
use futures::StreamExt as _;
use futures::channel::mpsc::unbounded;
use gpui::Context;

use super::agent_chat_ops::{agent_name_for, catalog_default_id, resolve_open_agent_id};
use super::view::{AgentSessionStatus, RuntimePrepPhase};
use crate::agent::launch_resolve::{
    ConnectCommandError, account_recipe_for_connect, resolve_launch,
};
use crate::surface::strings as s;
use crate::workspace::Workspace;
use crate::workspace::main_area::pane_tree::PaneId;

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

/// The model this agent's catalog entry asks its panes to start on. `None` for
/// an agent that pins no model, or an id no longer in the catalog — the
/// adapter's own choice then stands.
fn agent_default_model<'a>(
    agents: &'a [daruda_config::AgentDefinition],
    agent_id: &str,
) -> Option<&'a str> {
    agents
        .iter()
        .find(|a| a.id == agent_id)?
        .default_model
        .as_deref()
}

/// The model requested during the ACP handshake. A pane's explicit pick wins
/// over the catalog default; availability is checked by `daruda_acp` against
/// the live agent advertisement before any mode or queued prompt is applied.
fn connect_model_preference(
    remembered: Option<&str>,
    agent_default: Option<&str>,
) -> Option<String> {
    remembered
        .or(agent_default)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
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
                v.set_error(
                    s::agent_chat_account_prepare_failed(),
                    daruda_acp::Remedy::Retry,
                    cx,
                )
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
            if !matches!(v.status, AgentSessionStatus::Error { .. }) {
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

    /// Surface a session's non-fatal advisory through the app's error pipeline
    /// (toast → details modal → log) instead of the log alone.
    ///
    /// Every advisory reports the same shape of thing: something the user or
    /// their config asked for did not happen, and the session is fine anyway —
    /// a resume that could not replay history, a configured mode the agent
    /// refused, a config option it would not set. Each is the answer to an
    /// action the user just took, so it is neither noise nor something they
    /// can be expected to find in a log file.
    ///
    /// Reporting here rather than in the view keeps the one-way flow intact
    /// (`Workspace` owns error reporting) and covers every advisory: the other
    /// `apply_event` call site only ever feeds a synthetic terminal error.
    pub(in crate::workspace) fn report_agent_notice(
        &mut self,
        pane_id: PaneId,
        event: &daruda_acp::AcpEvent,
        cx: &mut Context<Self>,
    ) {
        let daruda_acp::AcpEvent::Notice(message) = event else {
            return;
        };
        // The body is the adapter's own diagnostic, not authored copy — the
        // same treatment a captured login failure gets.
        self.report_error(
            ErrorReport::new(s::agent_chat_session_notice())
                .message(message.clone())
                .severity(ErrorSeverity::Warning)
                .at(file!(), line!())
                .dedup(notice_dedup_key(pane_id, message))
                .build(),
            cx,
        );
    }

    /// Reconnect every pane a successful login into `target` has actually
    /// unblocked — the tail of both reauthenticate flows.
    ///
    /// Without this a user who signs in from a failure banner is left looking
    /// at the same error, with a second button to press for the thing they
    /// just asked for. Scoped by [`login_revives_pane`] rather than applied to
    /// every failed pane: a login fixes only the panes that run on the
    /// credentials it wrote.
    ///
    /// Only reaches panes whose *connection* failed, which is deliberately the
    /// narrower half. An expired login usually surfaces as a **turn** failure
    /// instead — the agent creates the session without checking credentials
    /// and refuses at the first real request — and such a pane is still
    /// `Connected`, so it is left alone here: whether a running adapter picks
    /// up freshly written credentials without a reconnect is not something
    /// this has established, and tearing down a live session to find out would
    /// cost the user their conversation on a guess. That pane keeps its own
    /// sign-in button and its next prompt shows whether the login took.
    pub(in crate::workspace) fn reconnect_panes_after_login(
        &mut self,
        target: crate::workspace::account_login_ops::LoginTarget,
        cx: &mut Context<Self>,
    ) {
        // Collected before any reconnect runs: `retry_agent_chat_connect`
        // leases each view, and holding a read across that would re-enter the
        // same entity (CLAUDE.md Pitfall #5).
        let revived: Vec<PaneId> = self
            .main_area
            .runtimes
            .values()
            .flat_map(|rt| rt.panes.iter())
            .map(|pane| (pane.id, pane.account_selection()))
            .collect::<Vec<_>>()
            .into_iter()
            .filter(|&(pane_id, selection)| {
                let pane_target = selection.and_then(|selection| {
                    let domain = crate::workspace::main_area::pane::AccountDomain::for_pane(
                        &self.account_pane_for(pane_id, cx),
                    );
                    crate::workspace::account_login_ops::pane_login_target(
                        selection,
                        domain,
                        &self.accounts,
                    )
                });
                let remedy =
                    self.agent_chat_view(pane_id)
                        .and_then(|view| match &view.read(cx).status {
                            AgentSessionStatus::Error { remedy, .. } => Some(*remedy),
                            _ => None,
                        });
                login_revives_pane(target, pane_target, remedy)
            })
            .map(|(pane_id, _)| pane_id)
            .collect();

        for pane_id in revived {
            self.retry_agent_chat_connect(pane_id, cx);
        }
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
        let remembered_model = self
            .agent_chat_view(pane_id)
            .and_then(|v| v.read(cx).last_known_model_id.clone());
        let initial_model = connect_model_preference(
            remembered_model.as_deref(),
            agent_default_model(&self.agents, &agent_id),
        );
        let vocabulary_source = crate::lane::session_host::adapter_command(&launch)
            .trim()
            .to_string();
        if let Some(view) = self.agent_chat_view(pane_id).cloned() {
            view.update(cx, |view, _| {
                view.agent_vocabulary_source = Some(vocabulary_source);
            });
        }
        // Priority-ordered modes to try on a *fresh* session: this agent's own
        // `default_mode`, when its catalog entry sets one — otherwise empty,
        // so the adapter's own default mode applies. Resolved after the
        // reconcile above so a pane whose agent_id was stale gets the mode of
        // the agent it actually launches. `run_connection` uses this only on
        // a fresh `session/new`; a real `session/load` uses `restore_mode`
        // below instead.
        let initial_modes = daruda_config::agent::connect_mode_priority(agent_default_mode(
            &self.agents,
            &agent_id,
        ));
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
        // Resolved once and reused below for `resolve_launch`, `is_remote`,
        // and the registry write-back — `effective_session_host` re-resolves
        // `registry_id` against the live catalog/tombstones on every call, so
        // sharing one result here avoids walking that chase more than once
        // for the same connect.
        let resolved_host = owning_lane.map(|lane| {
            lane.effective_session_host(&launch, &self.session_hosts, &self.session_host_tombstones)
        });
        // Cloned out now so it stays available after `owning_lane`'s borrow
        // of `self` ends (its last use is inside `resolve_launch` below,
        // right before the write-back needs `&mut self`).
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
        let resolved = resolve_launch(
            &launch,
            owning_lane,
            &cwd,
            prepared.as_ref(),
            cached_host.as_ref(),
            resolved_host.as_ref(),
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
            && let Some(corrected) = resolved.host_write_back
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
            .is_some_and(|view| view.read(cx).cwd.as_ref() != Some(&resolved.resolved_cwd));
        if cwd_changed {
            if let Some(view) = self.agent_chat_view(pane_id).cloned() {
                view.update(cx, |v, _| v.cwd = Some(resolved.resolved_cwd.clone()));
            }
            if let Some(content) = self
                .pane_mut(pane_id)
                .and_then(|p| p.agent_chat_content_mut())
            {
                content.cwd = Some(resolved.resolved_cwd.clone());
            }
            self.mutate_durable(cx, |_, _| {});
        }
        let connect_cwd = resolved.wire_cwd;
        // `wrap` fails only for the two `ConnectCommandError` reasons — see
        // that enum's doc. Never spawn a connection with a broken command:
        // park the pane in the matching error and bail out of this connect
        // attempt entirely.
        let launch_spec = match resolved.spec {
            Ok(spec) => spec,
            Err(err) => {
                let message = match err {
                    ConnectCommandError::NoRemotePath => s::agent_chat_no_remote_cwd(),
                    ConnectCommandError::JsonStdioRemote => {
                        s::agent_chat_json_stdio_remote_unsupported()
                    }
                };
                if let Some(view) = self.agent_chat_view(pane_id).cloned() {
                    view.update(cx, |v, cx| {
                        v.set_error(message, daruda_acp::Remedy::Configure, cx);
                    });
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
            .with_context("command", launch_spec.command.clone())
            .with_context("PATH", path)
            .at(file!(), line!())
            .dedup("agent_chat.connect.command")
            .build();
            daruda_store::observability::log_writer::LogWriter::log(report);
        }

        // Runtime provisioning (see `connect_agent_session_with_model`) can download
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
            // `connect_agent_session_with_model` is synchronous (it provisions node,
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
                    connect_agent_session_with_model(
                        launch_spec,
                        node_root,
                        connect_cwd,
                        initial_model,
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
                            && let daruda_acp::AcpEvent::Error(failure) = &event
                        {
                            let detail = failure.message().to_owned();
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
                            ws.report_agent_notice(pane_id, &event, cx);
                            // Refresh the persisted option vocabularies from
                            // what this agent just advertised. Also borrows
                            // `&event` before the move below.
                            ws.record_agent_vocabulary(pane_id, &event, cx);
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
                                            // Locally detected, not a protocol
                                            // error — there is no code or
                                            // `errorKind` behind it to classify.
                                            daruda_acp::AcpFailure::unclassified(
                                                s::agent_chat_error_stream_ended(),
                                            ),
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
                        let report = ErrorReport::new(
                            crate::surface::strings::error_acp_resume_failed_retrying(),
                        )
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
                    // Connect-time failures classify like any other: a Node
                    // runtime that would not provision is retryable, an
                    // expired login is not.
                    let failure = err.into_failure();
                    let remedy = failure.remedy();
                    let message = failure.message().to_owned();
                    // workspace gone before the connect resolved — nothing left
                    // to surface the failure on.
                    // SILENT-OK: workspace/window dropped before connect resolved
                    let _ = this.update(cx, |ws, cx| {
                        if let Some(view) = ws.agent_chat_view(pane_id).cloned() {
                            view.update(cx, |v, cx| {
                                v.set_error(message.clone(), remedy, cx);
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
                        let report =
                            ErrorReport::new(crate::surface::strings::error_acp_connect_failed())
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

/// Dedup key for a session advisory's toast.
///
/// Keyed by pane *and* content. A pane-only key looks right and quietly loses
/// information: dedup matches only against a toast still on screen, so a
/// connect that reports both "this conversation could not be restored" and
/// "the configured mode was refused" would show whichever landed first and
/// drop the other — the one case where both facts matter.
///
/// The digest only has to be stable within a run, which is all a live toast
/// queue spans; it keeps a multi-line adapter diagnostic out of the key that
/// lands in the log.
fn notice_dedup_key(pane_id: PaneId, message: &str) -> String {
    use std::hash::{Hash as _, Hasher as _};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    message.hash(&mut hasher);
    format!("agent_chat.notice.{pane_id}.{:x}", hasher.finish())
}

/// Whether a successful login into `target` should reconnect a pane that
/// resolves to `pane_target` and last failed with `remedy` (`None` when its
/// connection did not fail).
///
/// Both conditions are load-bearing. Matching the target keeps the sweep off
/// panes running on other credentials, which this login did nothing for. And
/// only `Reauthenticate` is a failure a fresh sign-in actually clears — an
/// organization-blocked account reconnects into the identical refusal, so
/// retrying it automatically would spend the user's attention on a loop
/// dressed up as progress.
fn login_revives_pane(
    target: crate::workspace::account_login_ops::LoginTarget,
    pane_target: Option<crate::workspace::account_login_ops::LoginTarget>,
    remedy: Option<daruda_acp::Remedy>,
) -> bool {
    pane_target == Some(target) && remedy == Some(daruda_acp::Remedy::Reauthenticate)
}

#[cfg(test)]
mod tests {
    use super::{
        agent_default_mode, agent_default_model, connect_model_preference, login_revives_pane,
        notice_dedup_key,
    };
    use crate::workspace::account_login_ops::LoginTarget;
    use daruda_acp::Remedy;
    use daruda_config::{AgentDefinition, AgentLaunch};
    use daruda_store::accounts::AccountRecipeId;

    fn claude_system() -> LoginTarget {
        LoginTarget::System {
            recipe: AccountRecipeId::Claude,
        }
    }

    /// A repeat of the same advisory should collapse onto the toast already on
    /// screen rather than stacking.
    #[test]
    fn the_same_advisory_from_one_pane_shares_a_key() {
        assert_eq!(
            notice_dedup_key(7, "could not restore this conversation"),
            notice_dedup_key(7, "could not restore this conversation")
        );
    }

    /// The case a pane-only key would break: a connect can report that history
    /// could not be restored *and* that the configured mode would not apply.
    /// Both matter, and dedup only ever matches a toast still on screen — so a
    /// shared key would silently swallow the second one.
    #[test]
    fn two_different_advisories_from_one_pane_do_not_collapse() {
        assert_ne!(
            notice_dedup_key(7, "could not restore this conversation"),
            notice_dedup_key(7, "the configured mode was refused")
        );
    }

    /// Two panes reporting the same thing are two separate facts about two
    /// separate sessions.
    #[test]
    fn the_same_advisory_from_two_panes_does_not_collapse() {
        assert_ne!(
            notice_dedup_key(7, "the configured mode was refused"),
            notice_dedup_key(8, "the configured mode was refused")
        );
    }

    #[test]
    fn a_pane_blocked_on_this_login_is_revived() {
        assert!(login_revives_pane(
            claude_system(),
            Some(claude_system()),
            Some(Remedy::Reauthenticate)
        ));
    }

    /// The signed-in credentials are not this pane's, so reconnecting it would
    /// re-run the same handshake with the same expired login.
    #[test]
    fn a_pane_on_other_credentials_is_left_alone() {
        assert!(!login_revives_pane(
            claude_system(),
            Some(LoginTarget::System {
                recipe: AccountRecipeId::Codex
            }),
            Some(Remedy::Reauthenticate)
        ));
    }

    /// An organization-blocked account is the case that must not reconnect: the
    /// login succeeds and changes nothing, so an automatic retry would fail
    /// identically while looking like the app is making progress.
    #[test]
    fn a_failure_a_login_cannot_fix_is_not_retried() {
        for remedy in [
            Remedy::ExternalAction,
            Remedy::Configure,
            Remedy::Retry,
            Remedy::NoneAvailable,
        ] {
            assert!(
                !login_revives_pane(claude_system(), Some(claude_system()), Some(remedy)),
                "{remedy:?} is not fixed by signing in"
            );
        }
    }

    /// A pane that never failed is mid-conversation. A login elsewhere in the
    /// app must not tear its live session down.
    #[test]
    fn a_working_pane_is_never_reconnected() {
        assert!(!login_revives_pane(
            claude_system(),
            Some(claude_system()),
            None
        ));
    }

    #[test]
    fn a_pane_with_no_resolvable_login_is_left_alone() {
        assert!(!login_revives_pane(
            claude_system(),
            None,
            Some(Remedy::Reauthenticate)
        ));
    }

    #[test]
    fn agent_default_mode_reads_the_matching_catalog_entry() {
        let agents = vec![
            AgentDefinition {
                id: "other".to_string(),
                name: "Other".to_string(),
                launch: AgentLaunch::Raw("run-other".to_string()),
                default_mode: Some("yolo".to_string()),
                default_model: None,
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

    #[test]
    fn agent_default_model_reads_the_matching_catalog_entry() {
        let agents = vec![
            AgentDefinition {
                id: "other".to_string(),
                name: "Other".to_string(),
                launch: AgentLaunch::Raw("run-other".to_string()),
                default_mode: None,
                default_model: Some("opus".to_string()),
            },
            AgentDefinition::claude_default(),
        ];
        assert_eq!(agent_default_model(&agents, "other"), Some("opus"));
        assert_eq!(
            agent_default_model(&agents, &AgentDefinition::claude_default().id),
            None,
            "an entry without a model leaves the adapter's own pick standing"
        );
        assert_eq!(
            agent_default_model(&agents, "gone"),
            None,
            "an id no longer in the catalog pins nothing"
        );
    }

    #[test]
    fn a_remembered_model_outranks_the_agent_default() {
        assert_eq!(
            connect_model_preference(Some("haiku"), Some("sonnet")),
            Some("haiku".to_string()),
            "the pane's own pick wins"
        );
        assert_eq!(
            connect_model_preference(None, Some("sonnet")),
            Some("sonnet".to_string()),
            "a pane that never picked starts on the agent's default"
        );
        assert_eq!(
            connect_model_preference(None, None),
            None,
            "neither axis names a model — nothing to apply"
        );
    }

    #[test]
    fn empty_model_preferences_leave_the_adapters_pick_standing() {
        assert_eq!(
            connect_model_preference(Some("  "), Some("sonnet")),
            None,
            "an explicit but empty remembered value does not fall back to the catalog default"
        );
        assert_eq!(
            connect_model_preference(None, Some("  ")),
            None,
            "an empty catalog default requests no handshake change"
        );
    }
}
