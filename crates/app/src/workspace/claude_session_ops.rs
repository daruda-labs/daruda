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

/// Per-lane leading indicator — the lane's collapsed session status.
pub(in crate::workspace) type LaneStatusMap =
    HashMap<daruda_store::project::LaneRef, SessionStatus>;

/// Per-lane `(session_id, status)` sub-rows in pane order — only lanes
/// with ≥ 2 sessions appear.
pub(in crate::workspace) type LaneSessionsMap =
    HashMap<daruda_store::project::LaneRef, Vec<(String, SessionStatus)>>;

/// Aggregate Claude status over panes, attributing each pane's session to
/// the lane that owns the pane (not the session's cwd).
///
/// `pane_lane` lists every pane in layout order paired with its owning
/// lane. For each pane: a missing binding means the pane has no Claude
/// session; a `session_id` absent from `store` is omitted entirely (it is
/// not surfaced as `Connecting`); a session already collected for the lane
/// (one session bound to two panes, e.g. `claude --resume` of a running
/// session) counts once at its first pane's position.
///
/// Returns:
/// - per-lane leading indicator: [`SessionStatus::aggregate`] over the
///   lane's sessions (only lanes with ≥ 1 session appear). Equal-priority
///   ties (`Working` vs `ExecutingTool`) resolve to the later session in
///   pane order.
/// - per-lane sub-rows: `(session_id, status)` in pane order, only for
///   lanes with ≥ 2 sessions (single-session lanes are fully described by
///   the leading indicator). Ids are cloned only for these lanes.
pub(in crate::workspace) fn aggregate_over_panes(
    pane_lane: &[(PaneId, daruda_store::project::LaneRef)],
    bindings: &HashMap<PaneId, PtyBinding>,
    store: &daruda_claude::ClaudeStatusStore,
) -> (LaneStatusMap, LaneSessionsMap) {
    let mut per_lane: HashMap<daruda_store::project::LaneRef, Vec<(&str, SessionStatus)>> =
        HashMap::new();
    for (pane_id, lane_ref) in pane_lane {
        let Some(binding) = bindings.get(pane_id) else {
            continue;
        };
        let Some(file) = store.get(&binding.session_id) else {
            continue;
        };
        let sessions = per_lane.entry(*lane_ref).or_default();
        // One session, one entry — the same session bound to a second
        // pane (e.g. `claude --resume` of a running session) keeps its
        // first-pane position and must not inflate the sub-row count.
        if sessions.iter().any(|(sid, _)| *sid == binding.session_id) {
            continue;
        }
        sessions.push((binding.session_id.as_str(), file.status));
    }

    let per_lane_status = per_lane
        .iter()
        .filter_map(|(lane_ref, sessions)| {
            SessionStatus::aggregate(sessions.iter().map(|(_, s)| *s)).map(|s| (*lane_ref, s))
        })
        .collect();

    let per_lane_sessions = per_lane
        .into_iter()
        .filter(|(_, sessions)| sessions.len() >= 2)
        .map(|(lane_ref, sessions)| {
            (
                lane_ref,
                sessions
                    .into_iter()
                    .map(|(sid, status)| (sid.to_string(), status))
                    .collect(),
            )
        })
        .collect();

    (per_lane_status, per_lane_sessions)
}

/// `true` when any pane's bound session is in a non-`Idle` status.
/// The status-pulse gate reads this instead of the per-lane aggregate:
/// the aggregate's max-priority collapse would hide a `Connecting`
/// session (priority 0) behind an `Idle` sibling (priority 1) even
/// though its sub-row badge animates. Short-circuits without
/// allocating, so it is cheap on the pulse tick.
pub(in crate::workspace) fn any_pane_session_animating(
    pane_lane: &[(PaneId, daruda_store::project::LaneRef)],
    bindings: &HashMap<PaneId, PtyBinding>,
    store: &daruda_claude::ClaudeStatusStore,
) -> bool {
    pane_lane.iter().any(|(pane_id, _)| {
        bindings
            .get(pane_id)
            .and_then(|binding| store.get(&binding.session_id))
            .is_some_and(|file| !matches!(file.status, SessionStatus::Idle))
    })
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
    /// The pane the session is bound to, if any — captured here so the
    /// transition logger does not re-resolve it.
    pane_id: Option<PaneId>,
    /// The lane that owns the session's pane, if resolvable.
    lane_ref: Option<daruda_store::project::LaneRef>,
}

#[cfg(debug_assertions)]
impl Workspace {
    /// Resolve a session to the pane it is bound to and that pane's lane.
    ///
    /// Scan the bindings for the pane bound to `session_id`, then locate
    /// that pane in `index` (the caller's [`Workspace::pane_lane_index`],
    /// passed in so a single probe builds it once). The reverse scan is
    /// `O(panes)` and debug-only / low-frequency, so it is not indexed.
    /// Either component is `None` when no live binding or pane mapping
    /// exists for the session.
    fn session_pane_lane(
        &self,
        session_id: &str,
        index: &[(PaneId, daruda_store::project::LaneRef)],
    ) -> (Option<PaneId>, Option<daruda_store::project::LaneRef>) {
        let pane_id = self
            .claude
            .pty_claude_bindings
            .iter()
            .find(|(_, binding)| binding.session_id == session_id)
            .map(|(pane_id, _)| *pane_id);
        let lane_ref =
            pane_id.and_then(|pane_id| index.iter().find(|(p, _)| *p == pane_id).map(|(_, l)| *l));
        (pane_id, lane_ref)
    }

    /// Capture the session's stored status and its lane aggregate.
    ///
    /// The lane is resolved from the session's pane via
    /// [`Workspace::session_pane_lane`].
    pub(in crate::workspace) fn probe_lane_status(&self, session_id: &str) -> LaneStatusProbe {
        let index = self.pane_lane_index();
        let (pane_id, lane_ref) = self.session_pane_lane(session_id, &index);
        let aggregate = lane_ref.and_then(|lane_ref| {
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
            pane_id,
            lane_ref,
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
        // Resolve human-readable project + lane labels for the log context
        // from the pane/lane the probe already resolved. Falls back to
        // "(unbound)" when the session has no live pane binding, or the
        // referenced project / lane no longer exists. A real project name
        // paired with an "(unbound)" lane means the project exists but the
        // lane was removed — intentional, not a resolution failure.
        let project = after
            .lane_ref
            .and_then(|lr| self.projects.iter().find(|p| p.id == lr.project))
            .map(|p| p.name.as_str())
            .unwrap_or("(unbound)");
        let lane = after
            .lane_ref
            .and_then(|lr| self.lane_for(lr))
            .map(|l| l.display_name())
            .unwrap_or_else(|| "(unbound)".to_string());
        if before.session != after.session {
            LogWriter::log(session_transition_report(
                before.session,
                after.session,
                session_id,
                cwd,
                last_event,
                source,
                after.pane_id,
                project,
                &lane,
            ));
        }
        if before.aggregate != after.aggregate {
            LogWriter::log(aggregate_transition_report(
                before.aggregate,
                after.aggregate,
                cwd,
                project,
                &lane,
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
#[allow(clippy::too_many_arguments)]
fn session_transition_report(
    prev: Option<SessionStatus>,
    next: Option<SessionStatus>,
    session_id: &str,
    cwd: &Path,
    last_event: &str,
    source: Source,
    pane: Option<PaneId>,
    project: &str,
    lane: &str,
) -> ErrorReport {
    let mut report = ErrorReport::new("Claude session status changed")
        .severity(ErrorSeverity::Info)
        .message(format!("{} -> {}", status_label(prev), status_label(next)))
        .at(file!(), line!())
        .with_context("session", session_id)
        .with_context("cwd", redact_home(cwd))
        .with_context("event", last_event)
        .with_context("source", source_label(source))
        .with_context("project", project)
        .with_context("lane", lane);
    // Pane id is only meaningful while a live binding exists; omit the
    // key entirely otherwise rather than logging a placeholder.
    if let Some(pane) = pane {
        report = report.with_context("pane", pane.to_string());
    }
    report.build()
}

// `cwd` here is the triggering session's own working directory, not the
// owning lane's path — the two can differ now that attribution is by pane.
// The `project` / `lane` keys carry the lane this indicator belongs to.
#[cfg(debug_assertions)]
fn aggregate_transition_report(
    prev: Option<SessionStatus>,
    next: Option<SessionStatus>,
    cwd: &Path,
    project: &str,
    lane: &str,
) -> ErrorReport {
    ErrorReport::new("Claude lane indicator changed")
        .severity(ErrorSeverity::Info)
        .message(format!("{} -> {}", status_label(prev), status_label(next)))
        .at(file!(), line!())
        .with_context("cwd", redact_home(cwd))
        .with_context("project", project)
        .with_context("lane", lane)
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
        // Pane (layout) order is observable through the id order.
        let ids: Vec<&str> = sessions.iter().map(|(sid, _)| sid.as_str()).collect();
        assert_eq!(ids, vec!["a", "b"]);
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

    #[test]
    fn aggregate_counts_a_session_once_per_lane() {
        // The same session bound to two panes (e.g. `claude --resume`
        // of a running session in a second pane) is one session — it
        // must not fabricate a multi-session sub-row.
        let lane = lane_ref(1, 1);
        let pane_lane = vec![(10u64, lane), (20u64, lane)];
        let bindings: HashMap<PaneId, PtyBinding> =
            [binding(10, "s"), binding(20, "s")].into_iter().collect();
        let store = store_with(vec![entry("s", "/x", SessionStatus::Working)]);

        let (per_lane, per_session) = aggregate_over_panes(&pane_lane, &bindings, &store);

        assert_eq!(per_lane.get(&lane), Some(&SessionStatus::Working));
        assert!(
            per_session.is_empty(),
            "one distinct session must not produce a sub-row"
        );
    }

    #[test]
    fn aggregate_dedup_keeps_first_pane_position() {
        // A duplicate binding later in layout order neither reorders
        // nor duplicates the sub-row entry for its session.
        let lane = lane_ref(1, 1);
        let pane_lane = vec![(10u64, lane), (20u64, lane), (30u64, lane)];
        let bindings: HashMap<PaneId, PtyBinding> =
            [binding(10, "a"), binding(20, "b"), binding(30, "a")]
                .into_iter()
                .collect();
        let store = store_with(vec![
            entry("a", "/x", SessionStatus::Working),
            entry("b", "/y", SessionStatus::NeedsAttention),
        ]);

        let (per_lane, per_session) = aggregate_over_panes(&pane_lane, &bindings, &store);

        assert_eq!(per_lane.get(&lane), Some(&SessionStatus::NeedsAttention));
        let ids: Vec<&str> = per_session
            .get(&lane)
            .expect("two distinct sessions keep the sub-row")
            .iter()
            .map(|(sid, _)| sid.as_str())
            .collect();
        assert_eq!(ids, vec!["a", "b"]);
    }

    #[test]
    fn animating_when_a_connecting_session_hides_behind_idle_aggregate() {
        // [Idle, Connecting] in one lane: the per-lane max-priority
        // aggregate is Idle (1 > 0), but the Connecting badge still
        // animates in the sub-row — the pulse gate must look at the
        // sessions, not the collapsed aggregate.
        let lane = lane_ref(1, 1);
        let pane_lane = vec![(10u64, lane), (20u64, lane)];
        let bindings: HashMap<PaneId, PtyBinding> =
            [binding(10, "a"), binding(20, "b")].into_iter().collect();
        let store = store_with(vec![
            entry("a", "/x", SessionStatus::Idle),
            entry("b", "/x", SessionStatus::Connecting),
        ]);
        assert!(any_pane_session_animating(&pane_lane, &bindings, &store));
    }

    #[test]
    fn not_animating_when_every_bound_session_is_idle() {
        let lane = lane_ref(1, 1);
        let pane_lane = vec![(10u64, lane), (20u64, lane)];
        let bindings: HashMap<PaneId, PtyBinding> =
            [binding(10, "a"), binding(20, "b")].into_iter().collect();
        let store = store_with(vec![
            entry("a", "/x", SessionStatus::Idle),
            entry("b", "/y", SessionStatus::Idle),
        ]);
        assert!(!any_pane_session_animating(&pane_lane, &bindings, &store));
    }

    #[test]
    fn not_animating_for_unbound_panes_or_missing_store_entries() {
        // Panes without a binding and bindings whose session has no
        // store entry contribute no motion.
        let lane = lane_ref(1, 1);
        let pane_lane = vec![(10u64, lane), (20u64, lane)];
        let bindings: HashMap<PaneId, PtyBinding> = [binding(10, "ghost")].into_iter().collect();
        let store = daruda_claude::ClaudeStatusStore::new();
        assert!(!any_pane_session_animating(&pane_lane, &bindings, &store));
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
            Some(42),
            "demo-project",
            "feature/x",
        );
        assert_eq!(report.severity, ErrorSeverity::Info);
        assert_eq!(report.title, "Claude session status changed");
        assert_eq!(report.message, "Working -> ExecutingTool");
        assert_eq!(
            report.context.get("session").map(String::as_str),
            Some("sess-1")
        );
        assert_eq!(
            report.context.get("cwd").map(String::as_str),
            Some("/tmp/wt")
        );
        assert_eq!(
            report.context.get("event").map(String::as_str),
            Some("PreToolUse")
        );
        assert_eq!(
            report.context.get("source").map(String::as_str),
            Some("hook")
        );
        // Multi-pane tracking context: pane id + human-readable lane.
        assert_eq!(report.context.get("pane").map(String::as_str), Some("42"));
        assert_eq!(
            report.context.get("project").map(String::as_str),
            Some("demo-project")
        );
        assert_eq!(
            report.context.get("lane").map(String::as_str),
            Some("feature/x")
        );
        // Every transition is its own line — never merged.
        assert!(report.dedup_key.is_none());
    }

    #[cfg(debug_assertions)]
    #[test]
    fn session_report_omits_pane_and_marks_unbound_lane() {
        let report = session_transition_report(
            Some(SessionStatus::Working),
            Some(SessionStatus::Idle),
            "sess-2",
            &PathBuf::from("/tmp/wt"),
            "Stop",
            Source::Jsonl,
            None,
            "(unbound)",
            "(unbound)",
        );
        // No live pane binding → no `pane` key at all.
        assert!(!report.context.contains_key("pane"));
        assert_eq!(
            report.context.get("project").map(String::as_str),
            Some("(unbound)")
        );
        assert_eq!(
            report.context.get("lane").map(String::as_str),
            Some("(unbound)")
        );
    }

    #[cfg(debug_assertions)]
    #[test]
    fn aggregate_report_renders_none_as_label() {
        let report = aggregate_transition_report(
            Some(SessionStatus::NeedsAttention),
            None,
            &PathBuf::from("/tmp/wt"),
            "demo-project",
            "feature/x",
        );
        assert_eq!(report.title, "Claude lane indicator changed");
        assert_eq!(report.message, "NeedsAttention -> (none)");
        assert_eq!(
            report.context.get("cwd").map(String::as_str),
            Some("/tmp/wt")
        );
        assert_eq!(
            report.context.get("project").map(String::as_str),
            Some("demo-project")
        );
        assert_eq!(
            report.context.get("lane").map(String::as_str),
            Some("feature/x")
        );
    }

    #[cfg(debug_assertions)]
    #[test]
    fn aggregate_report_marks_unbound_lane() {
        let report = aggregate_transition_report(
            None,
            Some(SessionStatus::Working),
            &PathBuf::from("/tmp/wt"),
            "(unbound)",
            "(unbound)",
        );
        assert_eq!(
            report.context.get("project").map(String::as_str),
            Some("(unbound)")
        );
        assert_eq!(
            report.context.get("lane").map(String::as_str),
            Some("(unbound)")
        );
        // cwd is retained even when project / lane are unresolved.
        assert_eq!(
            report.context.get("cwd").map(String::as_str),
            Some("/tmp/wt")
        );
    }
}
