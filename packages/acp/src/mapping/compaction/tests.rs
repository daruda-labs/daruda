use super::*;
use serde_json::{Value, json};

fn apply(items: &mut Vec<ChatItem>, session: &str, update: Value) -> usize {
    apply_compaction_update(items, session, &serde_json::from_value(update).unwrap()).unwrap()
}

fn item(items: &[ChatItem], index: usize) -> &CompactionItem {
    let ChatItem::Compaction(item) = &items[index] else {
        panic!("compaction")
    };
    item
}

fn text(item: &CompactionItem) -> String {
    item.summary
        .iter()
        .filter_map(|block| match block {
            ToolOutputBlock::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn update(status: &str) -> Value {
    json!({"sessionUpdate":"compaction_update", "compactionId":"c", "status":status})
}

fn chunk(text: &str) -> Value {
    json!({"sessionUpdate":"compaction_summary_chunk", "compactionId":"c", "content":{"type":"text", "text":text}})
}

#[test]
fn chunk_first_anchors_one_session_scoped_entity_and_late_chunks_append() {
    let mut items = vec![ChatItem::UserText("before".into())];
    assert_eq!(apply(&mut items, "root", chunk("## Re")), 1);
    items.push(ChatItem::UserText("after".into()));
    assert_eq!(apply(&mut items, "root", chunk("tained\n")), 1);
    assert_eq!(apply(&mut items, "child", update("completed")), 3);
    for status in ["completed", "failed", "cancelled", "future", "_paused"] {
        assert_eq!(apply(&mut items, "root", update(status)), 1);
        apply(&mut items, "root", chunk(status));
    }
    assert_eq!(items.len(), 4);
    assert_eq!(
        item(&items, 1).status,
        CompactionStatusView::Other("_paused".into())
    );
    assert_eq!(item(&items, 3).status, CompactionStatusView::Completed);
    assert!(text(item(&items, 1)).starts_with("## Retained\ncompleted"));
    assert!(text(item(&items, 1)).ends_with("_paused"));
    assert!(matches!(&items[2], ChatItem::UserText(text) if text == "after"));
}

#[test]
fn live_and_replay_patch_semantics_distinguish_omission_null_and_empty() {
    let mut items = Vec::new();
    let mut first = update("failed");
    first["summary"] = json!([{"type":"text", "text":"snapshot"}]);
    first["error"] = json!("failure detail");
    first["_meta"] = json!({"provider":"a"});
    apply(&mut items, "root", first);
    apply(&mut items, "root", chunk(" + chunk"));
    // A replay or terminal update with omitted fields retains earlier values.
    apply(&mut items, "root", update("completed"));
    assert_eq!(text(item(&items, 0)), "snapshot + chunk");
    assert_eq!(item(&items, 0).error.as_deref(), Some("failure detail"));
    assert_eq!(item(&items, 0).meta.as_ref().unwrap()["provider"], "a");

    let mut replacement = update("completed");
    replacement["summary"] = json!([{"type":"text", "text":"replacement"}]);
    replacement["error"] = Value::Null;
    replacement["_meta"] = json!({"other":true});
    apply(&mut items, "root", replacement);
    apply(&mut items, "root", chunk(" tail"));
    assert_eq!(text(item(&items, 0)), "replacement tail");
    assert_eq!(item(&items, 0).error, None);
    assert_eq!(item(&items, 0).meta.as_ref().unwrap().len(), 1);

    for clear in [Value::Null, json!([])] {
        let mut clearing = update("cancelled");
        clearing["summary"] = clear;
        clearing["_meta"] = Value::Null;
        apply(&mut items, "root", clearing);
        assert!(item(&items, 0).summary.is_empty());
        assert_eq!(item(&items, 0).meta, None);
        apply(&mut items, "root", chunk("new"));
        assert_eq!(text(item(&items, 0)), "new");
    }
}

#[test]
fn mixed_content_is_retained_and_streamed_text_is_bounded_on_utf8() {
    let mut items = Vec::new();
    let mut mixed = update("in_progress");
    mixed["summary"] = json!([
        {"type":"text", "text":"start"},
        {"type":"image", "data":"AA==", "mimeType":"image/png"},
        {"type":"resource_link", "name":"context", "uri":"file:///tmp/context.md"},
        {"type":"audio", "data":"AA==", "mimeType":"audio/wav"}
    ]);
    apply(&mut items, "root", mixed);
    assert_eq!(item(&items, 0).summary.len(), 4);
    assert!(matches!(
        item(&items, 0).summary[1],
        ToolOutputBlock::Image { .. }
    ));
    assert!(matches!(
        item(&items, 0).summary[2],
        ToolOutputBlock::ResourceLink { .. }
    ));
    assert!(matches!(
        item(&items, 0).summary[3],
        ToolOutputBlock::Media { .. }
    ));
    let long = "한".repeat(30_000);
    apply(&mut items, "root", chunk(&long));
    apply(&mut items, "root", chunk("tail"));
    let ToolOutputBlock::Text {
        text,
        truncated_from,
    } = item(&items, 0).summary.last().unwrap()
    else {
        panic!("text")
    };
    assert!(text.len() <= super::super::content::MAX_TOOL_OUTPUT_TEXT_BYTES);
    assert_eq!(*truncated_from, Some(long.len() + 4));
}

#[test]
fn compaction_does_not_split_an_identified_message_but_anchors_anonymous_messages() {
    for message_id in [Some("message"), None] {
        let mut items = Vec::new();
        let mut prose = json!({"sessionUpdate":"agent_message_chunk", "content":{"type":"text", "text":"before"}});
        if let Some(id) = message_id {
            prose["messageId"] = json!(id);
        }
        apply_update(&mut items, &serde_json::from_value(prose.clone()).unwrap());
        apply(&mut items, "root", update("completed"));
        prose["content"]["text"] = json!("after");
        apply_update(&mut items, &serde_json::from_value(prose).unwrap());
        if message_id.is_some() {
            assert_eq!(items.len(), 2);
            assert!(
                matches!(&items[0], ChatItem::AssistantText { text, .. } if text == "beforeafter")
            );
        } else {
            assert_eq!(items.len(), 3);
            assert!(matches!(&items[2], ChatItem::AssistantText { text, .. } if text == "after"));
        }
    }
}
