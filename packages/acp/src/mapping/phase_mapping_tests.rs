use super::*;
use crate::adapter::{CodexAdapter, MessagePhase};
use agent_client_protocol::schema::v1::{ContentChunk, TextContent};

fn chunk(text: &str, id: &str, phase: &str) -> ContentChunk {
    let mut c =
        ContentChunk::new(ContentBlock::Text(TextContent::new(text.to_string()))).message_id(id);
    c.meta = Some(
        serde_json::json!({"codex": {"phase": phase}})
            .as_object()
            .unwrap()
            .clone(),
    );
    c
}

fn phase_of(item: &ChatItem) -> MessagePhase {
    match item {
        ChatItem::AssistantText { phase, .. } => *phase,
        other => panic!("expected assistant text, got {other:?}"),
    }
}

#[test]
fn a_labelled_message_carries_its_phase_into_the_model() {
    let mut items = Vec::new();
    apply_update_with(
        &mut items,
        &SessionUpdate::AgentMessageChunk(chunk("looking", "msg_1", "commentary")),
        &CodexAdapter,
    );
    apply_update_with(
        &mut items,
        &SessionUpdate::AgentMessageChunk(chunk("done", "msg_2", "final_answer")),
        &CodexAdapter,
    );
    assert_eq!(phase_of(&items[0]), MessagePhase::Commentary);
    assert_eq!(phase_of(&items[1]), MessagePhase::Answer);
}

/// A message's role is fixed when it starts. Letting a later chunk restate
/// it would make "the role changed mid-message" representable, and the only
/// way to reach it is an adapter contradicting itself.
#[test]
fn a_later_chunk_of_the_same_message_cannot_change_its_phase() {
    let mut items = Vec::new();
    apply_update_with(
        &mut items,
        &SessionUpdate::AgentMessageChunk(chunk("look", "msg_1", "commentary")),
        &CodexAdapter,
    );
    apply_update_with(
        &mut items,
        &SessionUpdate::AgentMessageChunk(chunk("ing", "msg_1", "final_answer")),
        &CodexAdapter,
    );
    assert_eq!(items.len(), 1, "same message id, so one item");
    assert_eq!(phase_of(&items[0]), MessagePhase::Commentary);
}

/// An agent that labels nothing produces exactly what it did before.
#[test]
fn an_unlabelled_message_is_an_answer() {
    let mut items = Vec::new();
    apply_update(
        &mut items,
        &SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(TextContent::new(
            "hi".to_string(),
        )))),
    );
    assert_eq!(phase_of(&items[0]), MessagePhase::Answer);
}
