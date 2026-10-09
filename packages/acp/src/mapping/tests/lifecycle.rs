use super::*;

#[test]
fn finalize_settles_every_streaming_block_not_just_the_last() {
    // Real shape: the agent streams "let me look", then starts a tool call,
    // then streams a conclusion. The first text block is no longer the tail,
    // so clearing only the last item would leave it `streaming: true` —
    // making the turn's rollup blink and stay expanded after the turn ends.
    let mut items = vec![
        ChatItem::AssistantText {
            text: "let me look".to_string(),
            streaming: true,
            message_id: Some("m1".to_string()),
            phase: Default::default(),
        },
        ChatItem::ToolCall(ToolCallItem {
            id: "t1".to_string(),
            title: "Read".to_string(),
            kind: ToolKindView::Read,
            tool_name: None,
            status: ToolStatusView::Completed,
            diffs: Vec::new(),
            output: Vec::new(),
            raw_input: None,
            locations: Vec::new(),
            parent_tool_id: None,
            exit: None,
        }),
        ChatItem::AssistantText {
            text: "done".to_string(),
            streaming: true,
            message_id: Some("m2".to_string()),
            phase: Default::default(),
        },
    ];
    finalize_streaming(&mut items);
    assert!(
        !items.iter().any(|i| matches!(
            i,
            ChatItem::AssistantText {
                streaming: true,
                ..
            } | ChatItem::Thinking {
                streaming: true,
                ..
            }
        )),
        "every streamed block settles when the turn ends, not just the tail"
    );
}

/// Unlike a tool call, a `Pending` plan entry is already settled: the run
/// never reached it. Only the step it was *on* is a lie once the run ends.
#[test]
fn cancel_pending_plan_entries_settles_only_the_running_step() {
    let entry = |content: &str, status| PlanEntryView {
        content: content.to_string(),
        priority: crate::model::PlanPriority::Medium,
        status,
    };
    let mut plan = vec![
        entry("done", PlanStatus::Completed),
        entry("running", PlanStatus::InProgress),
        entry("todo", PlanStatus::Pending),
    ];
    cancel_pending_plan_entries(&mut plan);
    assert_eq!(plan[0].status, PlanStatus::Completed);
    assert_eq!(plan[1].status, PlanStatus::Cancelled, "running → cancelled");
    assert_eq!(
        plan[2].status,
        PlanStatus::Pending,
        "never started, so settled"
    );

    // Idempotent: a second exit for the same run must not move anything.
    cancel_pending_plan_entries(&mut plan);
    assert_eq!(plan[1].status, PlanStatus::Cancelled);
}

#[test]
fn cancel_pending_tools_settles_only_unfinished_calls() {
    let tool = |id: &str, status| {
        ChatItem::ToolCall(ToolCallItem {
            id: id.to_string(),
            title: id.to_string(),
            kind: ToolKindView::Read,
            tool_name: None,
            status,
            diffs: Vec::new(),
            output: Vec::new(),
            raw_input: None,
            locations: Vec::new(),
            parent_tool_id: None,
            exit: None,
        })
    };
    let mut items = vec![
        tool("pending", ToolStatusView::Pending),
        tool("running", ToolStatusView::InProgress),
        tool("done", ToolStatusView::Completed),
        tool("failed", ToolStatusView::Failed),
    ];
    cancel_pending_tools(&mut items);
    let status = |ix: usize| match &items[ix] {
        ChatItem::ToolCall(tc) => tc.status,
        _ => panic!("expected a tool call"),
    };
    assert_eq!(status(0), ToolStatusView::Cancelled, "pending → cancelled");
    assert_eq!(
        status(1),
        ToolStatusView::Cancelled,
        "in-progress → cancelled"
    );
    assert_eq!(
        status(2),
        ToolStatusView::Completed,
        "completed is terminal"
    );
    assert_eq!(status(3), ToolStatusView::Failed, "failed is terminal");
}

#[test]
fn subagent_activity_counts_live_child_as_running() {
    let items = [subagent_child("p1", ToolStatusView::InProgress)];
    let last_activity = HashMap::new();
    let activity = subagent_activity(
        &items,
        &last_activity,
        Instant::now(),
        Duration::from_secs(8),
    );
    assert_eq!(activity.total, 1);
    assert_eq!(activity.settled, 0);
    assert!(activity.any_running);
}

#[test]
fn subagent_activity_treats_recent_activity_as_running() {
    let items = [subagent_child("p1", ToolStatusView::Completed)];
    let now = Instant::now();
    let mut last_activity = HashMap::new();
    last_activity.insert("p1".to_string(), now - Duration::from_secs(2));
    let activity = subagent_activity(&items, &last_activity, now, Duration::from_secs(8));
    assert_eq!(activity.total, 1);
    assert_eq!(activity.settled, 0, "recent activity keeps it running");
    assert!(activity.any_running);
}

#[test]
fn subagent_activity_settles_after_quiescence_elapses() {
    let items = [subagent_child("p1", ToolStatusView::Completed)];
    let now = Instant::now();
    let mut last_activity = HashMap::new();
    last_activity.insert("p1".to_string(), now - Duration::from_secs(20));
    let activity = subagent_activity(&items, &last_activity, now, Duration::from_secs(8));
    assert_eq!(activity.total, 1);
    assert_eq!(activity.settled, 1);
    assert!(!activity.any_running);
}

#[test]
fn subagent_activity_boundary_is_exclusive_at_quiescence() {
    // Exactly `quiescence` old is the `<` boundary: `now - t == quiescence`
    // is NOT within the window (the check is strict `<`), so a subagent whose
    // only child is settled and whose last activity lands exactly on the
    // boundary is treated as settled, not running.
    let items = [subagent_child("p1", ToolStatusView::Completed)];
    let quiescence = Duration::from_secs(8);
    let now = Instant::now();
    let mut last_activity = HashMap::new();
    last_activity.insert("p1".to_string(), now - quiescence);
    let activity = subagent_activity(&items, &last_activity, now, quiescence);
    assert_eq!(activity.total, 1);
    assert_eq!(
        activity.settled, 1,
        "activity exactly quiescence old is outside the window"
    );
    assert!(!activity.any_running);
}

#[test]
fn subagent_activity_mixes_running_and_settled_parents() {
    let now = Instant::now();
    let items = [
        subagent_child("running", ToolStatusView::InProgress),
        subagent_child("quiesced", ToolStatusView::Completed),
    ];
    let mut last_activity = HashMap::new();
    last_activity.insert("quiesced".to_string(), now - Duration::from_secs(20));
    let activity = subagent_activity(&items, &last_activity, now, Duration::from_secs(8));
    assert_eq!(activity.total, 2);
    assert_eq!(activity.settled, 1);
    assert!(activity.any_running);
}

#[test]
fn subagent_activity_empty_items_is_all_zero() {
    let last_activity = HashMap::new();
    let activity = subagent_activity(&[], &last_activity, Instant::now(), Duration::from_secs(8));
    assert_eq!(activity.total, 0);
    assert_eq!(activity.settled, 0);
    assert!(!activity.any_running);
}

#[test]
fn subagent_activity_counts_untracked_settled_parent() {
    // A parent id with no live child and no last_activity entry at all
    // (never recorded, or the map predates it) is settled, not running.
    let items = [subagent_child("p1", ToolStatusView::Completed)];
    let last_activity = HashMap::new();
    let activity = subagent_activity(
        &items,
        &last_activity,
        Instant::now(),
        Duration::from_secs(8),
    );
    assert_eq!(activity.total, 1);
    assert_eq!(activity.settled, 1);
    assert!(!activity.any_running);
}

#[test]
fn tool_call_then_update_completes_in_place_with_diff() {
    let mut items = Vec::new();
    apply_update(
        &mut items,
        &SessionUpdate::ToolCall(ToolCall::new("t1", "Write").kind(ToolKind::Edit)),
    );

    let mut fields = ToolCallUpdateFields::default();
    fields.status = Some(ToolCallStatus::Completed);
    fields.content = Some(vec![ToolCallContent::Diff(Diff::new("/tmp/x.txt", "hi\n"))]);
    apply_update(
        &mut items,
        &SessionUpdate::ToolCallUpdate(ToolCallUpdate::new("t1", fields)),
    );

    assert_eq!(items.len(), 1, "update must mutate in place, not append");
    let ChatItem::ToolCall(tc) = &items[0] else {
        panic!("expected a tool call item");
    };
    assert_eq!(tc.status, ToolStatusView::Completed);
    assert_eq!(tc.diffs.len(), 1);
    assert_eq!(tc.diffs[0].new_text, "hi\n");
}

#[test]
fn touched_tool_id_extracts_only_tool_call_events() {
    // A `ToolCall` insert and a `ToolCallUpdate` both carry the target id.
    assert_eq!(
        touched_tool_id(&SessionUpdate::ToolCall(ToolCall::new("t1", "Read"))),
        Some("t1".to_string())
    );
    assert_eq!(
        touched_tool_id(&SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
            "t2",
            ToolCallUpdateFields::default()
        ))),
        Some("t2".to_string())
    );
    // A text chunk is not a tool-call event → no id.
    assert_eq!(
        touched_tool_id(&SessionUpdate::AgentMessageChunk(text_chunk("hi"))),
        None
    );
}

#[test]
fn a_fenced_read_becomes_source_text_in_the_extension_s_language() {
    // The adapter delivers the file as a language-less, line-numbered fence.
    assert_eq!(
        read_output_blocks("src/main.rs", "```\n1\tfn main() {}\n2\t// end\n```"),
        vec![ToolOutputBlock::SourceText {
            text: "fn main() {}\n// end".to_string(),
            language: Some("rust".to_string()),
            truncated_from: None,
        }]
    );
}

#[test]
fn an_unfenced_read_becomes_source_text_too() {
    // An adapter that does not markdown-escape: no fence to hang the
    // language off, and the `cat -n` gutter used to survive to the render.
    assert_eq!(
        read_output_blocks("src/main.rs", "   1\tfn main() {}\n   2\tlet x = 1;"),
        vec![ToolOutputBlock::SourceText {
            text: "fn main() {}\nlet x = 1;".to_string(),
            language: Some("rust".to_string()),
            truncated_from: None,
        }]
    );
}

#[test]
fn a_read_of_an_unknown_extension_is_source_text_without_a_language() {
    assert_eq!(
        read_output_blocks("NOTES", "plain contents"),
        vec![ToolOutputBlock::SourceText {
            text: "plain contents".to_string(),
            language: None,
            truncated_from: None,
        }]
    );
}

#[test]
fn a_read_with_only_path_raw_input_stays_markdown() {
    // `path` is too broad: directory/list-style tools naturally use it and
    // may still be classified as `Read`, so only `file_path` proves this is
    // one file's source text.
    assert_eq!(
        read_output_blocks_with_raw_input(
            serde_json::json!({"path": "src"}),
            "```\na.rs\nb.rs\n```",
        ),
        vec![ToolOutputBlock::Text {
            text: "```\na.rs\nb.rs\n```".to_string(),
            truncated_from: None,
        }]
    );
}

#[test]
fn a_shell_command_s_text_output_stays_markdown() {
    let mut items = Vec::new();
    apply_update(
        &mut items,
        &SessionUpdate::ToolCall(
            ToolCall::new("c1", "ls")
                .kind(ToolKind::Execute)
                .raw_input(serde_json::json!({"command": "ls"})),
        ),
    );
    let mut fields = ToolCallUpdateFields::default();
    fields.content = Some(vec![ToolCallContent::Content(Content::new(
        ContentBlock::Text(TextContent::new("```\na.rs\nb.rs\n```")),
    ))]);
    apply_update(
        &mut items,
        &SessionUpdate::ToolCallUpdate(ToolCallUpdate::new("c1", fields)),
    );
    let ChatItem::ToolCall(tc) = &items[0] else {
        panic!("expected a tool call item");
    };
    assert_eq!(
        tc.output,
        vec![ToolOutputBlock::Text {
            text: "```\na.rs\nb.rs\n```".to_string(),
            truncated_from: None,
        }]
    );
}
