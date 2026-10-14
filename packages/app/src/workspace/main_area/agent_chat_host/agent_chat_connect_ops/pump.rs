//! The background ACP connect and event pump for one agent-chat pane, and
//! what a pane does when that stream ends.

use super::{
    AgentChatConnectionPlan, fail_connect_account_prepare, retries_fresh, runtime_prep_phase,
    session_resume,
};
use crate::surface::strings as s;
use crate::workspace::Workspace;
use crate::workspace::main_area::agent_chat_host::agent_chat_event_ops::PumpStep;
use crate::workspace::main_area::pane_tree::PaneId;
use daruda_acp::preparation::PreparationCancellation;
use daruda_acp::{
    ClientCapabilitiesView, NodeProgress, connect_prepared_session_with_capabilities,
};
use daruda_chat::view::AgentSessionStatus;
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::project::PaneCwd;
use futures::StreamExt as _;
use futures::channel::mpsc::unbounded;
use gpui::Context;

impl Workspace {
    /// Open the live ACP session for an already-pushed pane and store the
    /// event-pump task on its view; closing the pane drops both. `resume`
    /// carries the persisted session id: `Some` branches `session/load`,
    /// `None` starts `session/new`. Only non-task resumes may retry fresh.
    pub(in crate::workspace) fn connect_agent_chat(
        &mut self,
        pane_id: PaneId,
        cwd: PaneCwd,
        resume: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let load_intent = self
            .agent_chat_view(pane_id)
            .and_then(|view| view.read(cx).load_intent());
        let read_only = load_intent.is_some();
        let strict_restore = resume.is_some() && self.requires_exact_resume(pane_id, cx);
        let snapshot = load_intent == Some(daruda_chat::view::LoadIntent::Snapshot);

        // A CLI snapshot mirrors one exact session; with nothing to load there
        // is nothing to show, and `session/new` would invent a conversation.
        if read_only && resume.is_none() {
            if let Some(view) = self.agent_chat_view(pane_id).cloned() {
                view.update(cx, |v, cx| {
                    v.set_error(
                        s::task::cli_read_only(),
                        daruda_acp::Remedy::NoneAvailable,
                        cx,
                    )
                });
            }
            return;
        }

        if strict_restore && let Some(view) = self.agent_chat_view(pane_id).cloned() {
            let chat = view.read(cx);
            let account = self
                .main_area
                .pane(pane_id)
                .and_then(|pane| pane.agent_chat_content())
                .and_then(|chat| chat.account.to_persisted());
            if !self.task_chat_identity_available(chat.agent_id(), account) {
                view.update(cx, |v, cx| {
                    v.set_error(
                        s::task::chat_missing_agent(),
                        daruda_acp::Remedy::Configure,
                        cx,
                    )
                });
                self.notify_status_docks(cx);
                self.apply_agent_chat_task_ended(
                    pane_id,
                    daruda_store::tasks::SessionEndReason::Error,
                    cx,
                );
                return;
            }
        }

        let Some(plan) = self.prepare_agent_chat_connection(pane_id, &cwd, cx) else {
            self.apply_agent_chat_task_ended(
                pane_id,
                daruda_store::tasks::SessionEndReason::Error,
                cx,
            );
            return;
        };
        let AgentChatConnectionPlan {
            agent_id,
            launch_spec,
            connect_cwd,
            mut initial_model,
            mut initial_modes,
            prepared,
            mut mcp_servers,
        } = plan;
        if read_only {
            initial_model = None;
            initial_modes.clear();
        }
        if snapshot {
            mcp_servers.clear();
        }

        // DIAG: an ACP adapter spawn that fails with `os error 2` means the
        // launcher (`docker` / `npx` / `ssh`) was not on this process's PATH —
        // a GUI launch whose `hydrate_path_from_login_shell` was skipped or
        // bailed leaves only the minimal launchd PATH. Log the launcher +
        // effective PATH once per connect so the failure has ground truth
        // instead of a bare `-32603`. Info severity: no toast, NDJSON only.
        //
        // `command_diagnostic` rather than the command itself: an agent's
        // declared environment is folded into that string as `KEY='value'`,
        // and those values are whatever the user typed into Settings.
        {
            let path = std::env::var("PATH").unwrap_or_else(|_| "<unset>".to_string());
            let report = daruda_store::observability::error_report::ErrorReport::new(
                "ACP connect: resolved launch command",
            )
            .severity(daruda_store::observability::error_report::ErrorSeverity::Info)
            .with_context(
                "launcher",
                daruda_acp::command_diagnostic(&launch_spec.command),
            )
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
        let cancellation = PreparationCancellation::default();
        let cancel_on_drop = cancellation.cancel_on_drop();
        let progress_cancellation = cancellation.clone();
        cx.spawn(async move |this, cx| {
            while let Some(progress) = progress_rx.next().await {
                if progress_cancellation.is_canceled() {
                    break;
                }
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
                            v.status(),
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
            let _cancel_preparation = cancel_on_drop;
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
                    // Refuses under test, so no real adapter ever signs in.
                    let adapter = crate::agent::adapter_prep::prepare(
                        &launch_spec,
                        &cancellation,
                        &mut progress,
                    )?;
                    // `Some` resumes the persisted session (`session/load`);
                    // `None` starts a fresh session (`session/new`).
                    connect_prepared_session_with_capabilities(
                        adapter,
                        connect_cwd,
                        initial_model,
                        initial_modes,
                        session_resume(resume, strict_restore),
                        &agent_id,
                        mcp_servers,
                        ClientCapabilitiesView { notices: true },
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
                            v.attach_handle(handle);
                            if matches!(v.status(), AgentSessionStatus::PreparingRuntime(_)) {
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
                        if retries_fresh(was_resume, strict_restore, connected_seen, &event)
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
                                        v.detach_handle();
                                        v.begin_connect(None, cx);
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
                        if matches!(&event, daruda_acp::AcpEvent::Connected { .. }) {
                            connected_seen = true;
                        }
                        let step =
                            this.update(cx, |ws, cx| ws.fold_agent_chat_event(pane_id, event, cx));
                        match step {
                            Ok(PumpStep::Continue) => {}
                            // The pane already let go of its session; the stream
                            // ending now is not a failure to report.
                            Ok(PumpStep::Release) => return,
                            // Workspace/window gone (Err) or view gone — stop pumping.
                            Ok(PumpStep::Stop) | Err(_) => break,
                        }
                    }
                    // EOF without a terminal event can occur before connect or
                    // while idle. Release replay and retire any still-live pane
                    // through the same failure path as a pending-request EOF.
                    // SILENT-OK: view/window already gone at end-of-stream — nothing to release
                    let _ = this.update(cx, |ws, cx| {
                        ws.agent_chat_stream_ended(pane_id, cx);
                    });
                }
                Err(err) => {
                    // No ACP request ran: a preparation failure is not evidence
                    // of an invalid saved session. Keep its resume target intact.
                    let failure = err.into_failure();
                    let detail = failure.message().to_owned();
                    // workspace gone before the connect resolved — nothing left
                    // to surface the failure on.
                    // SILENT-OK: workspace/window dropped before connect resolved
                    let _ = this.update(cx, |ws, cx| {
                        if let Some(view) = ws.agent_chat_view(pane_id).cloned() {
                            let (syntax_theme, is_light) = ws.agent_chat_theme_params(cx);
                            let effect = view.update(cx, |v, cx| {
                                v.apply_event(
                                    daruda_acp::AcpEvent::Error(failure),
                                    &syntax_theme,
                                    is_light,
                                    cx,
                                )
                            });
                            ws.relay_phone_ack_effect(pane_id, effect, cx);
                            // Connecting → Error clears the badge (maps to
                            // `None`); dirty the cached docks so the stale
                            // Connecting badge doesn't linger after the pulse
                            // stops.
                            ws.notify_status_docks(cx);
                            ws.apply_agent_chat_task_ended(
                                pane_id,
                                daruda_store::tasks::SessionEndReason::Error,
                                cx,
                            );
                        }
                        let report =
                            ErrorReport::new(crate::surface::strings::error::acp_connect_failed())
                                .severity(ErrorSeverity::Error)
                                .with_context("detail", detail)
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
            view.update(cx, |v, _| v.attach_event_pump(pump));
        }
    }

    /// A live pane losing its stream is a connection failure, not necessarily
    /// a task failure: an idle pane may share a cwd with another active task.
    pub(in crate::workspace) fn agent_chat_stream_ended(
        &mut self,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.agent_chat_view(pane_id).cloned() else {
            return;
        };
        let needs_disconnect_error = view.read(cx).needs_disconnect_error();
        let failed_to_start_prompt = {
            let view = view.read(cx);
            matches!(
                view.status(),
                AgentSessionStatus::PreparingRuntime(_)
                    | AgentSessionStatus::Connecting
                    | AgentSessionStatus::Handshaking(_)
            ) && view.queue().has_pending()
        };
        view.update(cx, |v, cx| v.abort_restore(cx));
        if !needs_disconnect_error {
            return;
        }
        let (syntax_theme, is_light) = self.agent_chat_theme_params(cx);
        let effect = view.update(cx, |v, cx| {
            v.apply_event(
                daruda_acp::AcpEvent::Error(daruda_acp::AcpFailure::TransportClosed {
                    message: s::agent_chat::error_stream_ended(),
                }),
                &syntax_theme,
                is_light,
                cx,
            )
        });
        self.relay_phone_ack_effect(pane_id, effect, cx);
        let edge = view.update(cx, |v, cx| v.tick_activity(std::time::Instant::now(), cx));
        if let Some(outcome) = edge {
            self.fire_activity_completion(pane_id, outcome, cx);
        } else if failed_to_start_prompt {
            // Startup can fail before a task's buffered prompt starts a turn;
            // there is no activity completion edge in that case.
            self.apply_agent_chat_task_ended(
                pane_id,
                daruda_store::tasks::SessionEndReason::Error,
                cx,
            );
        }
        self.notify_status_docks(cx);
    }
}
