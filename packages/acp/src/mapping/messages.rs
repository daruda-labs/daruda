//! Message streaming, turn boundaries, and replayed interrupt markers.

use super::*;

/// Fold a `UserMessageChunk` into the item list.
///
/// The host echoes the user's prompt locally on send (a `UserText` pushed
/// straight into `items`), so an adapter that *also* replays that prompt back
/// as a `UserMessageChunk` would double the bubble. Defensive dedup: skip a
/// chunk that exactly repeats the trailing user text — the local echo is the
/// authoritative copy of a user turn. This can only ever collapse an
/// agent-originated replay of the message already on screen; two genuine
/// consecutive user turns are pushed as separate local echoes and never travel
/// through this path, so they are unaffected. In the observed adapter the
/// replay does not occur, but the guard keeps a future/other adapter from
/// duplicating the turn.
///
/// WORKAROUND: also drop a chunk that is a harness-injected task-notification
/// blob (`<task-notification>…`) rather than real conversation text. The
/// Claude Agent SDK persists background-subagent completions as synthetic
/// `role: "user"` transcript entries; the claude-agent-acp adapter's *live*
/// path deliberately skips any such string-content user message
/// ("these seem to be messages we don't want in the feed" —
/// `acp-agent.js`'s live `session/update` loop), but its `session/load`
/// *replay* path (`replaySessionHistory`) has no equivalent guard — it only
/// strips `<command-*>`/`<local-command-*>` markers, not this one — so a
/// restored pane replays every past task-notification verbatim, including an
/// embedded `<system-reminder>`, straight into the user bubble. Root cause is
/// in the adapter (an npm dependency we don't vendor), so this is a
/// host-side filter until upstream carries the fix.
pub(super) fn append_user_chunk(items: &mut Vec<ChatItem>, text: &str) {
    if text.trim_start().starts_with("<task-notification>") {
        return;
    }
    if is_interrupt_marker(text.trim()) {
        // Structure, not a prompt — see [`ChatItem::Interrupted`]. The host
        // pushes the same marker live, so collapse a repeat rather than
        // stacking two rows for one stop.
        if !matches!(items.last(), Some(ChatItem::Interrupted)) {
            items.push(ChatItem::Interrupted);
        }
        return;
    }
    if matches!(items.last(), Some(ChatItem::UserText(prev)) if prev == text) {
        return;
    }
    items.push(ChatItem::UserText(text.to_string()));
}

/// Whether `text` is one of the Claude Agent SDK's synthetic interrupt entries.
///
/// The SDK interns exactly two today (`[Request interrupted by user]` and
/// `[… for tool use]`), but recognizes its own with a *shape* rather than a
/// list — `/\[Request interrupted by user[^\]]*\]/` in `cli.js`
/// (claude-agent-sdk 2.1.44). This mirrors that shape, so a third variant lands
/// as a marker instead of silently regressing to the user bubble this whole
/// path exists to remove.
///
/// Whole-string on purpose: prose that merely quotes a marker is a real prompt.
pub(super) fn is_interrupt_marker(text: &str) -> bool {
    const PREFIX: &str = "[Request interrupted by user";
    let Some(rest) = text.strip_prefix(PREFIX) else {
        return false;
    };
    // The upstream character class stops at the first `]`, so the closing
    // bracket must be the last character and the only one.
    matches!(rest.strip_suffix(']'), Some(inner) if !inner.contains(']'))
}

#[derive(Clone, Copy)]
pub(super) enum StreamKind {
    Assistant,
    Thinking,
}

/// The still-streaming body this chunk continues, or `None` when it starts a
/// new message.
///
/// The tail alone is not enough. Items reach `items` while a message is still
/// streaming — a background subagent's tool call re-keyed onto this transcript
/// by [`crate::native_subagents`], a permission request, a surfaced failure —
/// and they sit between the message and its own later chunks. Stopping at the
/// tail would start a fresh item there, and the message's markdown would then
/// be parsed in pieces: a table row without its delimiter row is a paragraph of
/// literal pipes, a `**` cut in half stays literal, and a fence splits across
/// the box boundary.
///
/// Only a chunk that names its message can be reunited with it. Without a
/// `message_id`, "same message" and "new message" are indistinguishable, so
/// those keep merging by adjacency and never walk back past anything.
pub(super) fn streaming_body<'a>(
    items: &'a mut [ChatItem],
    message_id: Option<&str>,
    kind: StreamKind,
) -> Option<&'a mut String> {
    for item in items.iter_mut().rev() {
        match (item, kind) {
            (
                ChatItem::AssistantText {
                    text,
                    streaming: true,
                    message_id: mid,
                    // Not matched on: the role was fixed when this message
                    // started, so a chunk that restates it differently is an
                    // adapter contradicting itself, not a new message.
                    phase: _,
                },
                StreamKind::Assistant,
            ) if mid.as_deref() == message_id => return Some(text),
            (
                ChatItem::Thinking {
                    text,
                    streaming: true,
                    message_id: mid,
                },
                StreamKind::Thinking,
            ) if mid.as_deref() == message_id => return Some(text),
            // The three kinds that can be appended while a message streams. The
            // walk stops at everything else, so it is bounded by what arrived
            // during this message rather than by the conversation.
            (ChatItem::ToolCall(_) | ChatItem::Permission(_) | ChatItem::Failure(_), _)
                if message_id.is_some() => {}
            _ => return None,
        }
    }
    None
}

/// Append streamed text to the message it belongs to (see [`streaming_body`]);
/// when there is none, finalize the previous streaming block and start a fresh
/// item. A change in `message_id` therefore splits two agent messages into
/// separate items even with no tool call between them — the protocol's "a
/// change in messageId indicates a new message" (`ContentChunk::message_id`).
/// Empty chunks (the agent emits a leading empty chunk per message) start the
/// item without text.
pub(super) fn append_streaming(
    items: &mut Vec<ChatItem>,
    text: &str,
    message_id: Option<String>,
    kind: StreamKind,
    phase: MessagePhase,
) {
    if let Some(prev) = streaming_body(items, message_id.as_deref(), kind) {
        prev.push_str(text);
        return;
    }
    // A new message (different id, or a kind switch) begins: the previous
    // streaming block, if any, is now complete.
    finalize_streaming(items);
    match kind {
        StreamKind::Assistant => items.push(ChatItem::AssistantText {
            text: text.to_string(),
            streaming: true,
            message_id,
            phase,
        }),
        StreamKind::Thinking => items.push(ChatItem::Thinking {
            text: text.to_string(),
            streaming: true,
            message_id,
        }),
    }
}

/// The owned `message_id` of a streamed content chunk, if the agent supplied
/// one. `MessageId` wraps an `Arc<str>`; we copy it into a `String` so the
/// GPUI-free model carries no protocol type.
pub(super) fn msg_id(chunk: &ContentChunk) -> Option<String> {
    chunk.message_id.as_ref().map(|m| m.0.to_string())
}

/// Extract renderable text from a content block. Non-text blocks (image,
/// audio, resource) collapse to empty for the MVP text view.
pub(super) fn text_of(block: &ContentBlock) -> String {
    match block {
        ContentBlock::Text(t) => t.text.clone(),
        _ => String::new(),
    }
}
