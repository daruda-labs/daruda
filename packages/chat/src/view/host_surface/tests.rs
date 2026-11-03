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

/// The composer summary follows the oldest question's keyboard: its field,
/// that field's title (or the question's message when it has none), and
/// whether typed text would ride on a single pick as a note.
#[gpui::test]
fn question_composer_reports_the_bound_field_and_note_mode(cx: &mut gpui::TestAppContext) {
    use daruda_acp::{AcpEvent, QuestionField, QuestionFieldKind, QuestionOption, QuestionRequest};

    use super::QuestionComposer;

    let field = |key: &str, title: Option<&str>, kind, companion_of: Option<&str>| QuestionField {
        key: key.into(),
        title: title.map(str::to_owned),
        description: None,
        required: false,
        kind,
        companion_of: companion_of.map(str::to_owned),
        default: None,
    };
    let option = |value: &str| QuestionOption {
        value: value.into(),
        label: value.into(),
        description: None,
    };
    let question = QuestionRequest {
        message: "Before I open the PR".into(),
        tool_call_id: Some("toolu_1".into()),
        delivery: daruda_acp::QuestionDelivery::Blocking,
        fields: vec![
            field(
                "question_0",
                None,
                QuestionFieldKind::SingleSelect(vec![option("a"), option("b")]),
                None,
            ),
            field(
                "question_0_custom",
                None,
                QuestionFieldKind::Text,
                Some("question_0"),
            ),
            field(
                "question_1",
                Some("Draft?"),
                QuestionFieldKind::Boolean,
                None,
            ),
        ],
    };
    let view = make_test_view(cx);
    view.update(cx, |v, _, cx| {
        assert_eq!(v.question_composer(), None);
        v.apply_event(AcpEvent::QuestionAsked { id: 4, question }, "", false, cx);
        assert_eq!(
            v.question_composer(),
            Some(QuestionComposer {
                question_id: 4,
                field: 0,
                position: (1, 2),
                waiting_after: 0,
                header: "Before I open the PR".into(),
                note_mode: false,
                bound: true,
                draft_became_answer: false,
                send_highlighted: false,
                deadline: None,
            })
        );
        v.pick_question_option(4, 0, 1, cx);
        let moved = v.question_composer().expect("still open");
        assert_eq!((moved.field, moved.position), (1, (2, 2)));
        assert_eq!(moved.header, "Draft?");
        assert!(!moved.note_mode, "a yes/no takes no note");
        v.bind_question_composer(4, Some(0), false, cx);
        let back = v.question_composer().expect("still open");
        assert_eq!(back.field, 0);
        assert!(back.note_mode, "text beside the pick is a note on it");
    })
    .unwrap();
}

/// A run report names the oldest question by its first field's title,
/// falls back to the message, and cuts a long text at a char boundary.
#[gpui::test]
fn pending_question_summary_prefers_the_title_and_truncates(cx: &mut gpui::TestAppContext) {
    use daruda_acp::{AcpEvent, QuestionField, QuestionFieldKind, QuestionRequest};

    let ask = |title: Option<&str>, message: &str| QuestionRequest {
        message: message.into(),
        tool_call_id: Some("toolu_1".into()),
        delivery: daruda_acp::QuestionDelivery::Blocking,
        fields: vec![QuestionField {
            key: "question_0".into(),
            title: title.map(str::to_owned),
            description: None,
            required: false,
            kind: QuestionFieldKind::Text,
            companion_of: None,
            default: None,
        }],
    };
    let view = make_test_view(cx);
    view.update(cx, |v, _, cx| {
        assert_eq!(v.pending_question_summary(), None);
        let question = ask(Some("Which check?"), "Before I ship");
        v.apply_event(AcpEvent::QuestionAsked { id: 1, question }, "", false, cx);
        assert_eq!(
            v.pending_question_summary().as_deref(),
            Some("Which check?")
        );
    })
    .unwrap();
    let view = make_test_view(cx);
    view.update(cx, |v, _, cx| {
        let long = "한".repeat(super::QUESTION_SUMMARY_MAX_CHARS + 5);
        let question = ask(None, &long);
        v.apply_event(AcpEvent::QuestionAsked { id: 1, question }, "", false, cx);
        let summary = v.pending_question_summary().unwrap();
        assert_eq!(
            summary.chars().count(),
            super::QUESTION_SUMMARY_MAX_CHARS + 1
        );
        assert!(summary.ends_with('…'));
    })
    .unwrap();
}
