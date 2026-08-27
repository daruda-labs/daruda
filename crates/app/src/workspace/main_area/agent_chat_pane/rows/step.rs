//! Partitions a response into tool runs plus their preceding prose. Trailing
//! prose remains outside every step as the response conclusion.

use std::collections::HashSet;
use std::ops::Range;

use daruda_acp::ChatItem;

use super::{is_nested_child, is_tool_call};

/// Minimum projected row count that earns a step header.
const STEP_MIN_ROWS: usize = 2;

/// Half-open item span for one step.
pub(in crate::workspace) struct StepSpan {
    pub(in crate::workspace) start: usize,
    pub(in crate::workspace) tool_start: usize,
    pub(in crate::workspace) end: usize,
}

pub(super) struct Step {
    pub(super) span: StepSpan,
    /// Nested subagent tools do not count toward the header.
    pub(super) tool_count: usize,
}

/// Partition a response into disjoint steps that earn headers.
pub(super) fn steps(items: &[ChatItem], run: Range<usize>, tool_ids: &HashSet<&str>) -> Vec<Step> {
    let mut out = Vec::new();
    let mut start = run.start;
    let mut k = run.start;
    while k < run.end {
        if !is_tool_call(&items[k]) {
            k += 1;
            continue;
        }
        let tool_start = k;
        while k < run.end && is_tool_call(&items[k]) {
            k += 1;
        }
        let span = StepSpan {
            start,
            tool_start,
            end: k,
        };
        if folded_rows(items, &span, tool_ids) >= STEP_MIN_ROWS {
            let tool_count = (span.tool_start..span.end)
                .filter(|&j| top_level_tool(items, j, tool_ids))
                .count();
            out.push(Step { span, tool_count });
        }
        start = k;
    }
    out
}

/// Recover the step keyed by `first_ix`.
pub(in crate::workspace) fn step_span_at(items: &[ChatItem], first_ix: usize) -> Option<StepSpan> {
    let mut k = first_ix;
    while k < items.len() && !is_tool_call(&items[k]) {
        if matches!(items[k], ChatItem::UserText(_)) {
            return None;
        }
        k += 1;
    }
    let tool_start = k;
    if tool_start >= items.len() {
        return None;
    }
    while k < items.len() && is_tool_call(&items[k]) {
        k += 1;
    }
    Some(StepSpan {
        start: first_ix,
        tool_start,
        end: k,
    })
}

/// Count the projected rows a step header would fold.
fn folded_rows(items: &[ChatItem], span: &StepSpan, tool_ids: &HashSet<&str>) -> usize {
    let prose = span.tool_start - span.start;
    let mut chunks = 0;
    let mut k = span.tool_start;
    while k < span.end {
        if !top_level_tool(items, k, tool_ids) {
            k += 1;
            continue;
        }
        chunks += 1;
        k += 1;
        while k < span.end && top_level_tool(items, k, tool_ids) {
            k += 1;
        }
    }
    prose + chunks
}

fn top_level_tool(items: &[ChatItem], ix: usize, tool_ids: &HashSet<&str>) -> bool {
    matches!(&items[ix], ChatItem::ToolCall(tc) if !is_nested_child(tool_ids, tc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use daruda_acp::{ToolCallItem, ToolKindView, ToolStatusView};

    fn tool(id: &str) -> ChatItem {
        ChatItem::ToolCall(ToolCallItem {
            id: id.to_owned(),
            title: format!("Tool {id}"),
            kind: ToolKindView::Execute,
            tool_name: None,
            status: ToolStatusView::Completed,
            diffs: Vec::new(),
            output: Vec::new(),
            raw_input: None,
            parent_tool_id: None,
            exit: None,
        })
    }

    fn child(id: &str, parent: &str) -> ChatItem {
        let mut it = tool(id);
        if let ChatItem::ToolCall(tc) = &mut it {
            tc.parent_tool_id = Some(parent.to_owned());
        }
        it
    }

    fn asst(s: &str) -> ChatItem {
        ChatItem::AssistantText {
            text: s.to_owned(),
            streaming: false,
            message_id: None,
        }
    }

    fn think(s: &str) -> ChatItem {
        ChatItem::Thinking {
            text: s.to_owned(),
            streaming: false,
            message_id: None,
        }
    }

    fn ids(items: &[ChatItem]) -> HashSet<&str> {
        items
            .iter()
            .filter_map(|it| match it {
                ChatItem::ToolCall(tc) => Some(tc.id.as_str()),
                _ => None,
            })
            .collect()
    }

    fn spans(items: &[ChatItem]) -> Vec<(usize, usize, usize, usize)> {
        steps(items, 0..items.len(), &ids(items))
            .into_iter()
            .map(|s| (s.span.start, s.span.tool_start, s.span.end, s.tool_count))
            .collect()
    }

    #[test]
    fn a_step_absorbs_the_prose_in_front_of_its_tool_run() {
        let items = [think("why"), asst("here goes"), tool("a"), tool("b")];
        assert_eq!(spans(&items), vec![(0, 2, 4, 2)]);
    }

    #[test]
    fn consecutive_cycles_split_at_the_prose_after_each_run() {
        let items = [
            asst("first"),
            tool("a"),
            tool("b"),
            asst("second"),
            tool("c"),
        ];
        assert_eq!(spans(&items), vec![(0, 1, 3, 2), (3, 4, 5, 1)]);
    }

    #[test]
    fn a_response_without_tools_has_no_step() {
        let items = [asst("just an answer"), asst("and a follow-up")];
        assert!(spans(&items).is_empty());
    }

    #[test]
    fn trailing_prose_belongs_to_no_step() {
        let items = [asst("looking"), tool("a"), asst("done")];
        assert_eq!(spans(&items), vec![(0, 1, 2, 1)]);
    }

    #[test]
    fn a_prose_less_single_tool_run_earns_no_header() {
        assert!(spans(&[tool("a")]).is_empty());
        assert!(spans(&[tool("a"), tool("b"), tool("c")]).is_empty());
    }

    #[test]
    fn one_prose_block_plus_one_tool_reaches_the_threshold() {
        let items = [asst("check this"), tool("a")];
        assert_eq!(spans(&items), vec![(0, 1, 2, 1)]);
    }

    #[test]
    fn a_nested_child_is_not_counted_as_the_steps_own_tool() {
        let items = [asst("delegate"), tool("parent"), child("kid", "parent")];
        assert_eq!(spans(&items), vec![(0, 1, 3, 1)]);
    }

    #[test]
    fn step_span_at_recovers_the_span_a_header_was_built_from() {
        let items = [think("why"), asst("here goes"), tool("a"), tool("b")];
        let span = step_span_at(&items, 0).expect("the Step starting at 0");
        assert_eq!((span.start, span.tool_start, span.end), (0, 2, 4));
    }

    #[test]
    fn step_span_at_stops_at_the_next_user_message() {
        let items = [asst("done"), ChatItem::UserText("next".into()), tool("a")];
        assert!(step_span_at(&items, 0).is_none());
    }

    #[test]
    fn step_span_at_is_none_past_the_last_tool() {
        let items = [asst("looking"), tool("a"), asst("done")];
        assert!(step_span_at(&items, 2).is_none());
    }
}
