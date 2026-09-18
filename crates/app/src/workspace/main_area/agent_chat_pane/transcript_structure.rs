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

use super::rows::is_bodyless;
use super::tool_hierarchy::ToolHierarchy;

/// The agent run starting at `start`: every item up to (not including) the next
/// user message or the stop marker that cut it, whichever comes first, else the
/// end of the conversation.
///
/// A marker closes the run it cut so the response bar summarizes what actually
/// ran and the marker stays a top-level row instead of folding away with the
/// response. An empty range when `start` is past the end, so a prompt with no
/// reply yet is not a special case.
pub(super) fn response_run(items: &[ChatItem], start: usize) -> Range<usize> {
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
pub(super) fn is_active(item: &ChatItem) -> bool {
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
pub(super) fn run_active(items: &[ChatItem], run: Range<usize>) -> bool {
    items.get(run).is_some_and(|run| run.iter().any(is_active))
}

/// The nesting-aware boundary questions, over a hierarchy the caller owns.
///
/// Two shared borrows, so it is `Copy`: a query can hand its own value to an
/// iterator it returns without tying that iterator to a temporary.
#[derive(Clone, Copy)]
pub(super) struct TranscriptStructure<'a> {
    items: &'a [ChatItem],
    hierarchy: &'a ToolHierarchy<'a>,
}

impl<'a> TranscriptStructure<'a> {
    pub(super) fn new(items: &'a [ChatItem], hierarchy: &'a ToolHierarchy<'a>) -> Self {
        Self { items, hierarchy }
    }

    /// Whether the item at `ix` is a tool call that earns a row of its own.
    pub(super) fn top_level_tool(&self, ix: usize) -> bool {
        matches!(self.items.get(ix), Some(ChatItem::ToolCall(tc)) if !self.hierarchy.is_nested_child(tc))
    }

    /// Whether the item at `ix` earns a row of its own at all. A nested child
    /// renders inside its parent's card and an empty streaming chunk renders
    /// nothing, so the row walk passes over both.
    pub(super) fn owns_a_row(&self, ix: usize) -> bool {
        match self.items.get(ix) {
            Some(ChatItem::ToolCall(tc)) => !self.hierarchy.is_nested_child(tc),
            Some(item) => !is_bodyless(item),
            None => false,
        }
    }

    /// The stretch of top-level tool calls beginning at `start`, bounded by
    /// `limit`. It ends at the first item that owns a row and is not one of
    /// them — an item the walk passes over is spanned, not a boundary, or two
    /// cards the reader sees side by side would land in separate groups.
    ///
    /// The range is what to walk, not what the group holds: ask
    /// [`Self::group_calls`] for the members.
    pub(super) fn tool_run(&self, start: usize, limit: usize) -> Range<usize> {
        let mut k = start + 1;
        while k < limit && (!self.owns_a_row(k) || self.top_level_tool(k)) {
            k += 1;
        }
        start..k
    }

    /// The calls a run's group holds — every row-owning item it spans, which by
    /// [`Self::tool_run`]'s boundary is exactly its top-level tool calls.
    pub(super) fn group_calls(self, run: Range<usize>) -> impl Iterator<Item = usize> + Clone + 'a {
        run.filter(move |&ix| self.top_level_tool(ix))
    }

    /// How many separate top-level tool runs `run` holds. One run means the
    /// group bar below carries the whole tally, which is what the turn bar
    /// checks before deciding whether repeating it would say anything.
    pub(super) fn top_level_tool_runs(&self, run: Range<usize>) -> usize {
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
    pub(super) fn owning_item(&self, id: &str) -> Option<usize> {
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

    /// A nested child is spanned but never counted; a child whose declared
    /// parent `items` never carried is a member like any other call, because
    /// presence is what decides row ownership.
    #[test]
    fn a_tool_run_spans_a_nested_child_but_never_counts_it() {
        let items = [
            tool("a", None, false),
            tool("b", None, false),
            tool("c", Some("a"), false),
            tool("d", None, false),
        ];
        let h = ToolHierarchy::build(&items);
        let s = TranscriptStructure::new(&items, &h);
        assert_eq!(s.tool_run(0, items.len()), 0..4);
        assert_eq!(
            s.group_calls(0..4).collect::<Vec<_>>(),
            [0, 1, 3],
            "c owns no row"
        );
        assert_eq!(
            s.top_level_tool_runs(0..items.len()),
            1,
            "a,b,d are one run"
        );

        let orphans = [tool("a", None, false), tool("b", Some("gone"), false)];
        let h = ToolHierarchy::build(&orphans);
        let s = TranscriptStructure::new(&orphans, &h);
        assert_eq!(s.tool_run(0, orphans.len()), 0..2, "an orphan stays in");
        assert_eq!(s.group_calls(0..2).collect::<Vec<_>>(), [0, 1]);
        assert_eq!(s.top_level_tool_runs(0..orphans.len()), 1);
    }

    /// A nested child renders inside its parent's card and an empty streaming
    /// chunk renders nowhere, so neither owns a row. A run has to span them:
    /// ending at one puts two cards the reader sees side by side into separate
    /// groups.
    #[test]
    fn a_tool_run_spans_the_items_that_own_no_row() {
        let items = [
            tool("a", None, false),
            tool("a1", Some("a"), false),
            tool("b", None, false),
            tool("b1", Some("b"), false),
            tool("c", None, false),
        ];
        let h = ToolHierarchy::build(&items);
        let s = TranscriptStructure::new(&items, &h);
        assert_eq!(s.tool_run(0, items.len()), 0..5, "the children are spanned");
        assert_eq!(
            s.group_calls(0..5).collect::<Vec<_>>(),
            [0, 2, 4],
            "the launches are the members"
        );
        assert_eq!(
            s.top_level_tool_runs(0..items.len()),
            1,
            "one run, not three"
        );
    }

    /// The other item the row walk passes over: a streaming chunk with nothing
    /// in it yet.
    #[test]
    fn an_empty_streaming_chunk_does_not_split_a_tool_run() {
        let items = [
            tool("a", None, false),
            ChatItem::AssistantText {
                text: String::new(),
                streaming: true,
                message_id: None,
                phase: Default::default(),
            },
            tool("b", None, false),
        ];
        let h = ToolHierarchy::build(&items);
        let s = TranscriptStructure::new(&items, &h);
        assert_eq!(s.tool_run(0, items.len()), 0..3);
        assert_eq!(s.top_level_tool_runs(0..items.len()), 1);
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
