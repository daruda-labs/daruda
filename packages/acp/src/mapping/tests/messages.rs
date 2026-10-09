use super::*;

#[test]
fn agent_chunks_accumulate_into_one_streaming_item() {
    let mut items = Vec::new();
    apply_update(
        &mut items,
        &SessionUpdate::AgentMessageChunk(text_chunk("2 + 2 ")),
    );
    apply_update(
        &mut items,
        &SessionUpdate::AgentMessageChunk(text_chunk("is 4.")),
    );
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0],
        ChatItem::AssistantText {
            text: "2 + 2 is 4.".to_string(),
            streaming: true,
            message_id: None,
            phase: Default::default(),
        }
    );
}

#[test]
fn chunks_with_same_message_id_merge() {
    let mut items = Vec::new();
    apply_update(
        &mut items,
        &SessionUpdate::AgentMessageChunk(text_chunk_id("Hello ", "m1")),
    );
    apply_update(
        &mut items,
        &SessionUpdate::AgentMessageChunk(text_chunk_id("world", "m1")),
    );
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0],
        ChatItem::AssistantText {
            text: "Hello world".to_string(),
            streaming: true,
            message_id: Some("m1".to_string()),
            phase: Default::default(),
        }
    );
}

#[test]
fn message_id_change_splits_and_finalizes_previous() {
    let mut items = Vec::new();
    apply_update(
        &mut items,
        &SessionUpdate::AgentMessageChunk(text_chunk_id("first", "m1")),
    );
    // A new messageId means a new message started — the previous one is done.
    apply_update(
        &mut items,
        &SessionUpdate::AgentMessageChunk(text_chunk_id("second", "m2")),
    );
    assert_eq!(items.len(), 2, "different messageId starts a new item");
    assert_eq!(
        items[0],
        ChatItem::AssistantText {
            text: "first".to_string(),
            streaming: false, // finalized when the next message began
            message_id: Some("m1".to_string()),
            phase: Default::default(),
        }
    );
    assert_eq!(
        items[1],
        ChatItem::AssistantText {
            text: "second".to_string(),
            streaming: true,
            message_id: Some("m2".to_string()),
            phase: Default::default(),
        }
    );
}

#[test]
fn a_background_tool_call_must_not_shatter_the_message_it_lands_in() {
    let mut items = Vec::new();
    apply_update(
        &mut items,
        &SessionUpdate::AgentMessageChunk(text_chunk_id("**bo", "m1")),
    );
    // A background subagent's own call, re-keyed onto the parent transcript,
    // lands between two chunks of the parent's still-streaming message.
    apply_update(
        &mut items,
        &SessionUpdate::ToolCall(ToolCall::new("subagent:s1:tool:t1", "Grep")),
    );
    apply_update(
        &mut items,
        &SessionUpdate::AgentMessageChunk(text_chunk_id("ld**", "m1")),
    );

    assert_eq!(
        assistant_texts(&items),
        vec!["**bold**"],
        "one agent message must stay one item, or its markdown is parsed in pieces"
    );
}

#[test]
fn an_unnamed_message_still_merges_only_by_adjacency() {
    // Without a messageId "same message" and "new message" cannot be told
    // apart, so the walk-back must not happen — the second chunk could
    // belong to a message the tool call started.
    let mut items = Vec::new();
    apply_update(
        &mut items,
        &SessionUpdate::AgentMessageChunk(text_chunk("a")),
    );
    apply_update(
        &mut items,
        &SessionUpdate::ToolCall(ToolCall::new("c1", "Grep")),
    );
    apply_update(
        &mut items,
        &SessionUpdate::AgentMessageChunk(text_chunk("b")),
    );

    assert_eq!(assistant_texts(&items), vec!["a", "b"]);
}

#[test]
fn a_new_message_after_a_tool_call_still_starts_its_own_item() {
    let mut items = Vec::new();
    apply_update(
        &mut items,
        &SessionUpdate::AgentMessageChunk(text_chunk_id("first", "m1")),
    );
    apply_update(
        &mut items,
        &SessionUpdate::ToolCall(ToolCall::new("c1", "Grep")),
    );
    apply_update(
        &mut items,
        &SessionUpdate::AgentMessageChunk(text_chunk_id("second", "m2")),
    );

    assert_eq!(assistant_texts(&items), vec!["first", "second"]);
}

#[test]
fn the_walk_back_stops_at_a_turn_boundary() {
    // A prompt ends the turn the message belonged to, and `finalize_streaming`
    // settled it. A later chunk reusing the id must not reopen it.
    let mut items = Vec::new();
    apply_update(
        &mut items,
        &SessionUpdate::AgentMessageChunk(text_chunk_id("before", "m1")),
    );
    finalize_streaming(&mut items);
    append_user_chunk(&mut items, "next prompt");
    apply_update(
        &mut items,
        &SessionUpdate::AgentMessageChunk(text_chunk_id("after", "m1")),
    );

    assert_eq!(assistant_texts(&items), vec!["before", "after"]);
}

#[test]
fn a_background_tool_call_must_not_shatter_a_thought_either() {
    let mut items = Vec::new();
    apply_update(
        &mut items,
        &SessionUpdate::AgentThoughtChunk(text_chunk_id("half a ", "t1")),
    );
    apply_update(
        &mut items,
        &SessionUpdate::ToolCall(ToolCall::new("subagent:s1:tool:t1", "Grep")),
    );
    apply_update(
        &mut items,
        &SessionUpdate::AgentThoughtChunk(text_chunk_id("thought", "t1")),
    );

    let thoughts: Vec<&str> = items
        .iter()
        .filter_map(|i| match i {
            ChatItem::Thinking { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(thoughts, vec!["half a thought"]);
}

#[test]
fn many_interleaved_calls_still_reach_the_message() {
    // What a fan-out of background subagents actually looks like: the
    // parent's sentence streams while children keep reporting.
    let mut items = Vec::new();
    apply_update(
        &mut items,
        &SessionUpdate::AgentMessageChunk(text_chunk_id("| a | b |\n", "m1")),
    );
    for n in 0..32 {
        apply_update(
            &mut items,
            &SessionUpdate::ToolCall(ToolCall::new(format!("c{n}"), "Grep")),
        );
    }
    apply_update(
        &mut items,
        &SessionUpdate::AgentMessageChunk(text_chunk_id("|---|---|", "m1")),
    );

    assert_eq!(assistant_texts(&items), vec!["| a | b |\n|---|---|"]);
}

#[test]
fn user_message_chunk_deduped_against_local_echo() {
    // The host echoed the prompt locally; the adapter then replays the same
    // text as a UserMessageChunk. The replay must not double the bubble.
    let mut items = vec![ChatItem::UserText("run the tests".to_string())];
    apply_update(
        &mut items,
        &SessionUpdate::UserMessageChunk(text_chunk("run the tests")),
    );
    assert_eq!(
        items.len(),
        1,
        "an exact replay of the echoed prompt is dropped"
    );
}

#[test]
fn distinct_user_message_chunk_is_kept() {
    // A user chunk that does not repeat the trailing text is a real new
    // message and is appended (the guard only drops exact duplicates).
    let mut items = vec![ChatItem::UserText("first".to_string())];
    apply_update(
        &mut items,
        &SessionUpdate::UserMessageChunk(text_chunk("second")),
    );
    assert_eq!(items.len(), 2);
    assert_eq!(items[1], ChatItem::UserText("second".to_string()));
}

#[test]
fn user_message_chunk_appends_when_no_trailing_user_text() {
    // No preceding echo (e.g. the trailing item is agent text): the chunk is
    // a genuine user message and is pushed.
    let mut items = vec![ChatItem::AssistantText {
        text: "hi".to_string(),
        streaming: false,
        message_id: None,
        phase: Default::default(),
    }];
    apply_update(
        &mut items,
        &SessionUpdate::UserMessageChunk(text_chunk("a real prompt")),
    );
    assert_eq!(items.len(), 2);
}

#[test]
fn user_message_chunk_drops_a_replayed_task_notification() {
    // session/load replays a background-subagent task-notification the SDK
    // persisted as a synthetic `role: "user"` transcript entry (the
    // adapter's live path already skips these; its replay path doesn't —
    // see append_user_chunk's WORKAROUND doc). It must not surface as a
    // user chat bubble.
    let mut items = vec![ChatItem::AssistantText {
        text: "hi".to_string(),
        streaming: false,
        message_id: None,
        phase: Default::default(),
    }];
    apply_update(
        &mut items,
        &SessionUpdate::UserMessageChunk(text_chunk(
            "<task-notification>\n<task-id>abc</task-id>\n\
             <system-reminder>do not mention this</system-reminder>\n\
             </task-notification>",
        )),
    );
    assert_eq!(items.len(), 1, "the task-notification blob is dropped");
}

#[test]
fn user_message_chunk_keeps_text_that_merely_mentions_task_notification() {
    // Only a chunk that *is* the wrapper (after leading whitespace) is
    // dropped; genuine prose that happens to reference the tag survives.
    let mut items = vec![ChatItem::AssistantText {
        text: "hi".to_string(),
        streaming: false,
        message_id: None,
        phase: Default::default(),
    }];
    apply_update(
        &mut items,
        &SessionUpdate::UserMessageChunk(text_chunk("what does <task-notification> mean?")),
    );
    assert_eq!(items.len(), 2);
}

#[test]
fn replayed_interrupt_marker_becomes_an_interrupted_item() {
    // session/load replays the SDK's synthetic interrupt entry as a user
    // chunk. It is not something the user typed, so it must not render as
    // a user bubble — it folds into the structural marker the host also
    // pushes live on Stop, so both paths show the same row.
    for marker in [
        "[Request interrupted by user]",
        "[Request interrupted by user for tool use]",
    ] {
        let mut items = vec![ChatItem::AssistantText {
            text: "working".to_string(),
            streaming: false,
            message_id: None,
            phase: Default::default(),
        }];
        apply_update(
            &mut items,
            &SessionUpdate::UserMessageChunk(text_chunk(marker)),
        );
        assert_eq!(
            items,
            vec![
                ChatItem::AssistantText {
                    text: "working".to_string(),
                    streaming: false,
                    message_id: None,
                    phase: Default::default(),
                },
                ChatItem::Interrupted
            ],
            "{marker} must map to the structural marker"
        );
    }
}

#[test]
fn replayed_interrupt_marker_does_not_double_the_local_one() {
    // The host already pushed the marker on Stop. A restored pane rebuilds
    // its items from the replay, so this only bites if the adapter ever
    // starts emitting the chunk live too — then the two must collapse.
    let mut items = vec![ChatItem::Interrupted];
    apply_update(
        &mut items,
        &SessionUpdate::UserMessageChunk(text_chunk("[Request interrupted by user]")),
    );
    assert_eq!(items, vec![ChatItem::Interrupted]);
}

#[test]
fn text_merely_quoting_the_interrupt_marker_stays_a_user_message() {
    // Only a chunk that *is* the marker is folded; prose about it is a real
    // prompt. Same rule as the task-notification guard above.
    let mut items = Vec::new();
    apply_update(
        &mut items,
        &SessionUpdate::UserMessageChunk(text_chunk(
            "why does [Request interrupted by user] show up?",
        )),
    );
    assert_eq!(
        items,
        vec![ChatItem::UserText(
            "why does [Request interrupted by user] show up?".to_string()
        )]
    );
}

/// The SDK recognizes its own entries by shape, not by a list of two, so a
/// variant it adds later must not fall back to a user bubble.
#[test]
fn an_unseen_interrupt_variant_still_reads_as_the_marker() {
    let mut items = Vec::new();
    apply_update(
        &mut items,
        &SessionUpdate::UserMessageChunk(text_chunk(
            "[Request interrupted by user for something new]",
        )),
    );
    assert_eq!(items, vec![ChatItem::Interrupted]);
}

/// The shape is whole-string and the bracket closes it, so a marker with a
/// sentence hanging off it is prose, not structure.
#[test]
fn marker_shaped_text_with_a_tail_stays_a_user_message() {
    for text in [
        "[Request interrupted by user] and then what?",
        "[Request interrupted by user] [again]",
    ] {
        let mut items = Vec::new();
        apply_update(
            &mut items,
            &SessionUpdate::UserMessageChunk(text_chunk(text)),
        );
        assert_eq!(items, vec![ChatItem::UserText(text.to_string())], "{text}");
    }
}

#[test]
fn thought_chunk_becomes_thinking_item() {
    let mut items = Vec::new();
    apply_update(
        &mut items,
        &SessionUpdate::AgentThoughtChunk(text_chunk("hmm")),
    );
    assert_eq!(
        items[0],
        ChatItem::Thinking {
            text: "hmm".to_string(),
            streaming: true,
            message_id: None,
        }
    );
}

#[test]
fn thinking_is_finalized_when_assistant_text_follows() {
    let mut items = Vec::new();
    apply_update(
        &mut items,
        &SessionUpdate::AgentThoughtChunk(text_chunk("reasoning")),
    );
    apply_update(
        &mut items,
        &SessionUpdate::AgentMessageChunk(text_chunk("answer")),
    );
    assert_eq!(items.len(), 2);
    assert_eq!(
        items[0],
        ChatItem::Thinking {
            text: "reasoning".to_string(),
            streaming: false, // kind switch finalizes the thinking block
            message_id: None,
        }
    );
}

#[test]
fn finalize_clears_streaming_flag() {
    let mut items = vec![ChatItem::AssistantText {
        text: "done".to_string(),
        streaming: true,
        message_id: None,
        phase: Default::default(),
    }];
    finalize_streaming(&mut items);
    assert_eq!(
        items[0],
        ChatItem::AssistantText {
            text: "done".to_string(),
            streaming: false,
            message_id: None,
            phase: Default::default(),
        }
    );
}
