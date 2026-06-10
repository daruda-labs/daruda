//! Claude status pane aggregation.
//!
//! `aggregate_over_panes` is the single source of truth for the left-dock
//! leading indicator's aggregate, shared with the render snapshot
//! (`render::snapshots`'s `claude_status_per_lane` /
//! `claude_per_session_per_lane`). The debug-only probe / logger sit on top
//! of it so the on-disk NDJSON log can be diffed against the rendered
//! indicator when verifying status transitions.

use std::collections::HashMap;

use crate::hooks::pty_tracker::PtyBinding;
use crate::workspace::Workspace;
use crate::workspace::main_area::pane_tree::PaneId;
use daruda_claude::SessionStatus;

// `Path` and `Source` are only named by the debug-only transition logger
// (and `Source` also by the always-compiled test fixtures), so gate the
// imports to those profiles to avoid an unused-import warning in release
// non-test builds.
#[cfg(any(debug_assertions, test))]
use daruda_claude::hooks::status_file::Source;
#[cfg(debug_assertions)]
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
#[cfg(debug_assertions)]
use daruda_store::observability::log_writer::LogWriter;
#[cfg(debug_assertions)]
use daruda_store::observability::system_info::redact_home;
#[cfg(debug_assertions)]
use std::path::Path;

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
    /// Every pane in the workspace paired with its owning lane. The
    /// active lane's panes come first, in tab/layout order, from the
    /// live `main_area` tabs; then each inactive lane's panes from its
    /// frozen runtime — layout order within a lane, cross-lane order
    /// unspecified (HashMap iteration), so consumers must group per
    /// lane. Feeds [`aggregate_over_panes`] so status attribution
    /// follows pane membership rather than session cwd.
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
        // Skip sessions not bound to any pane in this workspace (claude
        // running outside daruda, or another instance's session). They
        // never drive a lane indicator here, so logging their transitions
        // only floods the file with "(unbound)" noise. A `Some` lane_ref
        // whose project/lane was later removed still logs below (a real
        // ownership change worth recording).
        if after.lane_ref.is_none() {
            return;
        }
        // Resolve human-readable project + lane labels for the log context
        // from the pane/lane the probe already resolved. Falls back to
        // "(unbound)" when the referenced project / lane no longer exists.
        // A real project name paired with an "(unbound)" lane means the
        // project exists but the lane was removed — intentional, not a
        // resolution failure.
        let project = after
            .lane_ref
            .and_then(|lr| self.project_for(lr.project))
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
            notification: None,
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
