//! Hook-driven terminal session status and pane bindings.
//! Dropping the task and watcher guards stops their background pumps.

use std::collections::HashMap;

use gpui::Context;

use crate::workspace::Workspace;
use crate::workspace::main_area::pane_tree::PaneId;
use daruda_agent::pty_tracker::PtyBinding;
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::system_info::redact_home;

pub(in crate::workspace) struct ClaudeSessionState {
    /// Terminal session-status mirror driven by agent hooks. The left dock
    /// reads it to render each lane's
    /// Working/NeedsAttention/Idle/Connecting indicator.
    pub(in crate::workspace) status: daruda_agent::ClaudeStatusStore,

    /// Pane sessions named by routed hooks, shared by badge aggregation
    /// and the known-process exit projection.
    pub(in crate::workspace) pty_bindings: HashMap<PaneId, PtyBinding>,
    pub(in crate::workspace) task_hook_tail: smol::channel::Receiver<()>,

    /// Running count of `PostToolUseFailure` hook events per Claude
    /// session. When a session crosses
    /// `daruda_store::tasks::TASK_TOOL_USE_FAILURE_THRESHOLD` the owning task
    /// is escalated to `Error { message: "tool_use_failure xN" }`.
    /// Cleared whenever a session is removed or transitioned out of
    /// `Running`.
    pub(in crate::workspace) tool_use_failure_counts: HashMap<String, u32>,

    /// Last blocking-notification timestamp already handled by this workspace,
    /// per session. The status-dir watcher can re-deliver
    /// the same file (FSEvents coalescing / duplicate events) and every
    /// hook write replaces the store entry, so neither the store nor the
    /// `update` return value dedups for us. Fire a push only when the
    /// incoming notification timestamp is newer than the one recorded
    /// here. Pruned alongside the session on removal.
    pub(in crate::workspace) last_pushed_notification:
        HashMap<daruda_agent::StatusKey, chrono::DateTime<chrono::Utc>>,
    /// Turns the hook channel has seen open, so a `Stop` can be told from
    /// a re-delivered one. Fed by hook updates only; see `turn_end`.
    pub(in crate::workspace) open_turns: daruda_agent::hooks::turn_end::OpenTurns,
}

/// Marks that this app has swept `~/.daruda/status/` once. A GPUI global
/// rather than a process static, so each test app sweeps its own.
struct ClaudeStatusSwept;

impl gpui::Global for ClaudeStatusSwept {}

/// The session statuses a new window starts from. The first window of the
/// app runs the cold restore — resetting stale files and deleting orphans
/// left by a past run; every later one only reads, since by then the files
/// belong to sessions the open windows are following.
pub(in crate::workspace) fn initial_claude_status(
    config: &daruda_config::Config,
    cx: &mut gpui::App,
) -> daruda_agent::ClaudeStatusStore {
    use daruda_agent::hooks::cold_restore;
    let mut store = daruda_agent::ClaudeStatusStore::new();
    let Ok(dir) = daruda_agent::hooks::status_file::default_dir() else {
        return store;
    };
    let initial = if cx.has_global::<ClaudeStatusSwept>() {
        cold_restore::load(&dir)
    } else {
        cx.set_global(ClaudeStatusSwept);
        match daruda_agent::hooks::history::terminals(&dir) {
            Ok(terminals) => {
                let history_dir = dir.clone();
                cx.background_executor()
                    .spawn(async move {
                        if let Err(error) = daruda_agent::hooks::history::reconcile_terminals(
                            &history_dir,
                            &terminals,
                        ) {
                            daruda_store::observability::log_writer::LogWriter::log(
                                ErrorReport::new("Terminal history startup reconciliation failed")
                                    .from_error(&error)
                                    .build(),
                            );
                        }
                    })
                    .detach();
            }
            Err(error) => {
                daruda_store::observability::log_writer::LogWriter::log(
                    ErrorReport::new("Terminal history startup discovery failed")
                        .from_error(&error)
                        .build(),
                );
            }
        }
        let policy = cold_restore::ColdRestorePolicy::from_config_secs(
            config.claude_status.stale_threshold_secs,
            config.claude_status.file_ttl_days,
        );
        cold_restore::run(&dir, &policy)
    };
    if let Ok(initial) = initial {
        store.load_initial(initial);
    }
    store
}

impl ClaudeSessionState {
    /// Drop the tool-use failure tallies of sessions a task let go of.
    /// Session ids are never reused, so a tally left behind is only dead
    /// weight.
    pub(in crate::workspace) fn forget_tool_use_failures<'a>(
        &mut self,
        sessions: impl IntoIterator<Item = &'a String>,
    ) {
        for sid in sessions {
            self.tool_use_failure_counts.remove(sid);
        }
    }
}

impl Workspace {
    /// Apply one filesystem event from `~/.daruda/status/`. Pumped by
    /// `main.rs` from the global `hooks::watcher`.
    pub fn apply_claude_status_event(
        &mut self,
        event: crate::hooks::watcher::StatusEvent,
        cx: &mut Context<Self>,
    ) {
        use crate::hooks::watcher::StatusEvent;
        match event {
            StatusEvent::Changed(path) => match daruda_agent::hooks::status_file::read(&path) {
                Ok(Some(file)) => {
                    if self
                        .claude_sessions
                        .status
                        .get(&file.identity())
                        .is_some_and(|previous| previous.timestamp > file.timestamp)
                    {
                        return;
                    }
                    if !self.bind_routed_session(&file, cx) {
                        return;
                    }
                    if matches!(file.last_event.as_str(), "SessionStart" | "SessionEnd") {
                        crate::runs::RunsHost::cli_tracker(cx).poke();
                    }
                    self.apply_task_hook_observation(&file, cx);
                    if file.last_event == "SessionEnd" {
                        self.claude_sessions.pty_bindings.retain(|_, binding| {
                            match &file.terminal_id {
                                Some(terminal) => binding.terminal_id.as_ref() != Some(terminal),
                                None => binding.session_id != file.session_id,
                            }
                        });
                        self.forget_terminal_session(&file.identity());
                        self.notify_status_docks(cx);
                        return;
                    }
                    // Blocking hook notifications surface as a one-shot
                    // local + remote push here rather than latching the lane
                    // indicator (see `daruda_agent::hooks::fsm`). Called
                    // before `update` consumes `file`.
                    self.maybe_push_hook_notification(&file, cx);
                    #[cfg(debug_assertions)]
                    let dbg_probe = self.probe_lane_status(&file.identity());
                    #[cfg(debug_assertions)]
                    let dbg_fields = (file.identity(), file.cwd.clone(), file.last_event.clone());
                    let turn_end = self
                        .claude_sessions
                        .open_turns
                        .observe(&file.identity(), file.status, &file.last_event)
                        .zip(self.session_pane_id(&file.identity()));
                    if self.claude_sessions.status.update(file) {
                        self.notify_status_docks(cx);
                        #[cfg(debug_assertions)]
                        {
                            let (sid, cwd, event) = dbg_fields;
                            self.log_lane_status_change(dbg_probe, &sid, &cwd, &event);
                        }
                    }
                    if let Some((outcome, pane_id)) = turn_end {
                        self.record_agent_outcome(pane_id, outcome, cx);
                    }
                }
                Ok(None) => {
                    // Mid-write or malformed; skip silently.
                }
                Err(e) => {
                    let report = ErrorReport::new(
                        crate::surface::strings::error::claude_status_file_read_failed(),
                    )
                    .severity(ErrorSeverity::Warning)
                    .from_error(&e)
                    .at(file!(), line!())
                    .with_context("path", redact_home(&path))
                    .dedup("claude.status.read")
                    .build();
                    self.report_error(report, cx);
                }
            },
            StatusEvent::Removed { key, .. } => {
                // A remove may arrive after the same terminal's next session
                // rewrote the file. In that case ingest the current version.
                if let Ok(dir) = daruda_agent::hooks::status_file::default_dir() {
                    let path = daruda_agent::hooks::status_file::path_for(&dir, &key);
                    if path.exists() {
                        self.apply_claude_status_event(StatusEvent::Changed(path), cx);
                        return;
                    }
                }
                // A filename shared by legacy and routed identities cannot
                // identify which observation ended; keep both until clarified.
                if self
                    .claude_sessions
                    .status
                    .file_for_storage_key(&key)
                    .is_none()
                    && self
                        .claude_sessions
                        .status
                        .iter()
                        .any(|(identity, _)| identity.storage_key() == key)
                {
                    return;
                }
                let identity = self
                    .claude_sessions
                    .status
                    .file_for_storage_key(&key)
                    .map(|file| file.identity())
                    .unwrap_or_else(|| daruda_agent::StatusKey::LegacySession(key.clone()));
                let session_id = key;
                crate::runs::RunsHost::cli_tracker(cx).poke();
                self.claude_sessions
                    .last_pushed_notification
                    .remove(&identity);
                self.claude_sessions.open_turns.forget(&identity);
                if let Some(file) = self.claude_sessions.status.get(&identity).cloned() {
                    let session = file.session_id;
                    let terminal = file.terminal_id.map(|id| id.as_str().to_owned());
                    self.after_task_projection(
                        move |_, cx| {
                            crate::runs::RunsHost::settle_cli_terminal(
                                &session,
                                terminal,
                                daruda_store::tasks::SessionEndReason::Other,
                                cx,
                            );
                        },
                        cx,
                    );
                } else {
                    self.apply_task_session_ended(
                        &session_id,
                        daruda_store::tasks::SessionEndReason::Other,
                        cx,
                    );
                }
                #[cfg(debug_assertions)]
                let dbg_entry = self
                    .claude_sessions
                    .status
                    .get(&identity)
                    .map(|f| f.cwd.clone());
                #[cfg(debug_assertions)]
                let dbg_probe = dbg_entry
                    .as_ref()
                    .map(|_| self.probe_lane_status(&identity));
                if self.claude_sessions.status.remove(&identity).is_some() {
                    self.notify_status_docks(cx);
                    #[cfg(debug_assertions)]
                    if let (Some(cwd), Some(probe)) = (dbg_entry, dbg_probe) {
                        self.log_lane_status_change(probe, &identity, &cwd, "removed");
                    }
                }
            }
        }
    }

    /// Bind a routed hook only in the window owning its live terminal.
    /// PID observations and pane bindings do not depend on a task execution.
    fn bind_routed_session(
        &mut self,
        file: &daruda_agent::hooks::status_file::StatusFile,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(terminal) = &file.terminal_id else {
            return true;
        };
        let Some(pane) = crate::runs::RunsHost::terminal_pane(terminal, cx)
            .filter(|pane| pane.workspace == self.uuid())
        else {
            return false;
        };
        let binding = PtyBinding {
            terminal_id: file.terminal_id.clone(),
            claude_pid: file.agent_pid,
            session_id: file.session_id.clone(),
        };
        if self.claude_sessions.pty_bindings.get(&pane.pane) != Some(&binding) {
            if let Some(pid) = file.agent_pid {
                crate::runs::RunsHost::cli_tracker(cx).track_agent(file.session_id.clone(), pid);
            }
            if let Some(previous) = self.claude_sessions.pty_bindings.insert(pane.pane, binding)
                && previous.session_id != file.session_id
            {
                self.forget_terminal_session(&previous.identity());
            }
        }
        true
    }

    /// Clear pane-owned status without unlinking a hook writer's files.
    pub(in crate::workspace) fn forget_terminal_session(
        &mut self,
        identity: &daruda_agent::StatusKey,
    ) {
        self.claude_sessions.status.remove(identity);
        self.claude_sessions.open_turns.forget(identity);
        self.claude_sessions
            .last_pushed_notification
            .remove(identity);
        // Task tallies are retired by the ordered task-projection queue;
        // clearing them here could erase an earlier pending failure event.
    }

    /// An interrupt sent to the PTY ends the visible turn without completion.
    pub(in crate::workspace) fn interrupt_terminal_agent(
        &mut self,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self
            .claude_sessions
            .pty_bindings
            .get(&pane_id)
            .map(PtyBinding::identity)
        else {
            return;
        };
        let Some(mut file) = self.claude_sessions.status.get(&session).cloned() else {
            return;
        };
        if !matches!(
            file.status,
            daruda_agent::SessionStatus::Working
                | daruda_agent::SessionStatus::ExecutingTool
                | daruda_agent::SessionStatus::NeedsAttention
        ) {
            return;
        }
        file.status = daruda_agent::SessionStatus::Idle;
        file.last_event = "Interrupt".to_owned();
        file.notification = None;
        file.timestamp = chrono::Utc::now();
        self.claude_sessions.open_turns.forget(&session);
        self.claude_sessions.status.update(file);
        self.notify_status_docks(cx);
    }

    /// Raise transient desktop and remote pushes for a blocking Claude
    /// `Notification` (permission prompt / idle prompt / elicitation
    /// dialog). These do not latch the lane indicator into a
    /// persistent `NeedsAttention` (see `daruda_agent::hooks::fsm`), so
    /// this one-shot push is their only surface.
    ///
    /// Gated by the `hook_notification_enabled` config and the shared
    /// "skip the focused pane" rule (the user is already looking at the
    /// TUI prompt), and deduped by timestamp so a status file the
    /// watcher re-delivers does not double-fire.
    fn maybe_push_hook_notification(
        &mut self,
        file: &daruda_agent::hooks::status_file::StatusFile,
        cx: &mut Context<Self>,
    ) {
        use crate::surface::strings as s;
        use daruda_agent::hooks::events::NotificationType;

        let title = match file.notification {
            Some(NotificationType::PermissionPrompt) => s::notification::hook_permission_title(),
            Some(NotificationType::IdlePrompt) => s::notification::hook_idle_title(),
            Some(NotificationType::ElicitationDialog) => s::notification::hook_elicitation_title(),
            // Informational subtypes and non-notification events never push.
            _ => return,
        };

        if !self.mirrors.notifications.hook_notification_enabled {
            return;
        }

        // Freshness gate — a notification re-delivered past the stale
        // threshold (cold-restore rewrite by another instance, watcher
        // replay of an old file) is no longer actionable; pushing it
        // would tell the user to answer a prompt that is long gone.
        if file.event_expired(
            chrono::Utc::now(),
            std::time::Duration::from_secs(self.mirrors.stale_threshold_secs),
        ) {
            return;
        }

        // Dedup: skip when this notification is not strictly newer than
        // the last one already surfaced for the session.
        if self
            .claude_sessions
            .last_pushed_notification
            .get(&file.identity())
            .is_some_and(|&t| file.timestamp <= t)
        {
            return;
        }
        // Record before the focus-gate return below: a re-delivered file
        // that was suppressed by focus must not fire late once focus
        // moves away — the user already saw the TUI prompt.
        self.claude_sessions
            .last_pushed_notification
            .insert(file.identity(), file.timestamp);

        // In-view gate — silence when daruda is foreground and the
        // session's own pane is on screen, mirroring `handle_view_event`.
        if self.mirrors.notifications.skip_focused_pane
            && crate::platform::attention::is_app_active()
            && self.session_pane_on_screen(&file.identity())
        {
            return;
        }

        let pane_id = self.session_pane_id(&file.identity());
        let location = redact_home(&file.cwd);
        crate::platform::notifications::show(&title, &location);
        // The global watcher fans this event into every open workspace. Only
        // the one tracking the terminal's process may relay it to the phone.
        if pane_id.is_some() {
            self.relay_presence_notice_to_phone(format!("{title}\n{location}"), cx);
        }
    }

    /// The pane bound to `session_id`, if this workspace owns its terminal.
    fn session_pane_id(&self, identity: &daruda_agent::StatusKey) -> Option<PaneId> {
        self.claude_sessions
            .pty_bindings
            .iter()
            .find(|(_, binding)| binding.identity() == *identity)
            .map(|(pane_id, _)| *pane_id)
    }

    /// `true` when the pane bound to `session_id` is on screen.
    /// Reverse-scans the bindings (debug-free, low frequency — fired
    /// once per blocking notification). `false` when the session has no
    /// live pane binding.
    fn session_pane_on_screen(&self, identity: &daruda_agent::StatusKey) -> bool {
        self.session_pane_id(identity)
            .is_some_and(|pane_id| self.pane_on_screen(pane_id))
    }

    /// Retire pane routes, mirrored status and unseen outcomes at every
    /// teardown edge. Known processes remain tracked until OS-confirmed exit.
    pub(in crate::workspace) fn release_pane_tracking(
        &mut self,
        pane_ids: &[PaneId],
        cx: &mut Context<Self>,
    ) {
        for id in pane_ids {
            crate::runs::RunsHost::release_execution(
                self.run_pane(*id),
                daruda_store::tasks::machine::UndeliveredReason::PaneClosed,
                cx,
            );
            crate::runs::RunsHost::unregister_terminal(self.run_pane(*id), cx);
            if let Some(binding) = self.claude_sessions.pty_bindings.remove(id) {
                self.forget_terminal_session(&binding.identity());
            }
        }
        self.unseen_outcomes.forget(pane_ids);
        // Dropped bindings feed the left-dock per-lane agent badges and
        // `agent_active_session_id`; a workspace render re-stages the dock
        // snapshot and the staging diff invalidates the `.cached()` dock. The
        // panes going away may also have been what the Dock badge counted.
        self.notify_status_docks(cx);
    }
}
