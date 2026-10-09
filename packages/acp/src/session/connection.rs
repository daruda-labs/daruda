//! Connection handshake and initialization before the prompt queue starts.

use super::*;

/// Wall-clock budget for `initialize` and — on a *fresh* session —
/// `session/new` and the optional `set_mode`. Without this, a hung adapter
/// (e.g. an SSH-wrapped remote command stuck on a silent auth prompt, or a
/// dead network) parks `block_task().await` forever: no event ever reaches
/// the host, so the connecting status never resolves. `prompt_loop` is
/// deliberately NOT covered — a live session with no traffic is normal, not a
/// hang. `session/load` (a resume) is NOT covered by this budget — see
/// [`CONNECT_RESUME_LOAD_TIMEOUT`]: these three requests carry no bulk
/// payload and a healthy adapter answers in milliseconds, so a generous but
/// still-bounded 60s is appropriate.
pub(super) const CONNECT_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(60);

/// Wall-clock budget for `session/load` alone. Separate from
/// [`CONNECT_HANDSHAKE_TIMEOUT`] because a resume's response only arrives
/// after the adapter has replayed the *entire* prior conversation as
/// `session/update` notifications first — a large history, or a slow link
/// (e.g. the SSH-wrapped remote agent), can legitimately take much longer
/// than a fresh `session/new`'s near-instant reply without being hung. Still
/// bounded, so a genuinely stuck load (the same class of bug this whole
/// timeout mechanism exists for) doesn't strand the pane forever.
pub(super) const CONNECT_RESUME_LOAD_TIMEOUT: Duration = Duration::from_secs(300);

/// Race `fut` against a `timeout` timer; on timeout, returns a synthetic
/// protocol error naming `what` so the host's error banner names the stuck
/// step rather than a generic failure. `fut` must resolve on its own within
/// the timeout — cancellation here only stops *waiting* for it (the future is
/// dropped), it does not abort a stuck subprocess.
pub(super) async fn with_connect_timeout<T>(
    what: &str,
    timeout: Duration,
    fut: impl Future<Output = Result<T, agent_client_protocol::Error>>,
) -> Result<T, agent_client_protocol::Error> {
    // ALLOW: this crate is GPUI-free (see packages/acp/CLAUDE.md) and has
    // no BackgroundExecutor to time on; smol::Timer is the only timer source
    // available here. Tests pass a short explicit `timeout`, so this stays
    // deterministic.
    #[allow(clippy::disallowed_methods)]
    let timer = smol::Timer::after(timeout);
    match futures::future::select(Box::pin(fut), Box::pin(timer)).await {
        Either::Left((result, _)) => result,
        Either::Right(_) => Err(agent_client_protocol::Error::new(
            -32603,
            format!(
                "{what} timed out after {}s — check the agent command and network reachability \
                 (e.g. SSH host connectivity) and retry",
                timeout.as_secs()
            ),
        )),
    }
}

/// Drive the whole connection: handshake, session creation, then the prompt /
/// cancel select loop, until the command channel closes or the protocol fails.
///
/// Generic over the transport (production passes the subprocess-spawning
/// [`AcpAgent`]) so tests can wire an in-process fake agent — an SDK
/// `Agent.builder()` implements `ConnectTo<Client>` too — and drive this exact
/// code path deterministically, dispatch semantics included.
// Internal connection state threaded through one call — bundling wraps callers more than it saves.
#[allow(clippy::too_many_arguments)]
pub(super) async fn run_connection(
    agent: impl ConnectTo<Client> + 'static,
    cwd: PathBuf,
    initial_model: Option<String>,
    initial_modes: Vec<String>,
    resume: impl Into<SessionResume>,
    mcp_servers: Vec<McpServer>,
    command_rx: UnboundedReceiver<Command>,
    event_tx: UnboundedSender<AcpEvent>,
    permission_parks: PermissionParks,
) -> Result<(), AcpClientError> {
    let resume = resume.into();
    let notif_tx = event_tx.clone();
    let perm_event_tx = event_tx.clone();
    let next_permission_id = Arc::new(AtomicU64::new(0));
    // Owns the session's mode state for the whole connection; see
    // `crate::mode_tracker` for why mode can't be forwarded as it arrives.
    let mode_tracker = ModeTracker::default();
    let notif_mode_tracker = mode_tracker.clone();
    // Holds this connection's subagent session graph. Shared with the
    // notification handler, which is registered before `initialize` answers —
    // hence the handle rather than a value.
    let router = Arc::new(Mutex::new(NativeSubagentRouter::default()));
    let notif_router = router.clone();

    agent_client_protocol::Client
        .builder()
        .on_receive_notification(
            async move |notification: CompatSessionNotification, _cx| {
                // Routing happens before anything is typed: a native subagent's
                // update has no variant in this schema version, and the SDK
                // drops what it cannot parse (logged, never surfaced), so the
                // child's whole run would vanish without this hop.
                // One lock for both questions, and the advisory is only
                // asked for on the arm that emits it: `first_legacy_delegation`
                // spends a once-per-session slot, so asking anywhere else
                // costs the notice without reporting it.
                let (routed, legacy_delegation) = {
                    let mut router = notif_router
                        .lock()
                        .expect("native subagent router mutex poisoned");
                    let routed = router.route(&notification.session_id.0, &notification.update);
                    let legacy = matches!(routed, Routed::Standard(_))
                        && router.first_legacy_delegation(&notification.update);
                    (routed, legacy)
                };
                match routed {
                    Routed::Standard(update) => {
                        // The agent delegated the old way, so the subagent's own
                        // calls are never sent — the transcript would just show
                        // an opaque `spawnAgent` and nothing about what ran.
                        if legacy_delegation {
                            let _ = notif_tx.unbounded_send(AcpEvent::LegacyDelegation);
                        }
                        forward_session_update(*update, &notif_mode_tracker, &notif_tx);
                    }
                    Routed::Normalized(updates) => {
                        for update in updates {
                            forward_session_update(update, &notif_mode_tracker, &notif_tx);
                        }
                    }
                    // An update kind this build does not know is reported and
                    // skipped: an agent that adds one must not be able to break
                    // a session that otherwise works.
                    Routed::Unknown { kind } => {
                        let _ = notif_tx.unbounded_send(AcpEvent::Notice(format!(
                            "Ignoring an unrecognized session update from the agent ({kind})."
                        )));
                    }
                }
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            {
                let permission_parks = permission_parks.clone();
                let next_permission_id = next_permission_id.clone();
                let perm_event_tx = perm_event_tx.clone();
                async move |request: RequestPermissionRequest, responder, connection| {
                    let id = next_permission_id.fetch_add(1, Ordering::Relaxed);
                    let (decision_tx, decision_rx) = oneshot::channel::<PermissionDecision>();
                    permission_parks
                        .lock()
                        .expect("permission parks mutex poisoned")
                        .insert(id, decision_tx);

                    let _ = perm_event_tx.unbounded_send(AcpEvent::PermissionRequested {
                        id,
                        request: Box::new(request),
                    });

                    // Park in a spawned task, NOT in this handler: the SDK runs
                    // handlers inline on the connection's single dispatch task
                    // ("the server will not process new messages until this
                    // handler returns"), so awaiting the host's decision here
                    // would freeze every update queued behind the request —
                    // streaming, tool progress, a second permission request —
                    // until the user answers. Measured by
                    // `a_parked_permission_request_does_not_stall_update_dispatch`.
                    // If the sender is dropped (host went away / id reaped on
                    // shutdown), default to Cancelled — deny by absence.
                    connection.spawn(async move {
                        let decision = decision_rx.await.unwrap_or(PermissionDecision::Cancelled);
                        responder.respond(decision.into_response())
                    })
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(agent, move |connection: ConnectionTo<Agent>| async move {
            // Each handshake step gets its own deadline rather than one budget
            // for the whole sequence, because `session/load` is structurally
            // different: its response only arrives after the adapter replays
            // the *entire* prior conversation, which can legitimately take far
            // longer than `initialize`/`session/new`/`set_mode`'s near-instant
            // replies without being hung — see `CONNECT_RESUME_LOAD_TIMEOUT`.
            // `prompt_loop` below is deliberately outside every timeout here —
            // a live, quiet session is normal.
            let _ = event_tx.unbounded_send(AcpEvent::ConnectProgress(ConnectPhase::Handshaking));
            let (capabilities, login_methods, program, resume) =
                with_connect_timeout("initialize", CONNECT_HANDSHAKE_TIMEOUT, async {
                    let init = connection
                        .send_request(
                            InitializeRequest::new(ProtocolVersion::V1)
                                .client_capabilities(client_capabilities()),
                        )
                        .block_task()
                        .await?;
                    let capabilities = session_capabilities_from_protocol(&init.agent_capabilities);
                    let login_methods = parse_login_methods(&init.auth_methods);
                    let program = init.agent_info.map(|info| info.name);

                    // Gate the requested resume on advertised `session/load` support:
                    // downgrade to a fresh session (with a Notice) when the agent can't
                    // replay history, so a resume against a non-load agent no longer
                    // fails the whole connect.
                    let (resume, resume_notice) = resume.resolve(capabilities.load)?;
                    if let Some(notice) = resume_notice {
                        let _ = event_tx.unbounded_send(AcpEvent::Notice(notice));
                    }
                    Ok((capabilities, login_methods, program, resume))
                })
                .await?;
            // Before the branch below: a `session/load` inside it replays the
            // conversation as updates, and those must map under this program.
            let _ = event_tx.unbounded_send(AcpEvent::AgentIdentified {
                program: program.clone(),
            });
            // Same ordering requirement as the event above, for the same
            // reason: a `session/load` below replays under this program, and
            // the router reads that dialect's `_meta` to tell a subagent's
            // answer from its preamble.
            router
                .lock()
                .expect("native subagent router mutex poisoned")
                .set_adapter(crate::adapter::adapter_for(program.as_deref(), ""));

            // New session, or resume an existing one via session/load. A load
            // replays the prior conversation as session/update notifications
            // (handled by the notification handler above) before the response
            // resolves, so the host rebuilds its items exactly as for a live
            // turn. Both paths yield the same (session_id, modes, config_options)
            // and share the set_mode + Connected tail below.
            let (session_id, mut modes, mut config_options): (
                SessionId,
                Option<ModeStateView>,
                Vec<ConfigOptionView>,
            ) = match resume {
                Some(id) => {
                    let _ = event_tx
                        .unbounded_send(AcpEvent::ConnectProgress(ConnectPhase::LoadingSession));
                    with_connect_timeout("session/load", CONNECT_RESUME_LOAD_TIMEOUT, async {
                        let loaded = connection
                            .send_request(build_load_session_request(id.clone(), cwd, mcp_servers))
                            .block_task()
                            .await?;
                        Ok((
                            id,
                            loaded.modes.as_ref().map(Into::into),
                            config_options_from_protocol(
                                loaded.config_options.as_deref().unwrap_or(&[]),
                            ),
                        ))
                    })
                    .await?
                }
                None => {
                    let _ = event_tx
                        .unbounded_send(AcpEvent::ConnectProgress(ConnectPhase::CreatingSession));
                    with_connect_timeout("session/new", CONNECT_HANDSHAKE_TIMEOUT, async {
                        let new_session = connection
                            .send_request(build_new_session_request(cwd, mcp_servers))
                            .block_task()
                            .await?;
                        Ok((
                            new_session.session_id.clone(),
                            new_session.modes.as_ref().map(Into::into),
                            config_options_from_protocol(
                                new_session.config_options.as_deref().unwrap_or(&[]),
                            ),
                        ))
                    })
                    .await?
                }
            };

            // A model can rebuild the mode list, so settle it first. This also
            // happens before `Connected`, which is the host's gate for draining
            // prompts queued during the handshake.
            apply_initial_model(
                &connection,
                &session_id,
                initial_model.as_deref(),
                &mut modes,
                &mut config_options,
                &event_tx,
            )
            .await;

            // Apply the host's requested mode on every connect, a `session/load`
            // included: the host decides which mode a session runs in, so
            // whatever the adapter reports after a load (`claude-agent-acp`
            // recomputes it from `settings.json` per launch) does not stand.
            //
            // Try each candidate in turn and stop at the first the
            // adapter both advertises and accepts. A candidate that is not
            // advertised is skipped without a request; one whose set_mode is
            // rejected falls through to the next, so a preferred-but-unavailable
            // mode (e.g. `bypassPermissions`) degrades to its fallback (`auto`)
            // rather than leaving the session in an arbitrary state.
            //
            // Every candidate failing is NON-FATAL: the session already
            // succeeded and is usable. Leave mode_state.current at the adapter's
            // real current mode (the chip reflects that), emit a Notice so the
            // host can log it, and continue to Connected.
            if let Some(mode_state) = modes.as_mut() {
                let mut applied = false;
                let mut last_reject: Option<(String, String)> = None;
                for id in &initial_modes {
                    // Not advertised — this candidate can't apply; try the next.
                    if !mode_state.available.iter().any(|m| &m.id == id) {
                        continue;
                    }
                    // Already in this mode — nothing to send, we're done.
                    if &mode_state.current == id {
                        applied = true;
                        break;
                    }
                    let _ = event_tx
                        .unbounded_send(AcpEvent::ConnectProgress(ConnectPhase::ApplyingMode));
                    let set_mode_result = with_connect_timeout(
                        "session/set_mode",
                        CONNECT_HANDSHAKE_TIMEOUT,
                        connection
                            .send_request(SetSessionModeRequest::new(
                                session_id.clone(),
                                id.clone(),
                            ))
                            .block_task(),
                    )
                    .await;
                    match set_mode_result {
                        Ok(_) => {
                            mode_state.current = id.clone();
                            applied = true;
                            break;
                        }
                        // Rejected — remember it and fall through to the fallback.
                        Err(e) => last_reject = Some((id.clone(), format!("{e:?}"))),
                    }
                }
                // Only notice when a candidate was actually attempted and rejected
                // and no later candidate applied — a purely non-advertised list is
                // forward-compatible silence (matches a non-modes adapter).
                if !applied && let Some((id, err)) = last_reject {
                    let _ = event_tx.unbounded_send(AcpEvent::Notice(format!(
                        "set_mode({id}) on connect failed — session is active in the \
                         adapter's default mode: {err}"
                    )));
                }
            }

            // Hand the (post-`set_mode`) state to the tracker before any
            // mode-bearing traffic can reach the host: from here on it is the
            // one owner, and `Connected` is the last place mode arrives by any
            // other route. Seeding `None` marks a modeless agent permanently
            // inert, so mode affordances stay a connect-time decision.
            mode_tracker.seed(modes.clone());

            let _ = event_tx.unbounded_send(AcpEvent::Connected {
                session_id: session_id.to_string(),
                modes,
                // Mode is carried by `modes` above; strip the duplicate so the
                // host has exactly one representation of it (mirrors what
                // `send_config_options_fold` does for every later set).
                config_options: crate::mode_tracker::strip_mode_options(config_options),
                capabilities,
                login_methods,
                program,
            });

            prompt_loop(
                &connection,
                session_id,
                command_rx,
                &event_tx,
                &mode_tracker,
                capabilities,
            )
            .await?;
            Ok(())
        })
        .await
        .map_err(|e| AcpClientError::Protocol(AcpFailure::classify(&e)))?;

    Ok(())
}

/// Apply a host-requested model during the handshake. Optional by design: a
/// missing choice or a refusal leaves the adapter's current model standing,
/// matching the AgentChat default semantics while still guaranteeing that any
/// accepted switch completes before the first prompt can be sent.
pub(super) async fn apply_initial_model(
    connection: &ConnectionTo<Agent>,
    session_id: &SessionId,
    wanted: Option<&str>,
    modes: &mut Option<ModeStateView>,
    config_options: &mut Vec<ConfigOptionView>,
    event_tx: &UnboundedSender<AcpEvent>,
) {
    let Some(wanted) = wanted.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    let Some(option) = config_options
        .iter()
        .find(|option| option.category == ConfigOptionCategoryView::Model)
    else {
        return;
    };
    let ConfigOptionKindView::Select {
        current_value,
        options,
    } = &option.kind
    else {
        return;
    };
    if current_value == wanted || !options.iter().any(|choice| choice.value == wanted) {
        return;
    }
    let config_id = option.id.clone();
    let request = SetSessionConfigOptionRequest::new(
        session_id.clone(),
        config_id.clone(),
        SessionConfigOptionValue::value_id(wanted.to_string()),
    );
    match with_connect_timeout(
        "session/set_config_option(model)",
        CONNECT_HANDSHAKE_TIMEOUT,
        connection.send_request(request).block_task(),
    )
    .await
    {
        Ok(response) => {
            let updated = config_options_from_protocol(&response.config_options);
            if let Some(updated_modes) = ModeStateView::from_config_options(&updated) {
                *modes = Some(updated_modes);
            }
            *config_options = updated;
        }
        Err(error) => {
            let _ = event_tx.unbounded_send(AcpEvent::Notice(format!(
                "set_config_option({config_id}={wanted}) on connect failed — the session uses \
                 the adapter's current model: {error:?}"
            )));
        }
    }
}
