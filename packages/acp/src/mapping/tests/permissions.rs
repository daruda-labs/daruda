use super::*;

#[test]
fn permission_request_maps_options_in_order() {
    let mut tool_fields = ToolCallUpdateFields::default();
    tool_fields.title = Some("Write /tmp/x".to_string());
    let request = RequestPermissionRequest::new(
        "s1",
        ToolCallUpdate::new("t1", tool_fields),
        vec![
            PermissionOption::new(
                "allow_always",
                "Always Allow",
                PermissionOptionKind::AllowAlways,
            ),
            PermissionOption::new("reject", "Reject", PermissionOptionKind::RejectOnce),
        ],
    );

    let ChatItem::Permission(card) = permission_item(7, &request, &[]) else {
        panic!("expected permission item");
    };
    assert_eq!(card.tool_title.as_deref(), Some("Write /tmp/x"));
    assert_eq!(card.options.len(), 2);
    assert_eq!(card.options[0].kind, PermissionKindView::AllowAlways);
    assert_eq!(card.options[1].option_id, "reject");
    assert_eq!(card.resolved, None);
}

#[test]
fn permission_item_carries_both_correlation_ids() {
    // The request id correlates the card to its parked response; the tool
    // call id correlates it to the file diff the user is deciding on.
    let request = RequestPermissionRequest::new(
        "s1",
        ToolCallUpdate::new("t1", ToolCallUpdateFields::default()),
        vec![PermissionOption::new(
            "allow_once",
            "Allow",
            PermissionOptionKind::AllowOnce,
        )],
    );

    let ChatItem::Permission(card) = permission_item(42, &request, &[]) else {
        panic!("expected permission item");
    };
    assert_eq!(card.id, 42);
    assert_eq!(card.tool_call_id, "t1");
}

#[test]
fn permission_request_prefers_raw_input_from_a_matching_existing_tool_call() {
    // codex-acp sometimes re-quotes `rawInput.command` for the permission
    // request and mangles embedded quotes, even though a clean copy
    // already arrived via the tool_call insert for the same id. Prefer
    // the item list's already-clean copy over the request's own.
    let items = vec![ChatItem::ToolCall(ToolCallItem {
        id: "t1".to_string(),
        title: "perl -0pi -e ...".to_string(),
        kind: ToolKindView::Execute,
        tool_name: None,
        status: ToolStatusView::InProgress,
        diffs: Vec::new(),
        output: Vec::new(),
        raw_input: Some(serde_json::json!({"command": "perl -0pi -e 's/clean/'"})),
        locations: Vec::new(),
        parent_tool_id: None,
        exit: None,
    })];

    let mut tool_fields = ToolCallUpdateFields::default();
    tool_fields.raw_input = Some(serde_json::json!({"command": "perl -0pi -e 's/GARBLED\"'!--/'"}));
    let request = RequestPermissionRequest::new(
        "s1",
        ToolCallUpdate::new("t1", tool_fields),
        vec![PermissionOption::new(
            "allow",
            "Allow",
            PermissionOptionKind::AllowOnce,
        )],
    );

    let ChatItem::Permission(card) = permission_item(1, &request, &items) else {
        panic!("expected permission item");
    };
    assert_eq!(
        card.raw_input_summary.as_deref(),
        Some("command: perl -0pi -e 's/clean/'")
    );
}

#[test]
fn permission_request_falls_back_to_its_own_raw_input_without_a_matching_tool_call() {
    // No matching ToolCall in items (e.g. Claude-style: permission
    // arrives before any tool_call event for that id) — use the
    // request's own raw_input, same as before this change.
    let mut tool_fields = ToolCallUpdateFields::default();
    tool_fields.raw_input = Some(serde_json::json!({"command": "npm install"}));
    let request = RequestPermissionRequest::new(
        "s1",
        ToolCallUpdate::new("t1", tool_fields),
        vec![PermissionOption::new(
            "allow",
            "Allow",
            PermissionOptionKind::AllowOnce,
        )],
    );

    let ChatItem::Permission(card) = permission_item(1, &request, &[]) else {
        panic!("expected permission item");
    };
    assert_eq!(
        card.raw_input_summary.as_deref(),
        Some("command: npm install")
    );
}

#[test]
fn permission_request_summarizes_a_known_raw_input_field() {
    let mut tool_fields = ToolCallUpdateFields::default();
    tool_fields.title = Some("Run npm install".to_string());
    tool_fields.raw_input = Some(serde_json::json!({"command": "npm install"}));
    let request = RequestPermissionRequest::new(
        "s1",
        ToolCallUpdate::new("t1", tool_fields),
        vec![PermissionOption::new(
            "allow",
            "Allow",
            PermissionOptionKind::AllowOnce,
        )],
    );

    let ChatItem::Permission(card) = permission_item(1, &request, &[]) else {
        panic!("expected permission item");
    };
    assert_eq!(
        card.raw_input_summary.as_deref(),
        Some("command: npm install")
    );
}

#[test]
fn summarize_raw_input_picks_the_first_matching_key_in_priority_order() {
    // `command` outranks `file_path` when both happen to be present.
    assert_eq!(
        summarize_raw_input(Some(
            &serde_json::json!({"file_path": "/tmp/x", "command": "cat /tmp/x"})
        )),
        Some("command: cat /tmp/x".to_string())
    );
}

#[test]
fn summarize_raw_input_truncates_an_overlong_value() {
    let long_path = "a".repeat(RAW_INPUT_SUMMARY_MAX_CHARS + 50);
    let summary = summarize_raw_input(Some(&serde_json::json!({"file_path": long_path}))).unwrap();
    assert_eq!(
        summary.chars().count(),
        "file: ".len() + RAW_INPUT_SUMMARY_MAX_CHARS + 1 // +1 for the "…" marker
    );
    assert!(summary.ends_with('…'));
}

#[test]
fn summarize_raw_input_is_none_without_a_known_key() {
    assert_eq!(
        summarize_raw_input(Some(&serde_json::json!({"subagent_type": "reviewer"}))),
        None
    );
}

#[test]
fn summarize_raw_input_is_none_for_an_empty_matched_value() {
    assert_eq!(
        summarize_raw_input(Some(&serde_json::json!({"command": ""}))),
        None
    );
}

#[test]
fn summarize_raw_input_is_none_without_raw_input() {
    assert_eq!(summarize_raw_input(None), None);
}
