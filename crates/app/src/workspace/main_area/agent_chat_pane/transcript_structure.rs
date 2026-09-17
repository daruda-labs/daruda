//! Where a conversation's boundaries are: where a response ends, which
//! consecutive tool calls form one run, which row owns a nested call, and
//! whether anything in a range is still going.
//!
//! One home for every such rule, so the row walk, the fold decision, and the
//! click handler cannot answer the same question differently — they did, and a
//! group held itself open on a call that belonged to an inner card.
//!
//! Split by what each question needs rather than gathered under one type: a
//! response boundary reads `items` alone, while a tool run has to know which
//! calls are nested. [`TranscriptStructure`] carries the nesting-aware half and
//! *borrows* a hierarchy the caller already built — building one per query puts
//! a map allocation behind every response bar on the paint path, which the row
//! walk's linearity guard rejects.

use std::ops::Range;

use daruda_acp::ChatItem;

use super::tool_hierarchy::ToolHierarchy;

/// The agent run starting at `start`: every item up to (not including) the next
/// user message or the stop marker that cut it, whichever comes first, else the
/// end of the conversation.
///
/// A marker closes the run it cut so the response bar summarizes what actually
/// ran and the marker stays a top-level row instead of folding away with the
/// response. An empty range when `start` is past the end, so a prompt with no
/// reply yet is not a special case.
pub(in crate::workspace) fn response_run(items: &[ChatItem], start: usize) -> Range<usize> {
    let end = items
        .iter()
        .skip(start)
        .position(|item| matches!(item, ChatItem::UserText(_) | ChatItem::Interrupted))
        .map_or(items.len(), |offset| start + offset);
    start.min(end)..end
}

/// Whether an item is still being produced — the input to
/// [`FoldPolicy::ExpandedWhileActive`].
///
/// [`FoldPolicy::ExpandedWhileActive`]: super::fold::FoldPolicy::ExpandedWhileActive
pub(in crate::workspace) fn is_active(item: &ChatItem) -> bool {
    match item {
        ChatItem::AssistantText { streaming, .. } | ChatItem::Thinking { streaming, .. } => {
            *streaming
        }
        ChatItem::ToolCall(tc) => tc.status.is_live(),
        ChatItem::UserText(_)
        | ChatItem::Permission(_)
        | ChatItem::Failure(_)
        | ChatItem::Interrupted => false,
    }
}

/// Whether anything in `run` is active. Out-of-range indices read inactive
/// rather than panicking, so a stale key resolves to "settled".
pub(in crate::workspace) fn run_active(items: &[ChatItem], run: Range<usize>) -> bool {
    items.get(run).is_some_and(|run| run.iter().any(is_active))
}

/// The nesting-aware boundary questions, over a hierarchy the caller owns.
pub(in crate::workspace) struct TranscriptStructure<'a> {
    items: &'a [ChatItem],
    hierarchy: &'a ToolHierarchy<'a>,
}

impl<'a> TranscriptStructure<'a> {
    pub(in crate::workspace) fn new(
        items: &'a [ChatItem],
        hierarchy: &'a ToolHierarchy<'a>,
    ) -> Self {
        Self { items, hierarchy }
    }

    /// Whether the item at `ix` is a tool call that earns a row of its own.
    pub(in crate::workspace) fn top_level_tool(&self, ix: usize) -> bool {
        matches!(self.items.get(ix), Some(ChatItem::ToolCall(tc)) if !self.hierarchy.is_nested_child(tc))
    }

    /// The maximal stretch of consecutive top-level tool calls beginning at
    /// `start`, bounded by `limit`. A nested child renders inside its parent's
    /// card, so it breaks a run rather than joining it.
    pub(in crate::workspace) fn tool_run(&self, start: usize, limit: usize) -> Range<usize> {
        let mut k = start + 1;
        while k < limit && self.top_level_tool(k) {
            k += 1;
        }
        start..k
    }

    /// How many separate top-level tool runs `run` holds. One run means the
    /// group bar below carries the whole tally, which is what the turn bar
    /// checks before deciding whether repeating it would say anything.
    pub(in crate::workspace) fn top_level_tool_runs(&self, run: Range<usize>) -> usize {
        let limit = run.end;
        let mut runs = 0;
        let mut k = run.start;
        while k < limit {
            if self.top_level_tool(k) {
                runs += 1;
                k = self.tool_run(k, limit).end;
            } else {
                k += 1;
            }
        }
        runs
    }

    /// The `items` index of the call that owns a row for `id` — itself when its
    /// parent is absent from `items`, else the nearest present ancestor.
    pub(in crate::workspace) fn owning_item(&self, id: &str) -> Option<usize> {
        self.hierarchy.owning_row_index(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asst(streaming: bool) -> ChatItem {
        ChatItem::AssistantText {
            text: "a".to_owned(),
            streaming,
            message_id: None,
            phase: Default::default(),
        }
    }

    fn tool(id: &str, parent: Option<&str>, live: bool) -> ChatItem {
        ChatItem::ToolCall(daruda_acp::ToolCallItem {
            id: id.to_owned(),
            title: "t".to_owned(),
            kind: daruda_acp::ToolKindView::Edit,
            tool_name: None,
            status: if live {
                daruda_acp::ToolStatusView::InProgress
            } else {
                daruda_acp::ToolStatusView::Completed
            },
            diffs: Vec::new(),
            output: Vec::new(),
            raw_input: None,
            locations: Vec::new(),
            parent_tool_id: parent.map(str::to_owned),
            exit: None,
        })
    }

    /// Both terminators, and the empty run a prompt with no reply yields.
    #[test]
    fn a_response_run_ends_at_the_next_prompt_or_the_marker() {
        let items = [
            ChatItem::UserText("q".to_owned()),
            asst(false),
            ChatItem::Interrupted,
            asst(false),
            ChatItem::UserText("q2".to_owned()),
            asst(false),
        ];
        assert_eq!(response_run(&items, 1), 1..2, "the marker closes the run");
        assert_eq!(response_run(&items, 3), 3..4, "the next prompt closes it");
        assert_eq!(response_run(&items, 5), 5..6);
        assert_eq!(response_run(&items, 6), 6..6, "nothing follows the prompt");
        assert_eq!(response_run(&items, 99), 6..6, "past the end is empty");
    }

    /// A nested child breaks the run; a child whose declared parent `items`
    /// never carried stays in it, because presence is what decides row
    /// ownership.
    #[test]
    fn a_tool_run_stops_at_a_nested_child_but_not_at_an_orphan() {
        let items = [
            tool("a", None, false),
            tool("b", None, false),
            tool("c", Some("a"), false),
            tool("d", None, false),
        ];
        let h = ToolHierarchy::build(&items);
        let s = TranscriptStructure::new(&items, &h);
        assert_eq!(s.tool_run(0, items.len()), 0..2);
        assert_eq!(s.top_level_tool_runs(0..items.len()), 2, "a,b then d");

        let orphans = [tool("a", None, false), tool("b", Some("gone"), false)];
        let h = ToolHierarchy::build(&orphans);
        let s = TranscriptStructure::new(&orphans, &h);
        assert_eq!(s.tool_run(0, orphans.len()), 0..2, "an orphan stays in");
        assert_eq!(s.top_level_tool_runs(0..orphans.len()), 1);
    }

    /// A range holding a live call reads active; the same range without one
    /// does not, and neither does an out-of-range one.
    #[test]
    fn run_active_reads_the_range_it_is_given() {
        let items = [asst(false), tool("a", None, true), asst(false)];
        assert!(run_active(&items, 0..3));
        assert!(!run_active(&items, 2..3));
        assert!(!run_active(&items, 0..99), "a stale range reads settled");
    }
}
