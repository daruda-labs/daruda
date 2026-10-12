//! The activity span, as the clock moves it with no event to announce it.

use super::make_test_view;

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
