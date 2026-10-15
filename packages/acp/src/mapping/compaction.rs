//! ID-addressed upserts. Receive order fixes placement, independent of status.

use agent_client_protocol::schema::{MaybeUndefined, v1::CompactionStatus};

use super::content::output_block_of;
use super::*;
use crate::model::{CompactionItem, CompactionStatusView};

/// Apply a session-scoped compaction notification and return its stable item
/// index. Omitted fields retain their values during live delivery and replay.
pub fn apply_compaction_update(
    items: &mut Vec<ChatItem>,
    session_id: &str,
    update: &SessionUpdate,
) -> Option<usize> {
    let id = match update {
        SessionUpdate::CompactionUpdate(u) => &u.compaction_id,
        SessionUpdate::CompactionSummaryChunk(u) => &u.compaction_id,
        _ => return None,
    };
    let index = items.iter().position(|item| {
        matches!(item, ChatItem::Compaction(c) if c.session_id == session_id && c.id == id.0.as_ref())
    }).unwrap_or_else(|| {
        items.push(ChatItem::Compaction(Box::new(CompactionItem::new(session_id, id.to_string()))));
        items.len() - 1
    });
    let ChatItem::Compaction(item) = &mut items[index] else {
        return None;
    };
    match update {
        SessionUpdate::CompactionUpdate(update) => {
            item.status = match &update.status {
                CompactionStatus::InProgress => CompactionStatusView::InProgress,
                CompactionStatus::Completed => CompactionStatusView::Completed,
                CompactionStatus::Failed => CompactionStatusView::Failed,
                CompactionStatus::Cancelled => CompactionStatusView::Cancelled,
                CompactionStatus::Other(value) => CompactionStatusView::Other(value.clone()),
                other => CompactionStatusView::Other(format!("{other:?}")),
            };
            match &update.summary {
                MaybeUndefined::Undefined => {}
                MaybeUndefined::Null => item.summary.clear(),
                MaybeUndefined::Value(blocks) => {
                    item.summary = blocks.iter().filter_map(output_block_of).collect();
                }
            }
            patch(&mut item.error, &update.error);
            patch(&mut item.meta, &update.meta);
        }
        SessionUpdate::CompactionSummaryChunk(chunk) => {
            if let Some(block) = output_block_of(&chunk.content) {
                append_summary(&mut item.summary, block);
            }
        }
        _ => {}
    }
    Some(index)
}

fn append_summary(summary: &mut Vec<ToolOutputBlock>, block: ToolOutputBlock) {
    if let (
        Some(ToolOutputBlock::Text {
            text,
            truncated_from,
        }),
        ToolOutputBlock::Text {
            text: next,
            truncated_from: next_original,
        },
    ) = (summary.last_mut(), &block)
    {
        let original = truncated_from.unwrap_or(text.len()) + next_original.unwrap_or(next.len());
        let (kept, _) = super::content::bounded(format!("{text}{next}"));
        *text = kept;
        *truncated_from = (original > text.len()).then_some(original);
    } else {
        summary.push(block);
    }
}

fn patch<T: Clone>(slot: &mut Option<T>, patch: &MaybeUndefined<T>) {
    match patch {
        MaybeUndefined::Undefined => {}
        MaybeUndefined::Null => *slot = None,
        MaybeUndefined::Value(value) => *slot = Some(value.clone()),
    }
}

#[cfg(test)]
mod tests;
