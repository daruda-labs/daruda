//! Claude-specific hooks, session status, PTY tracking, and JSONL fallback.
//! Dropping the task and watcher guards stops their background pumps.

use std::collections::HashMap;

use gpui::{Context, Task};

use crate::workspace::Workspace;
use crate::workspace::main_area::pane_tree::PaneId;
use daruda_agent::pty_tracker::{PtyBinding, PtyTracker};
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::system_info::redact_home;

pub(in crate::workspace) struct ClaudeSessionState {
    /// Claude Code session-status mirror — driven by the hook channel and
    /// JSONL fallback. Read by the left dock to render the per-lane
    /// Working/NeedsAttention/Idle/Connecting indicator.
    pub(in crate::workspace) status: daruda_agent::ClaudeStatusStore,

    /// Whether daruda's hook entries are present in
    /// `~/.claude/settings.json`. Cached: refreshed at startup and after
    /// each install/uninstall action; on disk changes by the user
    /// directly editing settings.json the cache will lag until daruda
    /// restarts.
    pub(in crate::workspace) hooks_installed: bool,

    /// PTY → claude session tracker. Polls sysinfo every 3 s to find
    /// `claude` descendants of each registered Pane's shell PID and
    /// resolves their session_id via `~/.claude/sessions/<pid>.json`.
    pub(in crate::workspace) pty_tracker: PtyTracker,

    /// Last reported `(PaneId → claude session)` bindings from the
    /// tracker. Used by sub-row badges to highlight the focused tab's
    /// session and by `apply_pty_tracker_event` to detect dead sessions.
    pub(in crate::workspace) pty_bindings: HashMap<PaneId, PtyBinding>,

    /// Keeps the tracker event-pump task alive for the Workspace
    /// lifetime; dropped when Workspace is dropped, which lets the
    /// tracker thread exit cleanly via the receiver-disconnect probe.
    #[allow(dead_code)]
    pub(in crate::workspace) _pty_event_pump: Task<()>,

    /// JSONL fallback watcher's shutdown half — engaged only when
    /// the user has not installed the hook integration. The watcher
    /// thread reads `~/.claude/projects/<encoded(cwd)>/` per lane
    /// and feeds `JsonlEvent`s through the same store as hooks (with
    /// `Source::Jsonl`). `None` when the claude_status feature is
    /// disabled or no lane is active. Dropping this handle stops the
    /// watcher (FSEvents unregisters); `refresh_jsonl_watcher`
    /// reassigns it when install state or the lane set changes.
    pub(in crate::workspace) _jsonl_watcher: Option<crate::dir_watch::DirWatcher>,
    pub(in crate::workspace) _jsonl_event_pump: Option<Task<()>>,

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
        HashMap<String, chrono::DateTime<chrono::Utc>>,
    /// Turns the hook channel has seen open, so a `Stop` can be told from
    /// a re-delivered one. Fed by hook updates only; see `turn_end`.
    pub(in crate::workspace) open_turns: daruda_agent::hooks::turn_end::OpenTurns,
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
                    self.record_task_cli_transcript(
                        &file.session_id,
                        file.transcript_path.as_deref(),
                        cx,
                    );
                    // A session starting or ending is the moment its process
                    // may have appeared or gone; a tool call's update is not.
                    if matches!(file.last_event.as_str(), "SessionStart" | "SessionEnd") {
                        self.claude_sessions.pty_tracker.poke();
                    }
                    self.apply_task_session_changed(&file.session_id, cx);
                    if file.last_event == "PostToolUseFailure" {
                        self.bump_tool_use_failure(&file.session_id, cx);
                    }
                    if file.last_event == "PostToolUse"
                        && file.tool_name.as_deref() == Some("TodoWrite")
                        && let Some(input) = file.tool_input.as_ref()
                    {
                        self.apply_todo_write(&file.session_id, input, cx);
                    }
                    if let Some(reason) = Self::classify_hook_end_reason(&file.last_event) {
                        self.claude_sessions
                            .tool_use_failure_counts
                            .remove(&file.session_id);
                        self.apply_task_session_ended(&file.session_id, reason, cx);
                    }
                    // Blocking hook notifications surface as a one-shot
                    // local + remote push here rather than latching the lane
                    // indicator (see `daruda_agent::hooks::fsm`). Called
                    // before `update` consumes `file`.
                    self.maybe_push_hook_notification(&file, cx);
                    #[cfg(debug_assertions)]
                    let dbg_probe = self.probe_lane_status(&file.session_id);
                    #[cfg(debug_assertions)]
                    let dbg_fields = (
                        file.session_id.clone(),
                        file.cwd.clone(),
                        file.last_event.clone(),
                        file.source,
                    );
                    let turn_end = self
                        .claude_sessions
                        .open_turns
                        .observe(&file.session_id, file.status, &file.last_event)
                        .zip(self.session_pane_id(&file.session_id));
                    if self.claude_sessions.status.update(file) {
                        self.notify_status_docks(cx);
                        #[cfg(debug_assertions)]
                        {
                            let (sid, cwd, event, source) = dbg_fields;
                            self.log_lane_status_change(dbg_probe, &sid, &cwd, &event, source);
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
            StatusEvent::Removed { session_id, .. } => {
                self.claude_sessions.pty_tracker.poke();
                self.claude_sessions
                    .tool_use_failure_counts
                    .remove(&session_id);
                self.claude_sessions
                    .last_pushed_notification
                    .remove(&session_id);
                self.claude_sessions.open_turns.forget(&session_id);
                self.apply_task_session_ended(
                    &session_id,
                    daruda_store::tasks::SessionEndReason::Other,
                    cx,
                );
                #[cfg(debug_assertions)]
                let dbg_entry = self
                    .claude_sessions
                    .status
                    .get(&session_id)
                    .map(|f| (f.cwd.clone(), f.source));
                #[cfg(debug_assertions)]
                let dbg_probe = dbg_entry
                    .as_ref()
                    .map(|_| self.probe_lane_status(&session_id));
                if self.claude_sessions.status.remove(&session_id).is_some() {
                    self.notify_status_docks(cx);
                    #[cfg(debug_assertions)]
                    if let (Some((cwd, source)), Some(probe)) = (dbg_entry, dbg_probe) {
                        self.log_lane_status_change(probe, &session_id, &cwd, "removed", source);
                    }
                }
            }
        }
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
            .get(&file.session_id)
            .is_some_and(|&t| file.timestamp <= t)
        {
            return;
        }
        // Record before the focus-gate return below: a re-delivered file
        // that was suppressed by focus must not fire late once focus
        // moves away — the user already saw the TUI prompt.
        self.claude_sessions
            .last_pushed_notification
            .insert(file.session_id.clone(), file.timestamp);

        // In-view gate — silence when daruda is foreground and the
        // session's own pane is on screen, mirroring `handle_view_event`.
        if self.mirrors.notifications.skip_focused_pane
            && crate::platform::attention::is_app_active()
            && self.session_pane_on_screen(&file.session_id)
        {
            return;
        }

        let pane_id = self.session_pane_id(&file.session_id);
        let location = redact_home(&file.cwd);
        crate::platform::notifications::show(&title, &location);
        // The global watcher fans this event into every open workspace. Only
        // the one tracking the terminal's process may relay it to the phone.
        if pane_id.is_some() {
            self.relay_presence_notice_to_phone(format!("{title}\n{location}"), cx);
        }
    }

    /// The pane bound to `session_id`, if this workspace owns its terminal.
    fn session_pane_id(&self, session_id: &str) -> Option<PaneId> {
        self.claude_sessions
            .pty_bindings
            .iter()
            .find(|(_, binding)| binding.session_id == session_id)
            .map(|(pane_id, _)| *pane_id)
    }

    /// `true` when the pane bound to `session_id` is on screen.
    /// Reverse-scans the bindings (debug-free, low frequency — fired
    /// once per blocking notification). `false` when the session has no
    /// live pane binding.
    fn session_pane_on_screen(&self, session_id: &str) -> bool {
        self.session_pane_id(session_id)
            .is_some_and(|pane_id| self.pane_on_screen(pane_id))
    }

    /// Drop every trace of `pane_ids` from PTY→claude tracking: the
    /// tracker stops walking their shell PIDs (keeping its idle guard
    /// able to re-arm once nothing is registered) and their bindings
    /// leave the map immediately instead of waiting for the next
    /// poll's vanished-pane diff. Also drops their unseen turn outcomes,
    /// chat panes' included, since this is the one point every
    /// pane-teardown path (close pane / close tab / remove lane / close
    /// project) goes through.
    pub(in crate::workspace) fn release_pane_tracking(
        &mut self,
        pane_ids: &[PaneId],
        cx: &mut Context<Self>,
    ) {
        for id in pane_ids {
            self.claude_sessions.pty_tracker.unregister(*id);
            self.claude_sessions.pty_bindings.remove(id);
        }
        self.unseen_outcomes.forget(pane_ids);
        // Dropped bindings feed the left-dock per-lane agent badges and
        // `agent_active_session_id`; a workspace render re-stages the dock
        // snapshot and the staging diff invalidates the `.cached()` dock. The
        // panes going away may also have been what the Dock badge counted.
        self.notify_status_docks(cx);
    }
}
