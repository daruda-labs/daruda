use super::*;
use daruda_agent::hooks::status_file::{SCHEMA_VERSION, StatusFile};
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
        // test; the store applies no age-based transform.
        timestamp: chrono::Utc::now(),
        source: Source::Hook,
    }
}

fn store_with(entries: Vec<StatusFile>) -> daruda_agent::ClaudeStatusStore {
    let mut store = daruda_agent::ClaudeStatusStore::new();
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

    let (per_lane, per_session) = aggregate_over_panes(&pane_lane, &bindings, &store, &[]);

    assert_eq!(per_lane.get(&lane), Some(&SessionStatus::NeedsAttention));
    let sessions = per_session.get(&lane).expect("two-pane lane has sub-rows");
    // Pane (layout) order is observable through the id order.
    let ids: Vec<&str> = sessions.iter().map(|(sid, _)| sid.as_str()).collect();
    assert_eq!(ids, vec!["a", "b"]);
}

/// A tab shows its most urgent pane, PTY session or agent chat alike;
/// a tab with no session shows nothing.
#[test]
fn a_tab_takes_the_most_urgent_of_its_panes() {
    let bindings: HashMap<PaneId, PtyBinding> = [binding(10, "a")].into_iter().collect();
    let store = store_with(vec![entry("a", "/x", SessionStatus::Working)]);
    let acp = [(20u64, SessionStatus::NeedsAttention)];
    assert_eq!(
        tab_session_status(&[10, 20], &bindings, &store, &acp),
        Some(SessionStatus::NeedsAttention)
    );
    assert_eq!(
        tab_session_status(&[10], &bindings, &store, &acp),
        Some(SessionStatus::Working)
    );
    assert_eq!(tab_session_status(&[30], &bindings, &store, &acp), None);
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

    let (per_lane, per_session) = aggregate_over_panes(&pane_lane, &bindings, &store, &[]);

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

    let (per_lane, per_session) = aggregate_over_panes(&pane_lane, &bindings, &store, &[]);

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

    let (per_lane, per_session) = aggregate_over_panes(&pane_lane, &bindings, &store, &[]);

    assert_eq!(per_lane.get(&lane), Some(&SessionStatus::Working));
    // 'b' dropped, so only one effective session → no sub-rows.
    assert!(per_session.is_empty());
}

#[test]
fn aggregate_empty_when_no_panes() {
    let store = daruda_agent::ClaudeStatusStore::new();
    let bindings: HashMap<PaneId, PtyBinding> = HashMap::new();
    let (per_lane, per_session) = aggregate_over_panes(&[], &bindings, &store, &[]);
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

    let (per_lane, per_session) = aggregate_over_panes(&pane_lane, &bindings, &store, &[]);

    assert_eq!(per_lane.get(&lane), Some(&SessionStatus::Working));
    assert!(
        per_session.is_empty(),
        "one distinct session must not produce a sub-row"
    );
}

#[test]
fn distinct_sessions_count_a_session_once_across_lanes_and_keep_chat_panes() {
    let bindings: HashMap<PaneId, PtyBinding> =
        [binding(10, "s"), binding(20, "s"), binding(30, "gone")]
            .into_iter()
            .collect();
    let store = store_with(vec![entry("s", "/x", SessionStatus::ExecutingTool)]);
    let acp = [(40u64, SessionStatus::NeedsAttention)];

    let sessions = distinct_sessions([10, 20, 30, 40, 50], &bindings, &store, &acp);

    assert_eq!(
        sessions,
        vec![
            ("s".to_owned(), SessionStatus::ExecutingTool),
            ("acp:40".to_owned(), SessionStatus::NeedsAttention),
        ]
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

    let (per_lane, per_session) = aggregate_over_panes(&pane_lane, &bindings, &store, &[]);

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
    assert!(any_pane_session_animating(
        &pane_lane,
        &bindings,
        &store,
        &[],
        &daruda_config::StatusIconTheme::Dots,
        &crate::status_icon_themes::Catalog::default()
    ));
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
    assert!(!any_pane_session_animating(
        &pane_lane,
        &bindings,
        &store,
        &[],
        &daruda_config::StatusIconTheme::Dots,
        &crate::status_icon_themes::Catalog::default()
    ));
}

#[test]
fn not_animating_for_unbound_panes_or_missing_store_entries() {
    // Panes without a binding and bindings whose session has no
    // store entry contribute no motion.
    let lane = lane_ref(1, 1);
    let pane_lane = vec![(10u64, lane), (20u64, lane)];
    let bindings: HashMap<PaneId, PtyBinding> = [binding(10, "ghost")].into_iter().collect();
    let store = daruda_agent::ClaudeStatusStore::new();
    assert!(!any_pane_session_animating(
        &pane_lane,
        &bindings,
        &store,
        &[],
        &daruda_config::StatusIconTheme::Dots,
        &crate::status_icon_themes::Catalog::default()
    ));
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

#[test]
fn icon_attention_is_static_but_does_not_hide_a_busy_sibling() {
    let lane = lane_ref(1, 1);
    let panes = [(10, lane), (20, lane)];
    let bindings = HashMap::new();
    let store = daruda_agent::ClaudeStatusStore::new();
    let attention = [(10, SessionStatus::NeedsAttention)];
    assert!(!any_pane_session_animating(
        &panes,
        &bindings,
        &store,
        &attention,
        &daruda_config::StatusIconTheme::Icons,
        &crate::status_icon_themes::Catalog::default()
    ));
    assert!(any_pane_session_animating(
        &panes,
        &bindings,
        &store,
        &attention,
        &daruda_config::StatusIconTheme::Dots,
        &crate::status_icon_themes::Catalog::default()
    ));
    assert!(any_pane_session_animating(
        &panes,
        &bindings,
        &store,
        &[
            (10, SessionStatus::NeedsAttention),
            (20, SessionStatus::Working)
        ],
        &daruda_config::StatusIconTheme::Icons,
        &crate::status_icon_themes::Catalog::default()
    ));
}
