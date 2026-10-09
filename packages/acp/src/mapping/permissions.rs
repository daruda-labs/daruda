//! Permission cards and raw-input summaries.

use super::*;

/// Keys (in priority order) checked when summarizing a tool's `raw_input`
/// into one short, safe line for [`permission_item`] — an allowlist, not a
/// generic dump: a bulk-content field (Edit's `old_string`/`new_string`,
/// Write's `content`) is diff-shaped data exactly like
/// `ToolCallContent::Diff`, and just as wrong to surface in a one-line
/// summary as a full diff would be, so only short, single-value fields are
/// read.
const RAW_INPUT_SUMMARY_KEYS: &[(&str, &str)] = &[
    ("command", "command"),
    ("file_path", "file"),
    ("path", "file"),
    ("pattern", "pattern"),
    ("query", "query"),
    ("url", "url"),
];

/// Max chars kept from a matched raw-input value. A well-formed value for
/// any of [`RAW_INPUT_SUMMARY_KEYS`] is short (a path, a command, a query),
/// but nothing in the protocol bounds what an adapter actually sends.
pub(super) const RAW_INPUT_SUMMARY_MAX_CHARS: usize = 300;

/// Build a pending permission card from a `session/request_permission` request.
/// `items` is the conversation's current item list — used to look up a
/// `raw_input` already recorded for this tool call id (see
/// [`existing_raw_input`]) in preference to the request's own copy, which
/// some adapters (codex-acp) re-quote for the permission prompt and can
/// mangle in the process.
pub fn permission_item(
    id: u64,
    request: &RequestPermissionRequest,
    items: &[ChatItem],
) -> ChatItem {
    let own_raw_input = request.tool_call.fields.raw_input.as_ref();
    let raw_input = existing_raw_input(items, &request.tool_call.tool_call_id.0).or(own_raw_input);
    ChatItem::Permission(PermissionItem {
        id,
        tool_call_id: request.tool_call.tool_call_id.0.to_string(),
        tool_title: request.tool_call.fields.title.clone(),
        raw_input_summary: summarize_raw_input(raw_input),
        options: request.options.iter().map(choice_of).collect(),
        resolved: None,
    })
}

/// The `raw_input` already recorded on a `ChatItem::ToolCall` matching
/// `tool_call_id`, if the item list has one. A prior `tool_call` (insert)
/// event for the same id — which arrives before the permission request in
/// every observed adapter — carries the pristine value; the permission
/// request's own copy is a separate, adapter-reconstructed echo that isn't
/// guaranteed to match it byte-for-byte.
pub(super) fn existing_raw_input<'a>(
    items: &'a [ChatItem],
    tool_call_id: &str,
) -> Option<&'a serde_json::Value> {
    items.iter().rev().find_map(|item| match item {
        ChatItem::ToolCall(tc) if tc.id == tool_call_id => tc.raw_input.as_ref(),
        _ => None,
    })
}

/// Summarize `raw_input` into one `"<label>: <value>"` line — `None` when
/// `raw_input` is absent, isn't a JSON object, or none of
/// [`RAW_INPUT_SUMMARY_KEYS`] is present as a non-empty string.
pub(super) fn summarize_raw_input(raw_input: Option<&serde_json::Value>) -> Option<String> {
    let object = raw_input?.as_object()?;
    let (label, value) = RAW_INPUT_SUMMARY_KEYS.iter().find_map(|(key, label)| {
        let value = object.get(*key)?.as_str()?;
        (!value.is_empty()).then_some((*label, value))
    })?;
    let char_count = value.chars().count();
    let value = if char_count > RAW_INPUT_SUMMARY_MAX_CHARS {
        let truncated: String = value.chars().take(RAW_INPUT_SUMMARY_MAX_CHARS).collect();
        format!("{truncated}…")
    } else {
        value.to_string()
    };
    Some(format!("{label}: {value}"))
}

pub(super) fn choice_of(option: &PermissionOption) -> PermissionChoice {
    PermissionChoice {
        option_id: option.option_id.0.to_string(),
        name: option.name.clone(),
        kind: match option.kind {
            PermissionOptionKind::AllowOnce => PermissionKindView::AllowOnce,
            PermissionOptionKind::AllowAlways => PermissionKindView::AllowAlways,
            PermissionOptionKind::RejectOnce => PermissionKindView::RejectOnce,
            PermissionOptionKind::RejectAlways => PermissionKindView::RejectAlways,
            // Unknown future kind: render as a one-time reject (deny by default).
            _ => PermissionKindView::RejectOnce,
        },
    }
}
