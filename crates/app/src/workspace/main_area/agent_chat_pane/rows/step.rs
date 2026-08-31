//! Partitions a response into tool runs plus their preceding prose. Trailing
//! prose remains outside every step as the response conclusion.

use std::ops::Range;

use daruda_acp::ChatItem;

use super::super::tool_hierarchy::ToolHierarchy;
use super::{is_bodyless, is_tool_call};

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
    /// Whether this step gets a header; tail counting still includes it.
    pub(super) renders_header: bool,
}

impl WorkStep {
    /// Top-level tools in this step that `keep` admits. Nested subagent tools
    /// render inside their parent's card, so they never count.
    ///
    /// The header is a disclosure, so this is what expanding it puts on screen
    /// — under a display filter that is fewer calls than the step made, and
    /// the count has to say so.
    pub(super) fn kept_tool_count(
        &self,
        items: &[ChatItem],
        hierarchy: &ToolHierarchy<'_>,
        keep: impl Fn(&ChatItem) -> bool,
    ) -> usize {
        (self.span.tool_start..self.span.end)
            .filter(|&j| top_level_tool(items, j, hierarchy) && keep(&items[j]))
            .count()
    }
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
        let renders_header = folded_rows(items, &span, hierarchy) >= STEP_MIN_ROWS;
        out.push(WorkStep {
            span,
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

/// Which of a step's own prose items its header shows, per fold state.
///
/// Field names match [`super::super::render::fold_header`]'s slot vocabulary:
/// the header shows `collapsed` while folded and `expanded` once open.
pub(in crate::workspace) struct StepHeaderText {
    /// The preamble the agent wrote before this step's work. Shown collapsed,
    /// because it is what the fold hides and what the reader came for.
    pub(in crate::workspace) collapsed: usize,
    /// The thought summary, shown once the preamble is on screen — and whose
    /// row the header then owns. `Some` only when the header can show it
    /// whole; a longer thought keeps its own row, where the rest is reachable.
    pub(in crate::workspace) expanded: Option<usize>,
}

/// A step's header text, or `None` when the step has no labelled preamble and
/// the header keeps its plain collapsed-only summary.
///
/// Reading the preamble label rather than the agent id is what keeps this
/// agent-neutral: only an adapter that labels its messages produces one, so the
/// renderer never asks who is speaking.
///
/// Both halves skip a [`is_bodyless`] item. One renders nothing, so letting it
/// take a slot would have an invisible item change the layout of the visible
/// ones — the header would enter its two-state mode and take the thought's row
/// while showing the same line in both states.
pub(in crate::workspace) fn step_header_text(
    items: &[ChatItem],
    span: &StepSpan,
) -> Option<StepHeaderText> {
    let mut prose = (span.start..span.tool_start).filter(|&k| !is_bodyless(&items[k]));
    let collapsed = prose.clone().find(|&k| {
        matches!(&items[k], ChatItem::AssistantText { phase, .. }
            if *phase == daruda_acp::MessagePhase::Commentary)
    })?;
    let expanded = prose
        .find(|&k| matches!(&items[k], ChatItem::Thinking { text, .. } if fits_a_header(text)));
    Some(StepHeaderText {
        collapsed,
        expanded,
    })
}

/// Whether this body is a single line, so a header showing its first line shows
/// all of it. Guards multi-line bodies only — a long single line still fits by
/// this test and is ellipsized on screen, which costs the reader nothing, since
/// expanding would show them that same one line.
fn fits_a_header(text: &str) -> bool {
    text.lines().filter(|l| !l.trim().is_empty()).count() <= 1
}

/// Count the projected rows a step header would fold.
fn folded_rows(items: &[ChatItem], span: &StepSpan, hierarchy: &ToolHierarchy<'_>) -> usize {
    let prose = (span.start..span.tool_start)
        .filter(|&k| !is_bodyless(&items[k]))
        .count();
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
mod header_text_tests {
    use super::*;
    use daruda_acp::MessagePhase;

    fn asst(s: &str, phase: MessagePhase) -> ChatItem {
        ChatItem::AssistantText {
            text: s.to_owned(),
            streaming: false,
            message_id: None,
            phase,
        }
    }

    fn think(s: &str) -> ChatItem {
        ChatItem::Thinking {
            text: s.to_owned(),
            streaming: false,
            message_id: None,
        }
    }

    fn tool() -> ChatItem {
        ChatItem::ToolCall(daruda_acp::ToolCallItem {
            id: "t".into(),
            title: "Tool".into(),
            kind: daruda_acp::ToolKindView::Execute,
            tool_name: None,
            status: daruda_acp::ToolStatusView::Completed,
            diffs: Vec::new(),
            output: Vec::new(),
            raw_input: None,
            parent_tool_id: None,
            exit: None,
        })
    }

    fn span_of(items: &[ChatItem]) -> StepSpan {
        step_span_at(items, 0).expect("a step")
    }

    /// The captured codex shape: a one-line thought summary, then the preamble
    /// it wrote for the reader, then the tools.
    #[test]
    fn the_preamble_is_shown_collapsed_and_the_thought_once_expanded() {
        let items = [
            think("**Inspecting Workspace struct and operations**"),
            asst("`Workspace` structure walk", MessagePhase::Commentary),
            tool(),
        ];
        let text = step_header_text(&items, &span_of(&items)).expect("both halves present");
        assert_eq!(text.collapsed, 1, "the preamble is what the fold hides");
        assert_eq!(text.expanded, Some(0), "the header takes over the thought");
    }

    /// An item that renders nothing must not change the layout of items that do.
    /// An empty preamble would otherwise flip the header into its two-state mode
    /// and take the thought's row, leaving the header showing the same line in
    /// both states for no gain.
    #[test]
    fn a_preamble_with_no_text_is_not_a_preamble() {
        let items = [
            think("**Inspecting Workspace struct and operations**"),
            asst("", MessagePhase::Commentary),
            tool(),
        ];
        assert!(step_header_text(&items, &span_of(&items)).is_none());
    }

    /// A thought with no text cannot be shown whole by a header that shows
    /// nothing, so it must not be taken over either.
    #[test]
    fn a_thought_with_no_text_is_not_taken_over() {
        let items = [
            think(""),
            asst("walking the structure", MessagePhase::Commentary),
            tool(),
        ];
        let text = step_header_text(&items, &span_of(&items)).expect("a real preamble");
        assert_eq!(text.collapsed, 1);
        assert_eq!(text.expanded, None, "nothing to take over");
    }

    /// Without a labelled preamble there is nothing to alternate with, so the
    /// header keeps its existing collapsed-only behavior.
    #[test]
    fn an_unlabelled_reply_is_not_a_preamble() {
        let items = [
            think("why"),
            asst("here goes", MessagePhase::Answer),
            tool(),
        ];
        assert!(step_header_text(&items, &span_of(&items)).is_none());
    }

    /// A thought the header can only show the first line of stays in the body,
    /// where the rest of it is reachable.
    #[test]
    fn a_thought_too_long_for_the_header_is_left_in_the_body() {
        let items = [
            think("**Inspecting**\n\nand a second paragraph the header cannot show"),
            asst("walking the structure", MessagePhase::Commentary),
            tool(),
        ];
        let text = step_header_text(&items, &span_of(&items)).expect("a preamble is present");
        assert_eq!(text.collapsed, 1);
        assert_eq!(text.expanded, None, "nothing the header can show whole");
    }

    #[test]
    fn a_preamble_with_no_thought_still_alternates_from_an_empty_expanded_header() {
        let items = [
            asst("walking the structure", MessagePhase::Commentary),
            tool(),
        ];
        let text = step_header_text(&items, &span_of(&items)).expect("a preamble is present");
        assert_eq!(text.collapsed, 0);
        assert_eq!(text.expanded, None);
    }

    /// Only the step's own prose counts — a preamble belonging to a later step
    /// must not be pulled into this one's header.
    #[test]
    fn only_the_steps_own_prose_is_considered() {
        let items = [
            think("first"),
            asst("first preamble", MessagePhase::Commentary),
            tool(),
            asst("second preamble", MessagePhase::Commentary),
            tool(),
        ];
        let text = step_header_text(&items, &span_of(&items)).expect("a preamble is present");
        assert_eq!(text.collapsed, 1, "not the preamble of the next step");
    }
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
            phase: Default::default(),
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
        let hierarchy = ToolHierarchy::build(items);
        steps(items, 0..items.len(), &hierarchy)
            .into_iter()
            .map(|s| {
                (
                    s.span.start,
                    s.span.tool_start,
                    s.span.end,
                    s.kept_tool_count(items, &hierarchy, |_| true),
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
