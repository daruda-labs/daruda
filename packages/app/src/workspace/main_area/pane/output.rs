//! PTY output draining and adaptive polling.

use super::*;

impl Workspace {
    #[allow(clippy::too_many_arguments)] // PTY plumbing — splitting wraps tax callers more than it saves.
    pub(super) fn spawn_stdout_poll(
        view: Entity<TerminalView>,
        stdout_rx: mpsc::Receiver<Vec<u8>>,
        exit_rx: mpsc::Receiver<()>,
        error_rx: mpsc::Receiver<daruda_store::observability::error_report::ErrorReport>,
        mut poke_rx: UnboundedReceiver<()>,
        workspace: Entity<Workspace>,
        pane_id: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        window.spawn(cx, async move |cx| {
            let mut streaming_ticks: u32 = 0;
            let mut idle_ticks: u32 = 0;
            // `false` once every poke sender is gone (pane teardown) —
            // plain timer sleeps from then on avoid a busy select loop.
            let mut poke_open = true;
            // Cached redraw cap (`1000 / render.max_fps` ms, mirrored on
            // Workspace; 30 fps default). Refreshed from the mirror only
            // on active ticks so a deep-idle pane performs no entity
            // reads; a config reload is picked up on the next activity.
            let mut cap = cx
                .update(|_, app| workspace.read(app).mirrors.terminal_redraw_interval)
                .unwrap_or(CAP_FALLBACK);
            loop {
                let interval = stdout_poll_interval(cap, streaming_ticks, idle_ticks);
                let mut poked = false;
                if poke_open {
                    let timer = cx.background_executor().timer(interval);
                    match futures::future::select(timer, poke_rx.next()).await {
                        Either::Left(((), _)) => {}
                        Either::Right((Some(()), _)) => {
                            poked = true;
                            // Collapse a poke burst (typed word, pasted
                            // block) into a single fast tick.
                            while poke_rx.try_recv().is_ok() {}
                        }
                        Either::Right((None, _)) => poke_open = false,
                    }
                } else {
                    cx.background_executor().timer(interval).await;
                }

                let mut batch = Vec::new();
                while let Ok(chunk) = stdout_rx.try_recv() {
                    batch.extend_from_slice(&chunk);
                    // Bound one UI tick even when the producer never goes idle.
                    if batch.len() >= 256 * 1024 {
                        break;
                    }
                }
                if batch.is_empty() {
                    streaming_ticks = 0;
                    idle_ticks = if poked {
                        0
                    } else {
                        idle_ticks.saturating_add(1)
                    };
                } else {
                    streaming_ticks = streaming_ticks.saturating_add(1);
                    idle_ticks = 0;
                }
                if idle_ticks == 0
                    && let Ok(v) =
                        cx.update(|_, app| workspace.read(app).mirrors.terminal_redraw_interval)
                {
                    cap = v;
                }

                // Drain PTY thread errors (writer/reader death) on
                // every tick so the user sees them within one frame.
                // We enrich each report with pane id + cwd so
                // the user can tell which session died — the PTY
                // threads themselves are GPUI-free and don't have
                // access to the workspace's cached cwd.
                let mut errors = Vec::new();
                while let Ok(report) = error_rx.try_recv() {
                    errors.push(report);
                }
                if !errors.is_empty() {
                    let workspace_for_errors = workspace.clone();
                    // If the workspace window has dropped before this
                    // drain catches up (process exit, last-tab-close
                    // race) the toast pipeline becomes unreachable —
                    // the per-report `report_error` calls would
                    // surface as toasts otherwise. Surface the path
                    // loss on NDJSON so we don't lose track of the
                    // fact that errors were dropped (Iron Law: no
                    // silent failure).
                    let update_result = cx.update(|_, app_cx| {
                        workspace_for_errors.update(app_cx, |ws, cx| {
                            let cwd = ws
                                .active_runtime()
                                .panes
                                .iter()
                                .find(|p| p.id == pane_id)
                                .and_then(|p| p.cwd())
                                .map(daruda_store::observability::system_info::redact_home);
                            for mut report in errors {
                                report
                                    .context
                                    .insert("pane".to_string(), pane_id.to_string());
                                if let Some(cwd) = cwd.clone() {
                                    report.context.insert("cwd".to_string(), cwd);
                                }
                                ws.report_error(report, cx);
                            }
                        });
                    });
                    if let Err(e) = update_result {
                        daruda_store::observability::log_writer::LogWriter::log(
                            daruda_store::observability::error_report::ErrorReport::new(
                                "Pane background errors could not reach the workspace toast layer",
                            )
                            .severity(
                                daruda_store::observability::error_report::ErrorSeverity::Warning,
                            )
                            .at(file!(), line!())
                            .with_context("pane", pane_id.to_string())
                            .with_context("error", format!("{e}"))
                            .dedup("pane.background_error.update_failed")
                            .build(),
                        );
                    }
                }

                // Treat both "sender signaled" and "sender gone" as
                // shell termination so a panicking waiter thread never
                // leaves the pane stuck with a dead shell.
                let exited = matches!(
                    exit_rx.try_recv(),
                    Ok(()) | Err(mpsc::TryRecvError::Disconnected)
                );

                if !batch.is_empty() {
                    let ok = cx.update(|_, cx| {
                        view.update(cx, |this, cx| {
                            this.queue_output_bytes(&batch, cx);
                            // Flush so terminal_title/terminal_cwd reflect the bytes
                            // we just queued — render() may not run before the
                            // workspace.update below reads them.
                            this.flush_pending_output(cx);
                        });
                        let v = view.read(cx);
                        let title = v.terminal_title().to_string();
                        let cwd = v.terminal_cwd().map(PathBuf::from);
                        workspace.update(cx, |ws, cx| {
                            let is_focused = ws.active_runtime().focused_pane_id == pane_id;
                            // Capture the focused pane's cwd before the
                            // update so a change can re-target the MCP
                            // watcher (the Project scope reads the
                            // focused terminal's cwd `.mcp.json`).
                            let prev_cwd = ws
                                .active_runtime()
                                .panes
                                .iter()
                                .find(|p| p.id == pane_id)
                                .and_then(|p| p.cwd().map(std::path::Path::to_path_buf));
                            let updated = ws
                                .active_runtime_mut()
                                .panes
                                .iter_mut()
                                .find(|p| p.id == pane_id)
                                .map(|p| p.update_cached_terminal(title, cwd))
                                .unwrap_or(false);
                            if updated {
                                cx.notify();
                                if is_focused {
                                    let new_cwd = ws
                                        .active_runtime()
                                        .panes
                                        .iter()
                                        .find(|p| p.id == pane_id)
                                        .and_then(|p| p.cwd().map(std::path::Path::to_path_buf));
                                    if new_cwd != prev_cwd {
                                        ws.refresh_mcp_on_cwd_change(cx);
                                    }
                                }
                            }
                        });
                    });
                    if ok.is_err() {
                        break;
                    }
                }

                if exited {
                    // Read the config flag now, before any sibling
                    // spawn, so the read and the later update never
                    // share a closure — avoids any ambiguity with
                    // GPUI's reentrant-update guard.
                    let should_close = cx
                        .update(|_, app_cx| workspace.read(app_cx).mirrors.close_pane_on_exit)
                        .unwrap_or(false);

                    if !should_close {
                        // SILENT-OK: the window is gone, and the pane with it
                        let _ = cx.update(|_, app_cx| {
                            workspace.update(app_cx, |ws, cx| ws.note_terminal_exited(pane_id, cx));
                        });
                    }
                    if should_close {
                        // Self-drop hazard: calling `close_pane_by_id`
                        // inline would remove our own Pane and drop
                        // this very Task mid-poll. A sibling task runs
                        // after we return, so our future completes
                        // before its owner is freed.
                        let workspace_close = workspace.clone();
                        cx.spawn(async move |cx| {
                            // SILENT-OK: pane owner may drop during async task cleanup
                            let _ = cx.update(|window, app_cx| {
                                workspace_close.update(app_cx, |ws, cx| {
                                    ws.close_pane_by_id(pane_id, window, cx);
                                });
                            });
                        })
                        .detach();
                    }
                    break;
                }
            }
        })
    }
}

/// Responsive idle poll so first output after a quiet period appears
/// promptly; the cap interval only kicks in once output is sustained.
pub(super) const IDLE_POLL: Duration = Duration::from_millis(16);
/// Consecutive non-empty drains before backing off to the cap.
pub(super) const STREAM_ENTER_TICKS: u32 = 3;
/// Consecutive empty drains tolerated at the fast interval before the
/// idle backoff starts (~128 ms of fast polling after the last byte).
pub(super) const IDLE_GRACE_TICKS: u32 = 8;
/// Ceiling for the idle backoff. Also bounds how late a shell exit or
/// an un-poked first byte (background process output) is noticed.
pub(super) const IDLE_BACKOFF_MAX: Duration = Duration::from_millis(250);
/// 16 ms × 2⁴ = 256 ms ≥ `IDLE_BACKOFF_MAX` — further doublings are moot.
pub(super) const IDLE_BACKOFF_MAX_DOUBLINGS: u32 = 4;
/// Redraw-cap fallback when the workspace mirror is unreachable
/// (30 fps default, matching `render.max_fps`).
pub(super) const CAP_FALLBACK: Duration = Duration::from_millis(33);

/// Poll interval for the stdout drain loop, from the redraw cap and the
/// two drain counters.
///
/// Regimes: sustained output (≥ [`STREAM_ENTER_TICKS`] non-empty drains)
/// polls at the redraw cap; recent activity (output or an input poke
/// within [`IDLE_GRACE_TICKS`] drains) polls fast so a keystroke echo is
/// never held longer than [`IDLE_POLL`]; past the grace window the
/// interval doubles per empty drain up to [`IDLE_BACKOFF_MAX`] so a
/// quiet pane costs ~4 wakes/s instead of ~60.
pub(super) fn stdout_poll_interval(
    cap: Duration,
    streaming_ticks: u32,
    idle_ticks: u32,
) -> Duration {
    if streaming_ticks >= STREAM_ENTER_TICKS {
        return cap;
    }
    let fast = cap.min(IDLE_POLL);
    if idle_ticks <= IDLE_GRACE_TICKS {
        return fast;
    }
    let doublings = (idle_ticks - IDLE_GRACE_TICKS).min(IDLE_BACKOFF_MAX_DOUBLINGS);
    (fast * (1u32 << doublings)).min(IDLE_BACKOFF_MAX)
}
