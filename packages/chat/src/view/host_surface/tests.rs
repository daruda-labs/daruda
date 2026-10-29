#[test]
fn running_tools_exclude_history_pending_and_subagent_launches() {
    use daruda_acp::{ChatItem, ToolCallItem, ToolKindView, ToolStatusView};
    let tool = |status, input| {
        ChatItem::ToolCall(ToolCallItem {
            id: "tool".into(),
            title: "tool".into(),
            kind: ToolKindView::Execute,
            tool_name: None,
            status,
            diffs: vec![],
            output: vec![],
            raw_input: input,
            parent_tool_id: None,
            locations: vec![],
            exit: None,
        })
    };
    for status in [
        ToolStatusView::Pending,
        ToolStatusView::Completed,
        ToolStatusView::Failed,
        ToolStatusView::Cancelled,
    ] {
        assert!(!super::running_tool_in_current_run(&[tool(status, None)]));
    }
    assert!(super::running_tool_in_current_run(&[tool(
        ToolStatusView::InProgress,
        None
    )]));
    assert!(!super::running_tool_in_current_run(&[
        tool(ToolStatusView::InProgress, None),
        ChatItem::UserText("next run".into()),
    ]));
    let mut launch = tool(
        ToolStatusView::InProgress,
        Some(serde_json::json!({"subagent_type": "general-purpose"})),
    );
    if let ChatItem::ToolCall(tool) = &mut launch {
        tool.kind = ToolKindView::Think;
    }
    assert!(!super::running_tool_in_current_run(&[launch]));
}

use super::super::tests::make_test_view;

/// The readers answer from the constructor's arguments, which is the
/// one state a host sees before any ACP event arrives.
#[gpui::test]
fn a_fresh_pane_reads_back_what_it_was_built_with(cx: &mut gpui::TestAppContext) {
    let view = make_test_view(cx);
    view.read_with(cx, |v, _| {
        assert_eq!(v.agent_id(), "claude");
        assert_eq!(v.agent_name(), "Claude");
        assert_eq!(v.session_id(), None);
        assert!(v.cwd().is_none());
        assert!(v.items().is_empty());
        assert!(!v.last_reconciled_busy());
    })
    .unwrap();
}

#[test]
fn parked_prompts_lead_the_strip_and_the_live_queue_ends_it() {
    use super::super::tests::queued;
    let mut queue = super::PromptQueue {
        paused_prompts: vec![queued(1, "parked")],
        pending_prompts: vec![queued(2, "live")],
        ..super::PromptQueue::default()
    };
    let order: Vec<_> = queue
        .in_strip_order()
        .map(|(q, parked)| (q.text.as_str(), parked))
        .collect();
    assert_eq!(order, [("parked", true), ("live", false)]);
    assert_eq!(queue.last_prompt_id(), Some(queued(2, "").id));
    assert_eq!(
        queue.find(queued(1, "").id).map(|q| q.text.as_str()),
        Some("parked")
    );
    assert!(queue.has_pending());
    queue.pending_prompts.clear();
    assert_eq!(queue.last_prompt_id(), Some(queued(1, "").id));
}
