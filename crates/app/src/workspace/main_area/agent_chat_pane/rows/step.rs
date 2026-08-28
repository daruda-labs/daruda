//! Partitions a response into tool runs plus their preceding prose. Trailing
//! prose remains outside every step as the response conclusion.

use std::ops::Range;

use daruda_acp::ChatItem;

use super::super::tool_hierarchy::ToolHierarchy;
use super::is_tool_call;

/// Minimum projected row count that earns a step header.
const STEP_MIN_ROWS: usize = 2;

/// Half-open item span for one step.
pub(in crate::workspace) struct StepSpan {
    pub(in crate::workspace) start: usize,
    pub(in crate::workspace) tool_start: usize,
    pub(in crate::workspace) end: usize,
}

pub(super) struct WorkStep {
    pub(super) span: StepSpan,
    /// Nested subagent tools do not count toward the header.
    pub(super) tool_count: usize,
    /// Whether this step gets a header; tail counting still includes it.
    pub(super) renders_header: bool,
}

/// Partition a response into disjoint work steps.
pub(super) fn steps(
    items: &[ChatItem],
    run: Range<usize>,
    hierarchy: &ToolHierarchy<'_>,
) -> Vec<WorkStep> {
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
        let tool_count = (span.tool_start..span.end)
            .filter(|&j| top_level_tool(items, j, hierarchy))
            .count();
        let renders_header = folded_rows(items, &span, hierarchy) >= STEP_MIN_ROWS;
        out.push(WorkStep {
            span,
            tool_count,
            renders_header,
        });
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
fn folded_rows(items: &[ChatItem], span: &StepSpan, hierarchy: &ToolHierarchy<'_>) -> usize {
    let prose = span.tool_start - span.start;
    let mut chunks = 0;
    let mut k = span.tool_start;
    while k < span.end {
        if !top_level_tool(items, k, hierarchy) {
            k += 1;
            continue;
        }
        chunks += 1;
        k += 1;
        while k < span.end && top_level_tool(items, k, hierarchy) {
            k += 1;
        }
    }
    prose + chunks
}

fn top_level_tool(items: &[ChatItem], ix: usize, hierarchy: &ToolHierarchy<'_>) -> bool {
    matches!(&items[ix], ChatItem::ToolCall(tc) if !hierarchy.is_nested_child(tc))
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

    fn spans(items: &[ChatItem]) -> Vec<(usize, usize, usize, usize, bool)> {
        steps(items, 0..items.len(), &ToolHierarchy::build(items))
            .into_iter()
            .map(|s| {
                (
                    s.span.start,
                    s.span.tool_start,
                    s.span.end,
                    s.tool_count,
                    s.renders_header,
                )
            })
            .collect()
    }

    #[test]
    fn a_step_absorbs_the_prose_in_front_of_its_tool_run() {
        let items = [think("why"), asst("here goes"), tool("a"), tool("b")];
        assert_eq!(spans(&items), vec![(0, 2, 4, 2, true)]);
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
        assert_eq!(spans(&items), vec![(0, 1, 3, 2, true), (3, 4, 5, 1, true)]);
    }

    #[test]
    fn a_response_without_tools_has_no_step() {
        let items = [asst("just an answer"), asst("and a follow-up")];
        assert!(spans(&items).is_empty());
    }

    #[test]
    fn trailing_prose_belongs_to_no_step() {
        let items = [asst("looking"), tool("a"), asst("done")];
        assert_eq!(spans(&items), vec![(0, 1, 2, 1, true)]);
    }

    #[test]
    fn a_prose_less_tool_run_counts_without_earning_a_header() {
        assert_eq!(spans(&[tool("a")]), vec![(0, 0, 1, 1, false)]);
        assert_eq!(
            spans(&[tool("a"), tool("b"), tool("c")]),
            vec![(0, 0, 3, 3, false)]
        );
    }

    #[test]
    fn one_prose_block_plus_one_tool_reaches_the_threshold() {
        let items = [asst("check this"), tool("a")];
        assert_eq!(spans(&items), vec![(0, 1, 2, 1, true)]);
    }

    #[test]
    fn a_nested_child_is_not_counted_as_the_steps_own_tool() {
        let items = [asst("delegate"), tool("parent"), child("kid", "parent")];
        assert_eq!(spans(&items), vec![(0, 1, 3, 1, true)]);
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
