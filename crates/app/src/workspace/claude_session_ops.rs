//! Claude Code integration state owned by [`super::Workspace`].
//! Workspace methods that mutate or read `self.claude` live here.
//!
//! Groups the fields that together drive the right-panel Usage tab's
//! gauges + status pill, the per-lane session-status indicator, the
//! JSONL fallback watcher, the PTY tracker, and the plan-limits /
//! service-status polls. Workspace owns this struct directly (not as
//! an `Entity`) so the existing access patterns
//! (`self.claude.usage_by_account`, etc.) compile without
//! subscription / re-render plumbing changes — the goal at this stage is
//! **field grouping**, not actor isolation. A future refactor can
//! promote `ClaudeContext` to its own GPUI Entity once the call graph is
//! mapped.
//!
//! The `_*` prefixed fields are RAII guards: dropping the struct
//! cancels the matching pump tasks / shuts down the JSONL fallback
//! watcher.

use std::collections::HashMap;

use gpui::Task;

use daruda_store::accounts::{AccountRecipeId, AccountSelection};

use crate::hooks::pty_tracker::{PtyBinding, PtyTracker};
use crate::workspace::main_area::pane_tree::PaneId;

/// Which account's usage a cache entry belongs to. The auth domain is part of
/// the key because [`AccountSelection::SystemDefault`] names a *different*
/// login in each domain — the ambient Claude login and the ambient Codex login
/// would otherwise collide on one slot and overwrite each other every tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(in crate::workspace) struct UsageKey {
    pub recipe: AccountRecipeId,
    pub account: AccountSelection,
}

/// Per-account cache of the two usage quantities the Usage tab renders —
/// plan-rate limits and locally-aggregated activity. Encapsulates the maps
/// behind typed getters/setters so callers never reach into a raw `HashMap`;
/// the setters return whether a *visible* field changed, letting the caller
/// decide `cx.notify()` (keeps GPUI out of this plain-data type).
#[derive(Default)]
pub(in crate::workspace) struct PerAccountUsage {
    usage: HashMap<UsageKey, daruda_claude::UsageOutcome>,
    activity: HashMap<AccountSelection, daruda_claude::ActivityStats>,
}

impl PerAccountUsage {
    /// What the last poll established for `key`. `Pending` before the first
    /// one lands, so callers never have to treat "absent" separately.
    pub(in crate::workspace) fn usage(&self, key: UsageKey) -> daruda_claude::UsageOutcome {
        self.usage.get(&key).cloned().unwrap_or_default()
    }

    /// The cached activity aggregate for `key`, if any has landed.
    pub(in crate::workspace) fn activity(
        &self,
        key: AccountSelection,
    ) -> Option<&daruda_claude::ActivityStats> {
        self.activity.get(&key)
    }

    /// Fold a fetch result into `key`'s outcome; returns `true` when what is on
    /// screen changed, so only a meaningful change triggers a repaint. A
    /// snapshot's `fetched_at` moves every tick, so it is deliberately not part
    /// of the comparison — only the windows and the state are visible.
    pub(in crate::workspace) fn advance_usage(
        &mut self,
        key: UsageKey,
        result: Result<daruda_claude::ProviderUsage, daruda_claude::FetchError>,
    ) -> bool {
        let prev = self.usage(key);
        let next = prev.clone().advance(result);
        let visible_changed = prev.snapshot().map(|u| &u.windows)
            != next.snapshot().map(|u| &u.windows)
            || prev.is_stale() != next.is_stale()
            || prev.is_signed_out() != next.is_signed_out();
        self.usage.insert(key, next);
        visible_changed
    }

    /// Insert/replace the activity aggregate for `key`; returns `true` when
    /// it differs from the prior entry (a quiet tick that found no new JSONL
    /// lines returns `false`, so an idle pane doesn't repaint — Pitfall #10).
    pub(in crate::workspace) fn set_activity(
        &mut self,
        key: AccountSelection,
        activity: daruda_claude::ActivityStats,
    ) -> bool {
        if self.activity.get(&key) == Some(&activity) {
            return false;
        }
        self.activity.insert(key, activity);
        true
    }

    /// Drop every cached quantity for `account` across all domains — run when
    /// the account is deleted so no stale usage lingers under a dangling
    /// selection.
    pub(in crate::workspace) fn remove(&mut self, account: AccountSelection) {
        self.usage.retain(|key, _| key.account != account);
        self.activity.remove(&account);
    }
}

pub(in crate::workspace) struct ClaudeContext {
    /// Background-poll cadences for the OAuth `/api/oauth/usage` and
    /// public `status.claude.com` endpoints. Stored as a snapshot of
    /// `[usage.poll]` so `limits_pump` can read it under
    /// `read_with` without locking the full `Config`.
    pub(in crate::workspace) usage_poll: daruda_config::PollConfig,

    /// Per-account cache of the Usage tab's two quantities (plan-rate
    /// limits + locally-aggregated activity), keyed by [`UsageKey`].
    /// Empty before the first fetch/aggregation, which reads back as
    /// `UsageOutcome::Pending`. Reads/writes go through [`PerAccountUsage`]'s
    /// typed getters/setters (`advance_usage` / `set_activity_stats` decide
    /// `cx.notify()` from the returned "visible changed" flag).
    pub(in crate::workspace) usage_by_account: PerAccountUsage,

    /// Latest service-status indicator per auth domain. Account-independent —
    /// a status page reports its provider's health, not a per-account
    /// quantity — but not domain-independent: each provider hosts its own
    /// page. Absent before that domain's first fetch lands;
    /// `set_service_status` updates and bumps `cx.notify()`.
    pub(in crate::workspace) service_status: HashMap<AccountRecipeId, daruda_claude::ServiceStatus>,

    /// Guards `refresh_usage_now` against overlapping manual refreshes —
    /// the button's one-shot fetch sets this on entry and clears it when
    /// the background fetch resolves, so a double-click can't fan out
    /// two concurrent `/api/oauth/usage` round-trips.
    pub(in crate::workspace) usage_refresh_in_flight: bool,

    /// Claude Code session-status mirror — driven by the hook channel
    /// (and Phase B jsonl fallback). Read by the left dock to render the
    /// per-lane Working/NeedsAttention/Idle/Connecting indicator.
    pub(in crate::workspace) claude_status: daruda_claude::ClaudeStatusStore,

    /// Whether the Claude status feature is enabled in `[claude_status]`
    /// config. False suppresses both the indicator and the install banner.
    pub(in crate::workspace) claude_status_enabled: bool,

    /// `[claude_status] stale_threshold_secs` mirror — the same age past
    /// which cold restore resets a session also expires its blocking
    /// notification for the desktop-push gate (see
    /// `maybe_push_hook_notification`). Updated in `apply_config`.
    pub(in crate::workspace) stale_threshold_secs: u64,

    /// Whether daruda's hook entries are present in
    /// `~/.claude/settings.json`. Cached: refreshed at startup and after
    /// each install/uninstall action; on disk changes by the user
    /// directly editing settings.json the cache will lag until daruda
    /// restarts.
    pub(in crate::workspace) claude_hooks_installed: bool,

    /// PTY → claude session tracker. Polls sysinfo every 3 s to find
    /// `claude` descendants of each registered Pane's shell PID and
    /// resolves their session_id via `~/.claude/sessions/<pid>.json`.
    pub(in crate::workspace) pty_tracker: PtyTracker,

    /// Last reported `(PaneId → claude session)` bindings from the
    /// tracker. Used by sub-row badges to highlight the focused tab's
    /// session and by `apply_pty_tracker_event` to detect dead sessions.
    pub(in crate::workspace) pty_claude_bindings: HashMap<PaneId, PtyBinding>,

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

    /// Last blocking-notification timestamp already surfaced as a
    /// desktop push, per session. The status-dir watcher can re-deliver
    /// the same file (FSEvents coalescing / duplicate events) and every
    /// hook write replaces the store entry, so neither the store nor the
    /// `update` return value dedups for us. Fire a push only when the
    /// incoming notification timestamp is newer than the one recorded
    /// here. Pruned alongside the session on removal.
    pub(in crate::workspace) last_pushed_notification:
        HashMap<String, chrono::DateTime<chrono::Utc>>,

    /// Background-poll tasks driving the Usage tab: plan-rate windows and
    /// service status for each auth domain, plus the local JSONL activity
    /// aggregation. Each task re-reads the workspace's poll cadence every tick
    /// so live config edits flow without restart. Dropping the tasks (when
    /// Workspace is dropped) cancels the loops; held in one field so the set
    /// grows with the domain list rather than the struct.
    #[allow(dead_code)]
    pub(in crate::workspace) _limits_pumps: Vec<Task<()>>,
}

// ---- Workspace methods that own the claude field ----

use crate::workspace::Workspace;
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::system_info::redact_home;
use gpui::Context;

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
            StatusEvent::Changed(path) => match daruda_claude::hooks::status_file::read(&path) {
                Ok(Some(file)) => {
                    self.apply_task_session_changed(&file.cwd, &file.session_id, cx);
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
                        self.claude.tool_use_failure_counts.remove(&file.session_id);
                        self.apply_task_session_ended(&file.session_id, reason, cx);
                    }
                    // Blocking hook notifications surface as a one-shot
                    // desktop push here rather than latching the lane
                    // indicator (see `daruda_claude::hooks::fsm`). Called
                    // before `update` consumes `file`.
                    self.maybe_push_hook_notification(&file);
                    #[cfg(debug_assertions)]
                    let dbg_probe = self.probe_lane_status(&file.session_id);
                    #[cfg(debug_assertions)]
                    let dbg_fields = (
                        file.session_id.clone(),
                        file.cwd.clone(),
                        file.last_event.clone(),
                        file.source,
                    );
                    if self.claude.claude_status.update(file) {
                        cx.notify();
                        #[cfg(debug_assertions)]
                        {
                            let (sid, cwd, event, source) = dbg_fields;
                            self.log_lane_status_change(dbg_probe, &sid, &cwd, &event, source);
                        }
                    }
                }
                Ok(None) => {
                    // Mid-write or malformed; skip silently.
                }
                Err(e) => {
                    let report = ErrorReport::new("Claude status file read failed")
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
                self.claude.tool_use_failure_counts.remove(&session_id);
                self.claude.last_pushed_notification.remove(&session_id);
                self.apply_task_session_ended(
                    &session_id,
                    daruda_store::tasks::SessionEndReason::Other,
                    cx,
                );
                #[cfg(debug_assertions)]
                let dbg_entry = self
                    .claude
                    .claude_status
                    .get(&session_id)
                    .map(|f| (f.cwd.clone(), f.source));
                #[cfg(debug_assertions)]
                let dbg_probe = dbg_entry
                    .as_ref()
                    .map(|_| self.probe_lane_status(&session_id));
                if self.claude.claude_status.remove(&session_id).is_some() {
                    cx.notify();
                    #[cfg(debug_assertions)]
                    if let (Some((cwd, source)), Some(probe)) = (dbg_entry, dbg_probe) {
                        self.log_lane_status_change(probe, &session_id, &cwd, "removed", source);
                    }
                }
            }
        }
    }

    /// Raise a transient desktop push for a blocking Claude
    /// `Notification` (permission prompt / idle prompt / elicitation
    /// dialog). These do not latch the lane indicator into a
    /// persistent `NeedsAttention` (see `daruda_claude::hooks::fsm`), so
    /// this one-shot push is their only surface.
    ///
    /// Gated by the `hook_notification_enabled` config and the shared
    /// "skip the focused pane" rule (the user is already looking at the
    /// TUI prompt), and deduped by timestamp so a status file the
    /// watcher re-delivers does not double-fire.
    fn maybe_push_hook_notification(
        &mut self,
        file: &daruda_claude::hooks::status_file::StatusFile,
    ) {
        use crate::surface::strings as s;
        use daruda_claude::hooks::events::NotificationType;

        let title = match file.notification {
            Some(NotificationType::PermissionPrompt) => s::notification_hook_permission_title(),
            Some(NotificationType::IdlePrompt) => s::notification_hook_idle_title(),
            Some(NotificationType::ElicitationDialog) => s::notification_hook_elicitation_title(),
            // Informational subtypes and non-notification events never push.
            _ => return,
        };

        if !self.notifications.hook_notification_enabled {
            return;
        }

        // Freshness gate — a notification re-delivered past the stale
        // threshold (cold-restore rewrite by another instance, watcher
        // replay of an old file) is no longer actionable; pushing it
        // would tell the user to answer a prompt that is long gone.
        if file.event_expired(
            chrono::Utc::now(),
            std::time::Duration::from_secs(self.claude.stale_threshold_secs),
        ) {
            return;
        }

        // Dedup: skip when this notification is not strictly newer than
        // the last one already surfaced for the session.
        if self
            .claude
            .last_pushed_notification
            .get(&file.session_id)
            .is_some_and(|&t| file.timestamp <= t)
        {
            return;
        }
        // Record before the focus-gate return below: a re-delivered file
        // that was suppressed by focus must not fire late once focus
        // moves away — the user already saw the TUI prompt.
        self.claude
            .last_pushed_notification
            .insert(file.session_id.clone(), file.timestamp);

        // Focus gate — silence when daruda is foreground and the
        // session's own pane is focused, mirroring `handle_view_event`.
        if self.notifications.skip_focused_pane
            && crate::platform::attention::is_app_active()
            && self.session_pane_is_focused(&file.session_id)
        {
            return;
        }

        crate::platform::notifications::show(&title, &redact_home(&file.cwd));
    }

    /// `true` when the pane bound to `session_id` is the focused pane.
    /// Reverse-scans the bindings (debug-free, low frequency — fired
    /// once per blocking notification). `false` when the session has no
    /// live pane binding.
    fn session_pane_is_focused(&self, session_id: &str) -> bool {
        self.claude
            .pty_claude_bindings
            .iter()
            .find(|(_, b)| b.session_id == session_id)
            .is_some_and(|(pane_id, _)| *pane_id == self.active_runtime().focused_pane_id)
    }

    /// Fold a plan-rate fetch result into `key`'s cached outcome. Called by the
    /// limits pump and by `refresh_usage_now` — including on failure, since
    /// which failure it was is what tells a signed-out domain from a domain
    /// whose refresh merely broke. Skips `cx.notify()` when nothing visible
    /// moved.
    pub(in crate::workspace) fn advance_usage(
        &mut self,
        key: UsageKey,
        result: Result<daruda_claude::ProviderUsage, daruda_claude::FetchError>,
        cx: &mut Context<Self>,
    ) {
        if self.claude.usage_by_account.advance_usage(key, result) {
            cx.notify();
        }
    }

    /// Replace one provider's cached service-status snapshot. Called by the
    /// status pump after a successful fetch of that provider's status page.
    pub(in crate::workspace) fn set_service_status(
        &mut self,
        recipe: AccountRecipeId,
        status: daruda_claude::ServiceStatus,
        cx: &mut Context<Self>,
    ) {
        let prev = self.claude.service_status.get(&recipe);
        let visible_changed = prev.is_none_or(|prev| {
            prev.indicator != status.indicator || prev.description != status.description
        });
        self.claude.service_status.insert(recipe, status);
        if visible_changed {
            cx.notify();
        }
    }

    /// Replace the cached activity aggregate for `key` (the account it was
    /// aggregated for; [`AccountSelection::SystemDefault`] = system default).
    /// Called by the `Activity` pump and by `refresh_usage_now` after
    /// `update_activity` lands. Skips the redraw when the stats are unchanged
    /// vs. that account's prior entry (a quiet tick that found no new JSONL
    /// lines) so an idle pane doesn't repaint the whole window tree
    /// (Pitfall #10).
    pub(in crate::workspace) fn set_activity_stats(
        &mut self,
        key: AccountSelection,
        activity: daruda_claude::ActivityStats,
        cx: &mut Context<Self>,
    ) {
        if self.claude.usage_by_account.set_activity(key, activity) {
            cx.notify();
        }
    }

    /// Manual-refresh backend for the Usage tab's ⟳ button. Re-fetches
    /// all three usage sources (plan limits, service status, local
    /// activity) once off the GPUI thread and forwards each into its
    /// existing setter, keyed by the focused pane's account (mirroring
    /// the background pump — see `sync::limits::spawn_loop`). The
    /// `usage_refresh_in_flight` guard collapses rapid clicks into a
    /// single in-flight round-trip; it is cleared when the fetch
    /// resolves (or with the workspace if it is gone).
    pub(in crate::workspace) fn refresh_usage_now(&mut self, cx: &mut Context<Self>) {
        if self.claude.usage_refresh_in_flight {
            return;
        }
        self.claude.usage_refresh_in_flight = true;
        cx.notify();

        // The focused pane's domain decides which sources apply, exactly as
        // the pump resolves it — otherwise this path fetches Anthropic's
        // endpoints for any focused account and files the result under that
        // account's key.
        let domain = crate::workspace::main_area::pane::AccountDomain::for_pane(
            &self.focused_account_pane(cx),
        );
        let focused = self.focused_account();
        let recipe = focused.recipe(domain);
        let account_key = focused.key();
        let config_dir = focused.into_config_dir();

        cx.spawn(async move |this, cx| {
            let (limits, status, activity) = cx
                .background_executor()
                .spawn(async move {
                    crate::workspace::sync::limits::fetch_all_for(recipe, config_dir.as_deref())
                })
                .await;

            // The workspace can close mid-refresh; nothing to update or
            // unset then, so the dropped guard dies with the entity.
            // SILENT-OK: workspace gone mid-refresh — no state to clear.
            let _ = this.update(cx, |ws, cx| {
                // Failures are forwarded too: which failure it was is what
                // distinguishes a signed-out domain from a broken refresh.
                ws.advance_usage(
                    UsageKey {
                        recipe,
                        account: account_key,
                    },
                    limits,
                    cx,
                );
                if let Ok(s) = status {
                    ws.set_service_status(recipe, s, cx);
                }
                if let Some(a) = activity {
                    ws.set_activity_stats(account_key, a, cx);
                }
                ws.claude.usage_refresh_in_flight = false;
                cx.notify();
            });
        })
        .detach();
    }

    /// Drop every trace of `pane_ids` from PTY→claude tracking: the
    /// tracker stops walking their shell PIDs (keeping its idle guard
    /// able to re-arm once nothing is registered) and their bindings
    /// leave the map immediately instead of waiting for the next
    /// poll's vanished-pane diff. Every pane-teardown path (close
    /// pane / close tab / remove lane / close project) goes through
    /// this single release point.
    pub(in crate::workspace) fn release_pane_tracking(
        &mut self,
        pane_ids: &[PaneId],
        cx: &mut Context<Self>,
    ) {
        for id in pane_ids {
            self.claude.pty_tracker.unregister(*id);
            self.claude.pty_claude_bindings.remove(id);
        }
        // Dropped bindings feed the left-dock per-lane agent badges and
        // `agent_active_session_id`; a workspace render re-stages the dock
        // snapshot and the staging diff invalidates the `.cached()` dock.
        cx.notify();
    }
}
