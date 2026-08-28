//! Step headers and the row that reveals steps hidden by the tail window.

use daruda_acp::ChatItem;
use gpui::{AnyElement, Context, IntoElement, SharedString, div, prelude::*, px};

use super::fold_header::{FoldHeader, FoldRow, SummaryLine, rollup_glyph, window_boundary_row};
use super::tool::tool_category_icon;
use crate::surface::strings as s;
use crate::ui::theme;
use crate::ui::{Icon, Sizable as _};
use crate::workspace::main_area::agent_chat_pane::agent_chat_helpers::Rollup;
use crate::workspace::main_area::agent_chat_pane::fold::FoldKey;
use crate::workspace::main_area::agent_chat_pane::rows::LiveSubagentUnits;
use crate::workspace::main_area::agent_chat_pane::rows::step::{StepSpan, step_span_at};
use crate::workspace::main_area::agent_chat_pane::tool_category::{ToolCategory, classify_tool};
use crate::workspace::main_area::agent_chat_pane::view::AgentChatView;

pub(super) fn step_bar(
    this: &AgentChatView,
    first_ix: usize,
    tool_count: usize,
    collapsed: bool,
    t: &theme::DarudaTheme,
    cx: &mut Context<AgentChatView>,
) -> AnyElement {
    let items = &this.items;
    // A stale key renders without a title or verdict instead of disappearing.
    let span = step_span_at(items, first_ix);
    let (title_run, run) = match &span {
        Some(span) => (span.start..span.tool_start, span.tool_start..span.end),
        None => (first_ix..first_ix, first_ix..first_ix),
    };

    let fg = this.dim(theme::agent_chat_fg(cx));
    let icon = tool_category_icon(dominant_category(items, run));
    // Prefer reasoning, then fall back to assistant prose.
    let title_items = items;
    let mut header = FoldHeader::with_summary(move || {
        step_title(title_items, title_run.clone(), TitleSource::Thinking)
            .or_else(|| step_title(title_items, title_run, TitleSource::Assistant))
    })
    .leading(
        div()
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .child(Icon::new(icon).xsmall().text_color(fg))
            .into_any_element(),
    );
    if tool_count > 0 {
        header = header.trailing(
            div()
                .flex_none()
                .text_color(this.dim(theme::agent_chat_fg_subtle(cx)))
                .text_size(px(theme::agent_chat_font_size(cx)))
                .child(SharedString::from(s::agent_chat_step_tool_count(
                    tool_count,
                )))
                .into_any_element(),
        );
    }
    if let Some(rollup) = step_rollup(items, span.as_ref(), &this.live_units) {
        header = header.trailing(rollup_glyph(rollup, t, cx));
    }
    FoldRow::section(
        SharedString::from(format!("agent-chat-step-{first_ix}")),
        FoldKey::Step(first_ix),
        !collapsed,
        header,
    )
    .render(this.dim_amount, cx)
}

fn step_rollup(
    items: &[ChatItem],
    span: Option<&StepSpan>,
    live_units: &LiveSubagentUnits,
) -> Option<Rollup> {
    span.map(|s| Rollup::of_run_with_live_units(items, s.tool_start..s.end, live_units))
}

/// The boundary row's copy. Each state names an action, and the open one names
/// the window it returns to rather than the steps it would re-hide: `Hide N
/// earlier steps` is readable as a *description* of the current state precisely
/// when those steps are on screen, which is the misread that made the row's two
/// states indistinguishable.
fn tail_more_label(hidden_steps: usize, kept_steps: usize, collapsed: bool) -> String {
    if collapsed {
        s::agent_chat_tail_more_show(hidden_steps)
    } else {
        s::agent_chat_tail_more_collapse(kept_steps)
    }
}

pub(super) fn tail_more_bar(
    this: &AgentChatView,
    run_start: usize,
    hidden_steps: usize,
    kept_steps: usize,
    collapsed: bool,
    cx: &mut Context<AgentChatView>,
) -> AnyElement {
    window_boundary_row(
        SharedString::from(format!("agent-chat-tail-{run_start}")),
        FoldKey::Tail(run_start),
        !collapsed,
        SharedString::from(tail_more_label(hidden_steps, kept_steps, collapsed)),
        this.dim_amount,
        cx,
    )
}

#[derive(Clone, Copy)]
enum TitleSource {
    Thinking,
    Assistant,
}

fn step_title(
    items: &[ChatItem],
    prose: std::ops::Range<usize>,
    source: TitleSource,
) -> Option<SummaryLine> {
    prose
        .filter_map(|k| match (items.get(k), source) {
            (Some(ChatItem::Thinking { text, .. }), TitleSource::Thinking) => {
                SummaryLine::from_markdown(text).map(SummaryLine::reasoning)
            }
            (Some(ChatItem::AssistantText { text, .. }), TitleSource::Assistant) => {
                SummaryLine::from_markdown(text)
            }
            _ => None,
        })
        .next()
}

/// Most frequent shared tool category, with ties resolved by first occurrence.
fn dominant_category(items: &[ChatItem], run: std::ops::Range<usize>) -> ToolCategory {
    let mut tally: Vec<(ToolCategory, usize)> = Vec::new();
    for item in run.filter_map(|k| items.get(k)) {
        let ChatItem::ToolCall(tc) = item else {
            continue;
        };
        let category = classify_tool(tc);
        match tally
            .iter_mut()
            .find(|(candidate, _)| *candidate == category)
        {
            Some((_, n)) => *n += 1,
            None => tally.push((category, 1)),
        }
    }
    // `max_by_key` keeps the *last* maximum; iterate reversed so a tie resolves
    // to the kind that appeared first.
    tally
        .iter()
        .rev()
        .max_by_key(|(_, n)| *n)
        .map(|(category, _)| *category)
        .unwrap_or(ToolCategory::Other)
}

#[cfg(test)]
mod tests {
    use super::*;
    use daruda_acp::{ToolCallItem, ToolKindView, ToolStatusView};

    fn tool(kind: ToolKindView) -> ChatItem {
        ChatItem::ToolCall(ToolCallItem {
            id: format!("{kind:?}"),
            title: "t".into(),
            kind,
            tool_name: None,
            status: ToolStatusView::Completed,
            diffs: Vec::new(),
            output: Vec::new(),
            raw_input: None,
            parent_tool_id: None,
            exit: None,
        })
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

    #[test]
    fn a_step_with_no_span_shows_no_verdict() {
        let items = [asst("looking"), tool(ToolKindView::Read)];
        let live_units = LiveSubagentUnits::of(&items);
        assert_eq!(step_rollup(&items, None, &live_units), None);

        let span = step_span_at(&items, 0).expect("the step starting at 0");
        assert_eq!(
            step_rollup(&items, Some(&span), &live_units),
            Some(Rollup::Ok)
        );
    }

    #[test]
    fn a_step_rollup_stays_running_while_a_later_subagent_child_is_live() {
        let parent = tool(ToolKindView::Think);
        let parent_id = match &parent {
            ChatItem::ToolCall(tc) => tc.id.clone(),
            _ => unreachable!(),
        };
        let mut child = tool(ToolKindView::Read);
        if let ChatItem::ToolCall(tc) = &mut child {
            tc.id = "child".into();
            tc.status = ToolStatusView::InProgress;
            tc.parent_tool_id = Some(parent_id);
        }
        let items = [asst("working"), parent, asst("reported"), child];
        let live_units = LiveSubagentUnits::of(&items);
        let span = StepSpan {
            start: 0,
            tool_start: 1,
            end: 2,
        };

        assert_eq!(
            step_rollup(&items, Some(&span), &live_units),
            Some(Rollup::Running)
        );
    }

    #[test]
    fn dominant_category_reports_the_most_frequent_call() {
        let items = [
            tool(ToolKindView::Execute),
            tool(ToolKindView::Read),
            tool(ToolKindView::Read),
        ];
        assert_eq!(dominant_category(&items, 0..3), ToolCategory::Read);
    }

    #[test]
    fn dominant_category_breaks_a_tie_on_first_occurrence() {
        let items = [tool(ToolKindView::Search), tool(ToolKindView::Edit)];
        assert_eq!(dominant_category(&items, 0..2), ToolCategory::Search);
    }

    #[test]
    fn dominant_category_of_an_empty_run_is_other() {
        assert_eq!(dominant_category(&[], 0..0), ToolCategory::Other);
    }

    #[test]
    fn dominant_category_uses_the_same_name_override_as_the_filter() {
        let mut read = tool(ToolKindView::Execute);
        if let ChatItem::ToolCall(tc) = &mut read {
            tc.tool_name = Some("Read".into());
        }
        assert_eq!(dominant_category(&[read], 0..1), ToolCategory::Read);
    }

    #[test]
    fn the_title_prefers_reasoning_over_the_reply() {
        let items = [think("**Preparing the review**"), asst("Let me check.")];
        let title = step_title(&items, 0..2, TitleSource::Thinking)
            .or_else(|| step_title(&items, 0..2, TitleSource::Assistant))
            .expect("a thinking block yields a title");
        assert_eq!(title.text(), "Preparing the review");
    }

    #[test]
    fn the_title_falls_back_to_the_reply_when_there_is_no_reasoning() {
        let items = [asst("실제로 실행해서 검증하겠습니다.")];
        let title = step_title(&items, 0..1, TitleSource::Thinking)
            .or_else(|| step_title(&items, 0..1, TitleSource::Assistant))
            .expect("an assistant block yields a title");
        assert_eq!(title.text(), "실제로 실행해서 검증하겠습니다.");
    }

    #[test]
    fn an_empty_leading_block_falls_through_to_the_next() {
        let items = [asst("   "), asst("Running the build.")];
        let title =
            step_title(&items, 0..2, TitleSource::Assistant).expect("the next block yields one");
        assert_eq!(title.text(), "Running the build.");
    }

    /// Each state names an action, and they name *different* counts: closed
    /// promises the hidden steps, open promises the window it returns to. A
    /// label built from the hidden count in both states is what read as a
    /// description of the current state once the steps were on screen.
    #[test]
    fn the_boundary_label_names_the_hidden_count_closed_and_the_kept_count_open() {
        let closed = tail_more_label(6, 5, true);
        let open = tail_more_label(6, 5, false);
        assert_ne!(
            closed, open,
            "an open boundary must not repeat the closed promise"
        );
        assert!(
            closed.contains('6'),
            "closed names the hidden steps: {closed}"
        );
        assert!(
            open.contains('5') && !open.contains('6'),
            "open names the kept steps, not the hidden ones: {open}"
        );
        // Singular is handled on both sides, like the filtered-row placeholder.
        assert_ne!(tail_more_label(1, 5, true), tail_more_label(2, 5, true));
        assert_ne!(tail_more_label(6, 1, false), tail_more_label(6, 2, false));
    }

    #[test]
    fn a_step_without_prose_has_no_title() {
        let items = [tool(ToolKindView::Read)];
        assert!(step_title(&items, 0..0, TitleSource::Thinking).is_none());
        assert!(step_title(&items, 0..0, TitleSource::Assistant).is_none());
    }
}
