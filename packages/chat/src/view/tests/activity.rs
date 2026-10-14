//! The activity span and the clocks it drives — the pane's busy level and each
//! tool call's running time — including what the clock moves with no event.

use super::*;

#[gpui::test]
fn protocol_notices_do_not_change_transcript_turn_or_cancel_ack(cx: &mut gpui::TestAppContext) {
    use daruda_acp::{AcpEvent, NoticeSeverityView, SessionNoticeView};
    let window = make_test_view(cx);
    window
        .update(cx, |view, _, cx| {
            view.set_turn_in_flight();
            for severity in [
                NoticeSeverityView::Info,
                NoticeSeverityView::Error,
                NoticeSeverityView::Other("future".into()),
            ] {
                let items = view.items.clone();
                let outcome = view.activity.pending_completion.clone();
                view.apply_event(
                    AcpEvent::SessionNotice {
                        session_id: "root".into(),
                        notice: SessionNoticeView {
                            severity,
                            title: "Advisory".into(),
                            description: None,
                        },
                    },
                    "",
                    false,
                    cx,
                );
                assert!(!view.turn_is_idle());
                assert_eq!(view.items, items);
                assert_eq!(view.activity.pending_completion, outcome);
            }
            view.cancel_turn(cx);
            assert!(view.turn_awaiting_cancel_ack());
            let items = view.items.clone();
            view.apply_event(
                AcpEvent::SessionNotice {
                    session_id: "child".into(),
                    notice: SessionNoticeView {
                        severity: NoticeSeverityView::Error,
                        title: "Still advisory".into(),
                        description: None,
                    },
                },
                "",
                false,
                cx,
            );
            assert!(view.turn_awaiting_cancel_ack());
            assert_eq!(view.items, items);
            assert_eq!(
                view.activity.pending_completion,
                Some(crate::view::TurnOutcome::Stopped)
            );
        })
        .unwrap();
}

/// The inline "Working…" row is projected from `activity_state()`, which is a
/// *time-dependent* predicate: a trailing subagent counts as busy until its
/// last activity falls out of the quiescence window. Nothing mutates the model
/// when that window lapses — the projected row simply goes stale — so the
/// activity tick that notices the settle is the only thing that can drop it.
///
/// The lapse is constructed by backdating the timestamp rather than sleeping
/// out the real window; the end state is the same one the clock reaches.
#[gpui::test]
fn a_time_driven_settle_drops_the_working_indicator(cx: &mut gpui::TestAppContext) {
    use crate::rows::RowKind;
    use daruda_acp::{ChatItem, ToolCallItem, ToolKindView, ToolStatusView};

    let tool = |id: &str, parent: Option<&str>| {
        ChatItem::ToolCall(ToolCallItem {
            id: id.to_string(),
            title: id.to_string(),
            kind: ToolKindView::Read,
            tool_name: None,
            // Settled: the only thing holding the pane busy is the parent's
            // quiescence window, which is what makes the settle time-driven.
            status: ToolStatusView::Completed,
            diffs: Vec::new(),
            output: Vec::new(),
            raw_input: None,
            locations: Vec::new(),
            parent_tool_id: parent.map(str::to_string),
            exit: None,
        })
    };
    let working = |v: &crate::view::AgentChatView| {
        v.rows
            .iter()
            .any(|r| matches!(r.kind, RowKind::WorkingIndicator))
    };

    let window = make_test_view(cx);
    window
        .update(cx, |v, _, cx| {
            v.items = vec![tool("task-1", None), tool("child-1", Some("task-1"))];
            v.activity
                .subagent_last_activity
                .insert("task-1".into(), std::time::Instant::now());
            // A benign event projects the rows while the subagent is live.
            v.apply_event(daruda_acp::AcpEvent::Notice("tick".into()), "", false, cx);
            assert!(working(v), "a live subagent puts the indicator on screen");
            // Observe busy first so the tick has an edge to detect.
            v.tick_activity(std::time::Instant::now(), cx);

            // The subagent's quiescence window lapses, with no event to say so.
            let lapsed = std::time::Instant::now()
                .checked_sub(std::time::Duration::from_secs(20))
                .expect("the monotonic clock has 20s of history");
            v.activity
                .subagent_last_activity
                .insert("task-1".into(), lapsed);
            v.tick_activity(std::time::Instant::now(), cx);

            assert!(
                !working(v),
                "the settle tick must drop the row the run no longer earns"
            );
        })
        .unwrap();
}

/// The tooltip says "last active", so the timestamp has to advance whenever
/// this pane goes quiet. The adapter sends `updatedAt` only alongside a
/// *changed* title, so relying on it alone would freeze the value.
#[gpui::test]
fn activity_settle_stamps_last_active(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, _cx| {
            assert_eq!(
                view.session_updated_at, None,
                "a fresh pane has no activity"
            );

            view.set_turn_in_flight();
            view.reconcile_activity(std::time::Instant::now());
            assert_eq!(
                view.session_updated_at, None,
                "still working -- the span has not ended yet"
            );

            view.set_turn_idle();
            view.reconcile_activity(std::time::Instant::now());
            let stamped = view
                .session_updated_at
                .clone()
                .expect("the busy->idle settle stamps last-active");
            // The field feeds `format_last_active`, which parses RFC 3339.
            chrono::DateTime::parse_from_rfc3339(&stamped)
                .unwrap_or_else(|e| panic!("stamped {stamped:?} is not RFC 3339: {e}"));
        })
        .unwrap();
}

/// Activity reconciliation turns `is_busy()` into idle->busy / busy->idle
/// edges, tracks one busy span across contiguous turns, and fires captured
/// completion exactly once when the pane settles.
#[gpui::test]
fn reconcile_activity_edges_and_completion_delivery(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, _cx| {
            let reset = |view: &mut crate::view::AgentChatView| {
                view.set_turn_idle();
                view.items.clear();
                view.activity = crate::view::ActivityTracker::default();
            };

            assert!(view.activity_elapsed().is_none(), "idle -> no span");
            view.set_turn_in_flight();
            let edge = view.reconcile_activity(std::time::Instant::now());
            assert_eq!(edge, None, "the busy edge fires no completion");
            assert!(
                view.activity.span.is_busy(),
                "reconcile records the busy level"
            );
            assert!(
                view.activity_elapsed().is_some(),
                "the span start is stamped on idle->busy"
            );

            reset(view);
            view.set_turn_in_flight();
            assert_eq!(view.reconcile_activity(std::time::Instant::now()), None);
            view.activity.pending_completion = Some(crate::view::TurnOutcome::Completed);
            view.set_turn_idle();
            let edge = view.reconcile_activity(std::time::Instant::now());
            assert_eq!(
                edge,
                Some(crate::view::TurnOutcome::Completed),
                "the busy->idle edge returns the stashed outcome"
            );
            assert!(
                view.activity_elapsed().is_none(),
                "the span start clears on settle"
            );
            assert_eq!(
                view.reconcile_activity(std::time::Instant::now()),
                None,
                "the outcome fires exactly once"
            );

            reset(view);
            view.set_turn_in_flight();
            assert_eq!(view.reconcile_activity(std::time::Instant::now()), None);
            view.set_turn_idle();
            assert_eq!(
                view.reconcile_activity(std::time::Instant::now()),
                None,
                "a settle with no pending outcome fires nothing"
            );

            reset(view);
            view.set_turn_in_flight();
            assert_eq!(view.reconcile_activity(std::time::Instant::now()), None);
            view.activity.pending_completion = Some(crate::view::TurnOutcome::Stopped);
            view.set_turn_in_flight();
            assert_eq!(
                view.reconcile_activity(std::time::Instant::now()),
                None,
                "still busy across the turn boundary -> no settle edge"
            );
            view.activity.pending_completion = Some(crate::view::TurnOutcome::Completed);
            view.set_turn_idle();
            assert_eq!(
                view.reconcile_activity(std::time::Instant::now()),
                Some(crate::view::TurnOutcome::Completed),
                "the span fires the latest outcome exactly once at the final settle"
            );

            reset(view);
            view.set_turn_in_flight();
            assert_eq!(view.reconcile_activity(std::time::Instant::now()), None);
            assert!(
                view.activity.span.is_busy(),
                "the connect-time reconcile stamps the busy level"
            );
            view.activity.pending_completion = Some(crate::view::TurnOutcome::Completed);
            view.set_turn_idle();
            assert_eq!(
                view.reconcile_activity(std::time::Instant::now()),
                Some(crate::view::TurnOutcome::Completed),
                "connect-time pump completion fires at settle instead of stranding"
            );
        })
        .unwrap();
}

#[gpui::test]
fn activity_state_maps_permission_turn_and_background_subagent(cx: &mut gpui::TestAppContext) {
    use daruda_acp::ToolStatusView;

    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, _cx| {
            view.status = crate::view::AgentSessionStatus::Connected;

            view.set_turn_in_flight();
            view.items.clear();
            assert_eq!(view.activity_state(), crate::view::ActivityState::Working);

            view.set_turn_idle();
            view.items = vec![tool_call(
                "child",
                ToolStatusView::InProgress,
                Some("parent"),
            )];
            assert_eq!(
                view.activity_state(),
                crate::view::ActivityState::Working,
                "a running background child tool keeps the pane Working after end_turn"
            );

            view.set_turn_idle();
            view.items.clear();
            view.pending_permissions.insert(7);
            assert_eq!(
                view.activity_state(),
                crate::view::ActivityState::AwaitingPermission
            );

            view.set_turn_idle();
            view.pending_permissions.clear();
            view.items = vec![
                tool_call("child", ToolStatusView::Completed, Some("parent")),
                tool_call("top", ToolStatusView::InProgress, None),
            ];
            assert_eq!(
                view.activity_state(),
                crate::view::ActivityState::Idle,
                "a completed child and a top-level running tool are not background activity"
            );
        })
        .unwrap();
}

// When a tool call's running clock starts, survives a settle, and is skipped.

/// A tool call's elapsed time is measured from the first update that named it,
/// not from any field the protocol carries — `ToolCallItem` has no timestamp,
/// and the mapper is deliberately clock-free. The first sighting wins, so a
/// long-running call's later progress updates don't reset its age.
#[gpui::test]
fn a_tool_calls_age_runs_from_its_first_update(cx: &mut gpui::TestAppContext) {
    use agent_client_protocol::schema::v1::{SessionUpdate, ToolCall, ToolCallUpdate};
    use daruda_acp::AcpEvent;

    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.apply_event(
                AcpEvent::Update(Box::new(SessionUpdate::ToolCall(ToolCall::new(
                    "t1", "Bash",
                )))),
                "",
                false,
                cx,
            );
            let first = view
                .activity
                .tool_started_at
                .get("t1")
                .copied()
                .expect("a seen call is being timed");

            // A later progress update must not restart the clock.
            view.apply_event(
                AcpEvent::Update(Box::new(SessionUpdate::ToolCallUpdate(
                    ToolCallUpdate::new("t1", Default::default()),
                ))),
                "",
                false,
                cx,
            );
            let second = view
                .activity
                .tool_started_at
                .get("t1")
                .copied()
                .expect("still timed");
            assert_eq!(first, second, "a later update must not restart the clock");

            assert!(
                !view.activity.tool_started_at.contains_key("never-seen"),
                "a call nothing reported has no age to show"
            );
        })
        .unwrap();
}

/// The settle edge is time-driven — a trailing subagent's window lapsing — and
/// a *top-level* call is invisible to `is_busy`, so the edge can arrive while
/// one is still running. Bounding the start-time map there must not take the
/// clock out from under a card that is still showing it: the counter would
/// vanish mid-run, then the next update would restart it from `0s`.
#[gpui::test]
fn a_settle_edge_keeps_the_clock_of_a_call_that_is_still_running(cx: &mut gpui::TestAppContext) {
    use daruda_acp::ToolStatusView;

    let lapsed = |secs| {
        std::time::Instant::now()
            .checked_sub(std::time::Duration::from_secs(secs))
            .expect("the monotonic clock has history")
    };

    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, _cx| {
            view.items = vec![
                tool_call("task-1", ToolStatusView::Completed, None),
                tool_call("child-1", ToolStatusView::Completed, Some("task-1")),
                tool_call("bash-1", ToolStatusView::InProgress, None),
            ];
            view.activity
                .subagent_last_activity
                .insert("task-1".into(), std::time::Instant::now());
            view.activity
                .tool_started_at
                .insert("bash-1".into(), lapsed(30));
            view.activity
                .tool_started_at
                .insert("child-1".into(), lapsed(30));
            view.reconcile_activity(std::time::Instant::now());

            // The subagent's window lapses with the top-level call still live.
            view.activity
                .subagent_last_activity
                .insert("task-1".into(), lapsed(20));
            view.reconcile_activity(std::time::Instant::now());

            assert!(
                view.activity.tool_started_at.contains_key("bash-1"),
                "a running call keeps the clock its card is showing"
            );
            assert!(
                !view.activity.tool_started_at.contains_key("child-1"),
                "a settled call is dropped — the map stays bounded"
            );
        })
        .unwrap();
}

/// A replay re-states history; none of it happened now. The subagent stamp and
/// the tool clock are the two places an `Update` reads the wall clock, so both
/// have to skip it — otherwise a restored conversation reads busy, and the
/// pump's `tick_activity` reprojects in the middle of the load the gate exists
/// to coalesce.
#[gpui::test]
fn a_replay_stamps_neither_clock(cx: &mut gpui::TestAppContext) {
    use agent_client_protocol::schema::v1::{SessionUpdate, ToolCallUpdate};
    use daruda_acp::{AcpEvent, ToolStatusView};

    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.begin_connect(Some("sess-1".into()), cx);
            view.items = vec![
                tool_call("task-1", ToolStatusView::Completed, None),
                tool_call("child-1", ToolStatusView::InProgress, Some("task-1")),
            ];
            view.apply_event(
                AcpEvent::Update(Box::new(SessionUpdate::ToolCallUpdate(
                    ToolCallUpdate::new("child-1", Default::default()),
                ))),
                "",
                false,
                cx,
            );

            assert!(
                view.activity.subagent_last_activity.is_empty(),
                "a replayed subagent event did not happen now"
            );
            assert!(
                view.activity.tool_started_at.is_empty(),
                "a replayed call did not start now"
            );
        })
        .unwrap();
}
