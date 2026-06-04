//! Claude Code integration state owned by [`super::Workspace`].
//! Workspace methods that mutate or read `self.claude` live here.
//!
//! Groups the 18 fields that together drive the right-panel Usage tab,
//! the per-lane session-status indicator, the JSONL fallback
//! watcher, the PTY tracker, and the plan-limits / service-status
//! polls. Workspace owns this struct directly (not as an `Entity`) so
//! the existing access patterns (`self.claude.usage`, etc.) compile
//! without subscription / re-render plumbing changes — the goal at
//! this stage is **field grouping**, not actor isolation. A future
//! refactor can promote `ClaudeContext` to its own GPUI Entity once
//! the call graph is mapped.
//!
//! The `_*` prefixed fields are RAII guards: dropping the struct
//! cancels the matching pump tasks / shuts down the JSONL fallback
//! watcher.

use std::collections::HashMap;
use std::sync::mpsc;

use gpui::{Entity, Subscription, Task};

use crate::hooks::pty_tracker::{PtyBinding, PtyTracker};
use crate::ui::select::SelectState;
use crate::workspace::main_area::pane_tree::PaneId;

pub(in crate::workspace) struct ClaudeContext {
    /// Token-usage accumulator for the right dock's Usage tab. Folded
    /// per-session by `apply_claude_jsonl_event` whenever the JSONL
    /// watcher surfaces new assistant entries with a `usage` block.
    /// Not persisted — the watcher's first fire after each launch
    /// re-emits the entire session history, providing automatic cold
    /// restore.
    pub(in crate::workspace) usage: daruda_claude::usage::UsageState,

    /// Per-million-token pricing applied when rendering the Usage
    /// tab's cost columns. Derived from `[usage.pricing]` at construct
    /// time and refreshed by `apply_config` so live config edits flow
    /// to the next frame without re-reading TOML in the render path.
    pub(in crate::workspace) usage_pricing: daruda_claude::usage::UsagePricing,

    /// Background-poll cadences for the OAuth `/api/oauth/usage` and
    /// public `status.claude.com` endpoints. Stored as a snapshot of
    /// `[usage.poll]` so `limits_pump` can read it under
    /// `read_with` without locking the full `Config`.
    pub(in crate::workspace) usage_poll: daruda_config::PollConfig,

    /// Latest plan-rate response (5-hour + 7-day windows). Default-
    /// constructed before the first successful fetch — gauges then
    /// render in their placeholder state. `set_plan_limits` updates
    /// this and bumps `cx.notify()`.
    pub(in crate::workspace) plan_limits: daruda_claude::PlanLimits,

    /// Latest service-status indicator from `status.claude.com`.
    /// Default-constructed (`Unknown`) before the first fetch lands;
    /// `set_service_status` updates and bumps `cx.notify()`.
    pub(in crate::workspace) service_status: daruda_claude::ServiceStatus,

    /// Time window applied when rendering the Usage tab — sessions
    /// older than the window are hidden and the Total summary
    /// aggregates only sessions inside it. Persisted via
    /// `ProjectState::active_usage_window` (default = `Last7d`).
    pub(in crate::workspace) usage_window: daruda_store::project::UsageWindow,

    /// Picker rendered at the top of the Usage tab. Workspace owns
    /// the entity so selection survives dock teardown and
    /// `restore_state` can resync the picker after the saved
    /// `usage_window` is reapplied.
    pub(in crate::workspace) usage_select: Entity<SelectState>,

    /// Keeps the `usage_select` subscription alive for the lifetime
    /// of the Workspace; dropping `Self` cancels the closure.
    #[allow(dead_code)]
    pub(in crate::workspace) _usage_select_subscription: Subscription,

    /// Claude Code session-status mirror — driven by the hook channel
    /// (and Phase B jsonl fallback). Read by the left dock to render the
    /// per-lane Working/NeedsAttention/Idle/Connecting indicator.
    pub(in crate::workspace) claude_status: daruda_claude::ClaudeStatusStore,

    /// Whether the Claude status feature is enabled in `[claude_status]`
    /// config. False suppresses both the indicator and the install banner.
    pub(in crate::workspace) claude_status_enabled: bool,

    /// Whether daruda's hook entries are present in
    /// `~/.claude/settings.json`. Cached: refreshed at startup and after
    /// each install/uninstall action; on disk changes by the user
    /// directly editing settings.json the cache will lag until daruda
    /// restarts (acceptable — Phase C-1 adds a settings.json watch).
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
    /// `Source::Jsonl`). `None` when hooks are installed (or the
    /// claude_status feature is disabled). Dropping this sender
    /// shuts the watcher thread down via its shutdown channel;
    /// `refresh_jsonl_watcher` reassigns this when install state or
    /// the lane set changes.
    pub(in crate::workspace) _jsonl_watcher_shutdown: Option<mpsc::Sender<()>>,
    pub(in crate::workspace) _jsonl_event_pump: Option<Task<()>>,

    /// Running count of `PostToolUseFailure` hook events per Claude
    /// session. When a session crosses
    /// `daruda_store::tasks::TASK_TOOL_USE_FAILURE_THRESHOLD` the owning task
    /// is escalated to `Error { message: "tool_use_failure xN" }`
    /// (R-15 Phase 2). Cleared whenever a session is removed or
    /// transitioned out of `Running`.
    pub(in crate::workspace) tool_use_failure_counts: HashMap<String, u32>,

    /// Two background-poll tasks for `[usage.poll]` —
    /// `/api/oauth/usage` (plan-rate windows) and
    /// `status.claude.com` (service status). Each task re-reads the
    /// workspace's poll cadence every tick so live config edits flow
    /// without restart. Dropping the tasks (when Workspace is
    /// dropped) cancels the loops; held in a tuple so a single field
    /// owns both.
    #[allow(dead_code)]
    pub(in crate::workspace) _limits_pumps: (Task<()>, Task<()>),
}

// ---- Workspace methods that own the claude field ----

use crate::workspace::Workspace;
use daruda_claude::SessionStatus;
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::system_info::redact_home;
use gpui::{Context, Window};

// `Path` and `Source` are only named by the debug-only transition logger
// (and `Source` also by the always-compiled test fixtures), so gate the
// imports to those profiles to avoid an unused-import warning in release
// non-test builds.
#[cfg(any(debug_assertions, test))]
use daruda_claude::hooks::status_file::Source;
#[cfg(debug_assertions)]
use daruda_store::observability::log_writer::LogWriter;
#[cfg(debug_assertions)]
use std::path::Path;

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

    pub(in crate::workspace) fn set_usage_window(
        &mut self,
        value: daruda_store::project::UsageWindow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.claude.usage_window == value {
            return;
        }
        self.mutate_durable_in(window, cx, |ws, window, cx| {
            ws.claude.usage_window = value;
            let slug = gpui::SharedString::from(value.slug());
            ws.claude.usage_select.update(cx, |s, cx_inner| {
                s.set_selected_value(&slug, window, cx_inner)
            });
        });
        cx.notify();
    }

    /// Replace the cached plan-rate snapshot. Called by `limits_pump`
    /// after a successful `/api/oauth/usage` fetch. Skips `cx.notify()`
    /// when only `fetched_at` moved.
    pub(in crate::workspace) fn set_plan_limits(
        &mut self,
        limits: daruda_claude::PlanLimits,
        cx: &mut Context<Self>,
    ) {
        let visible_changed = self.claude.plan_limits.five_hour != limits.five_hour
            || self.claude.plan_limits.seven_day != limits.seven_day;
        self.claude.plan_limits = limits;
        if visible_changed {
            cx.notify();
        }
    }

    /// Replace the cached service-status snapshot. Called by
    /// `limits_pump` after a successful `status.claude.com` fetch.
    pub(in crate::workspace) fn set_service_status(
        &mut self,
        status: daruda_claude::ServiceStatus,
        cx: &mut Context<Self>,
    ) {
        let visible_changed = self.claude.service_status.indicator != status.indicator
            || self.claude.service_status.description != status.description;
        self.claude.service_status = status;
        if visible_changed {
            cx.notify();
        }
    }
}

// ── Claude status pane aggregation ────────────────────────────────────
//
// `aggregate_over_panes` is the single source of truth for the left-dock
// leading indicator's aggregate, shared with the render snapshot
// (`render::snapshots`'s `claude_status_per_lane` /
// `claude_per_session_per_lane`). The debug-only probe / logger sit on top
// of it so the on-disk NDJSON log can be diffed against the rendered
// indicator when verifying status transitions.

/// One pane's resolved Claude session and its current status. The unit of
/// per-lane sub-row rendering — a lane's panes contribute these in layout
/// order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::workspace) struct PaneSessionStatus {
    pub(in crate::workspace) pane_id: PaneId,
    pub(in crate::workspace) session_id: String,
    pub(in crate::workspace) status: SessionStatus,
}

/// Aggregate Claude status over panes, attributing each pane's session to
/// the lane that owns the pane (not the session's cwd).
///
/// `pane_lane` lists every pane in layout order paired with its owning
/// lane. For each pane: a missing binding means the pane has no Claude
/// session; a `session_id` absent from `store` is omitted entirely (it is
/// not surfaced as `Connecting`).
///
/// Returns:
/// - per-lane leading indicator: the highest-priority status among the
///   lane's panes' sessions (only lanes with ≥ 1 such session appear).
/// - per-lane sub-rows: the lane's `PaneSessionStatus` list in pane order,
///   only for lanes with ≥ 2 sessions (single-session lanes are fully
///   described by the leading indicator).
pub(in crate::workspace) fn aggregate_over_panes(
    pane_lane: &[(PaneId, daruda_store::project::LaneRef)],
    bindings: &HashMap<PaneId, PtyBinding>,
    store: &daruda_claude::ClaudeStatusStore,
) -> (
    HashMap<daruda_store::project::LaneRef, SessionStatus>,
    HashMap<daruda_store::project::LaneRef, Vec<PaneSessionStatus>>,
) {
    let mut per_lane_sessions: HashMap<daruda_store::project::LaneRef, Vec<PaneSessionStatus>> =
        HashMap::new();
    for (pane_id, lane_ref) in pane_lane {
        let Some(binding) = bindings.get(pane_id) else {
            continue;
        };
        let Some(file) = store.get(&binding.session_id) else {
            continue;
        };
        per_lane_sessions
            .entry(*lane_ref)
            .or_default()
            .push(PaneSessionStatus {
                pane_id: *pane_id,
                session_id: binding.session_id.clone(),
                status: file.status,
            });
    }

    let per_lane_status = per_lane_sessions
        .iter()
        .filter_map(|(lane_ref, sessions)| {
            sessions
                .iter()
                .map(|ps| ps.status)
                .max_by_key(|s| s.priority())
                .map(|s| (*lane_ref, s))
        })
        .collect();

    per_lane_sessions.retain(|_, sessions| sessions.len() >= 2);

    (per_lane_status, per_lane_sessions)
}

impl Workspace {
    /// Every pane in the workspace paired with its owning lane, in layout
    /// order. The active lane's panes come from the live `main_area`
    /// tabs; every inactive lane's panes come from its frozen runtime.
    /// Feeds [`aggregate_over_panes`] so status attribution follows pane
    /// membership rather than session cwd.
    pub(in crate::workspace) fn pane_lane_index(
        &self,
    ) -> Vec<(PaneId, daruda_store::project::LaneRef)> {
        let mut index = Vec::new();
        for tab in &self.main_area.tabs {
            for pane_id in tab.layout.pane_ids() {
                index.push((pane_id, self.active));
            }
        }
        for (lane_ref, runtime) in &self.main_area.inactive_lane_runtimes {
            for tab in &runtime.tabs {
                for pane_id in tab.layout.pane_ids() {
                    index.push((pane_id, *lane_ref));
                }
            }
        }
        index
    }
}

/// Pre-mutation snapshot of one lane's status, captured so a transition
/// can be detected after the store mutates.
#[cfg(debug_assertions)]
pub(in crate::workspace) struct LaneStatusProbe {
    /// The mutated session's own stored status (if present).
    session: Option<SessionStatus>,
    /// The lane's displayed leading-indicator aggregate.
    aggregate: Option<SessionStatus>,
}

#[cfg(debug_assertions)]
impl Workspace {
    /// Capture the session's stored status and its lane aggregate.
    ///
    /// The lane is resolved from the session's pane: scan the bindings for
    /// the pane bound to `session_id`, then locate that pane in
    /// [`Workspace::pane_lane_index`]. The reverse scan is `O(panes)` and
    /// debug-only / low-frequency, so it is not indexed.
    pub(in crate::workspace) fn probe_lane_status(&self, session_id: &str) -> LaneStatusProbe {
        let aggregate = self
            .claude
            .pty_claude_bindings
            .iter()
            .find(|(_, binding)| binding.session_id == session_id)
            .map(|(pane_id, _)| *pane_id)
            .and_then(|pane_id| {
                let index = self.pane_lane_index();
                let lane_ref = index.iter().find(|(p, _)| *p == pane_id).map(|(_, l)| *l)?;
                let (per_lane, _) = aggregate_over_panes(
                    &index,
                    &self.claude.pty_claude_bindings,
                    &self.claude.claude_status,
                );
                // `None` when no pane in the lane has a store entry yet
                // (e.g. a brand-new session before its first hook write) —
                // logged as "(none)", matching the rendered indicator.
                per_lane.get(&lane_ref).copied()
            });
        LaneStatusProbe {
            session: self.claude.claude_status.get(session_id).map(|f| f.status),
            aggregate,
        }
    }

    /// Emit one Info NDJSON line per level (session / lane indicator)
    /// that changed since `before`. Log-only — never a toast — so the
    /// frequent tool-call transitions don't flood the UI; verify against
    /// the rendered indicator via the on-disk log.
    pub(in crate::workspace) fn log_lane_status_change(
        &self,
        before: LaneStatusProbe,
        session_id: &str,
        cwd: &Path,
        last_event: &str,
        source: Source,
    ) {
        let after = self.probe_lane_status(session_id);
        if before.session != after.session {
            LogWriter::log(session_transition_report(
                before.session,
                after.session,
                session_id,
                cwd,
                last_event,
                source,
            ));
        }
        if before.aggregate != after.aggregate {
            LogWriter::log(aggregate_transition_report(
                before.aggregate,
                after.aggregate,
                cwd,
            ));
        }
    }
}

#[cfg(debug_assertions)]
fn status_label(status: Option<SessionStatus>) -> &'static str {
    match status {
        Some(SessionStatus::Working) => "Working",
        Some(SessionStatus::ExecutingTool) => "ExecutingTool",
        Some(SessionStatus::NeedsAttention) => "NeedsAttention",
        Some(SessionStatus::Idle) => "Idle",
        Some(SessionStatus::Connecting) => "Connecting",
        None => "(none)",
    }
}

#[cfg(debug_assertions)]
fn source_label(source: Source) -> &'static str {
    match source {
        Source::Hook => "hook",
        Source::Jsonl => "jsonl",
    }
}

#[cfg(debug_assertions)]
fn session_transition_report(
    prev: Option<SessionStatus>,
    next: Option<SessionStatus>,
    session_id: &str,
    cwd: &Path,
    last_event: &str,
    source: Source,
) -> ErrorReport {
    ErrorReport::new("Claude session status changed")
        .severity(ErrorSeverity::Info)
        .message(format!("{} -> {}", status_label(prev), status_label(next)))
        .at(file!(), line!())
        .with_context("session", session_id)
        .with_context("cwd", redact_home(cwd))
        .with_context("event", last_event)
        .with_context("source", source_label(source))
        .build()
}

#[cfg(debug_assertions)]
fn aggregate_transition_report(
    prev: Option<SessionStatus>,
    next: Option<SessionStatus>,
    cwd: &Path,
) -> ErrorReport {
    ErrorReport::new("Claude lane indicator changed")
        .severity(ErrorSeverity::Info)
        .message(format!("{} -> {}", status_label(prev), status_label(next)))
        .at(file!(), line!())
        .with_context("cwd", redact_home(cwd))
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use daruda_claude::hooks::status_file::{SCHEMA_VERSION, StatusFile};
    use std::path::PathBuf;

    fn entry(session_id: &str, cwd: &str, status: SessionStatus) -> StatusFile {
        StatusFile {
            schema_version: SCHEMA_VERSION,
            session_id: session_id.into(),
            cwd: PathBuf::from(cwd),
            transcript_path: None,
            status,
            last_event: "test".into(),
            tool_name: None,
            tool_input: None,
            permission_mode: None,
            // Timestamp is irrelevant to the priority/filter logic under
            // test; the store no longer applies any age-based transform.
            timestamp: chrono::Utc::now(),
            source: Source::Hook,
        }
    }

    fn store_with(entries: Vec<StatusFile>) -> daruda_claude::ClaudeStatusStore {
        let mut store = daruda_claude::ClaudeStatusStore::new();
        for e in entries {
            store.update(e);
        }
        store
    }

    fn lane_ref(project: u64, lane: u64) -> daruda_store::project::LaneRef {
        daruda_store::project::LaneRef { project, lane }
    }

    fn binding(pane_id: PaneId, session_id: &str) -> (PaneId, PtyBinding) {
        (
            pane_id,
            PtyBinding {
                claude_pid: 0,
                session_id: session_id.into(),
                discovered_at: std::time::SystemTime::UNIX_EPOCH,
            },
        )
    }

    #[test]
    fn aggregate_two_panes_in_one_lane_orders_by_layout_and_maxes_priority() {
        let lane = lane_ref(1, 7);
        // Layout order: pane 10 before pane 20.
        let pane_lane = vec![(10u64, lane), (20u64, lane)];
        let bindings: HashMap<PaneId, PtyBinding> =
            [binding(10, "a"), binding(20, "b")].into_iter().collect();
        // The session cwds deliberately do NOT match the lane path —
        // attribution comes from pane membership, not cwd.
        let store = store_with(vec![
            entry("a", "/elsewhere", SessionStatus::Working),
            entry("b", "/nowhere", SessionStatus::NeedsAttention),
        ]);

        let (per_lane, per_session) = aggregate_over_panes(&pane_lane, &bindings, &store);

        assert_eq!(per_lane.get(&lane), Some(&SessionStatus::NeedsAttention));
        let sessions = per_session.get(&lane).expect("two-pane lane has sub-rows");
        let ids: Vec<&str> = sessions.iter().map(|ps| ps.session_id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b"]);
        assert_eq!(sessions[0].pane_id, 10);
        assert_eq!(sessions[1].pane_id, 20);
    }

    #[test]
    fn aggregate_does_not_mix_panes_across_lanes() {
        let lane_a = lane_ref(1, 1);
        let lane_b = lane_ref(2, 1);
        let pane_lane = vec![(10u64, lane_a), (20u64, lane_b)];
        let bindings: HashMap<PaneId, PtyBinding> =
            [binding(10, "a"), binding(20, "b")].into_iter().collect();
        let store = store_with(vec![
            entry("a", "/x", SessionStatus::Working),
            entry("b", "/y", SessionStatus::NeedsAttention),
        ]);

        let (per_lane, per_session) = aggregate_over_panes(&pane_lane, &bindings, &store);

        assert_eq!(per_lane.get(&lane_a), Some(&SessionStatus::Working));
        assert_eq!(per_lane.get(&lane_b), Some(&SessionStatus::NeedsAttention));
        // Each lane has a single pane → no sub-rows for either.
        assert!(per_session.is_empty());
    }

    #[test]
    fn aggregate_omits_panes_without_binding() {
        let lane = lane_ref(1, 1);
        // Pane 20 has no binding (no Claude session in that pane).
        let pane_lane = vec![(10u64, lane), (20u64, lane)];
        let bindings: HashMap<PaneId, PtyBinding> = [binding(10, "a")].into_iter().collect();
        let store = store_with(vec![entry("a", "/x", SessionStatus::Working)]);

        let (per_lane, per_session) = aggregate_over_panes(&pane_lane, &bindings, &store);

        assert_eq!(per_lane.get(&lane), Some(&SessionStatus::Working));
        // Only one bound session → not a multi-session lane.
        assert!(per_session.is_empty());
    }

    #[test]
    fn aggregate_omits_session_missing_from_store() {
        let lane = lane_ref(1, 1);
        let pane_lane = vec![(10u64, lane), (20u64, lane)];
        // Both panes bound, but 'b' has no store entry → omitted entirely
        // (NOT surfaced as Connecting).
        let bindings: HashMap<PaneId, PtyBinding> =
            [binding(10, "a"), binding(20, "b")].into_iter().collect();
        let store = store_with(vec![entry("a", "/x", SessionStatus::Working)]);

        let (per_lane, per_session) = aggregate_over_panes(&pane_lane, &bindings, &store);

        assert_eq!(per_lane.get(&lane), Some(&SessionStatus::Working));
        // 'b' dropped, so only one effective session → no sub-rows.
        assert!(per_session.is_empty());
    }

    #[test]
    fn aggregate_empty_when_no_panes() {
        let store = daruda_claude::ClaudeStatusStore::new();
        let bindings: HashMap<PaneId, PtyBinding> = HashMap::new();
        let (per_lane, per_session) = aggregate_over_panes(&[], &bindings, &store);
        assert!(per_lane.is_empty());
        assert!(per_session.is_empty());
    }

    #[cfg(debug_assertions)]
    #[test]
    fn session_report_is_info_with_transition_message_and_context() {
        let report = session_transition_report(
            Some(SessionStatus::Working),
            Some(SessionStatus::ExecutingTool),
            "sess-1",
            &PathBuf::from("/tmp/wt"),
            "PreToolUse",
            Source::Hook,
        );
        assert_eq!(report.severity, ErrorSeverity::Info);
        assert_eq!(report.title, "Claude session status changed");
        assert_eq!(report.message, "Working -> ExecutingTool");
        assert_eq!(
            report.context.get("event").map(String::as_str),
            Some("PreToolUse")
        );
        assert_eq!(
            report.context.get("source").map(String::as_str),
            Some("hook")
        );
        // Every transition is its own line — never merged.
        assert!(report.dedup_key.is_none());
    }

    #[cfg(debug_assertions)]
    #[test]
    fn aggregate_report_renders_none_as_label() {
        let report = aggregate_transition_report(
            Some(SessionStatus::NeedsAttention),
            None,
            &PathBuf::from("/tmp/wt"),
        );
        assert_eq!(report.title, "Claude lane indicator changed");
        assert_eq!(report.message, "NeedsAttention -> (none)");
    }
}
