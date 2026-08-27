//! Step headers and the row that reveals steps hidden by the tail window.

use daruda_acp::{ChatItem, ToolKindView};
use gpui::{AnyElement, Context, IntoElement, SharedString, div, prelude::*, px};

use super::fold_header::{FoldHeader, FoldRow, SummaryLine, rollup_glyph};
use super::tool::tool_kind_icon;
use crate::surface::strings as s;
use crate::ui::theme;
use crate::ui::{Icon, Sizable as _};
use crate::workspace::main_area::agent_chat_pane::agent_chat_helpers::Rollup;
use crate::workspace::main_area::agent_chat_pane::fold::FoldKey;
use crate::workspace::main_area::agent_chat_pane::rows::LiveSubagentUnits;
use crate::workspace::main_area::agent_chat_pane::rows::step::{StepSpan, step_span_at};
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
    let icon = tool_kind_icon(dominant_kind(items, run));
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

pub(super) fn tail_more_bar(
    this: &AgentChatView,
    run_start: usize,
    hidden_steps: usize,
    collapsed: bool,
    cx: &mut Context<AgentChatView>,
) -> AnyElement {
    let title = div()
        .text_color(this.dim(theme::agent_chat_fg_muted(cx)))
        .text_size(px(theme::agent_chat_font_size(cx)))
        .child(SharedString::from(s::agent_chat_tail_more(hidden_steps)))
        .into_any_element();
    FoldRow::section(
        SharedString::from(format!("agent-chat-tail-{run_start}")),
        FoldKey::Tail(run_start),
        !collapsed,
        FoldHeader::with_title(title),
    )
    .render(this.dim_amount, cx)
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

/// Most frequent tool kind, with ties resolved by first occurrence.
fn dominant_kind(items: &[ChatItem], run: std::ops::Range<usize>) -> ToolKindView {
    let mut tally: Vec<(ToolKindView, usize)> = Vec::new();
    for item in run.filter_map(|k| items.get(k)) {
        let ChatItem::ToolCall(tc) = item else {
            continue;
        };
        match tally.iter_mut().find(|(kind, _)| *kind == tc.kind) {
            Some((_, n)) => *n += 1,
            None => tally.push((tc.kind, 1)),
        }
    }
    // `max_by_key` keeps the *last* maximum; iterate reversed so a tie resolves
    // to the kind that appeared first.
    tally
        .iter()
        .rev()
        .max_by_key(|(_, n)| *n)
        .map(|(kind, _)| *kind)
        .unwrap_or(ToolKindView::Other)
}

#[cfg(test)]
mod tests {
    use super::*;
    use daruda_acp::{ToolCallItem, ToolStatusView};

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
        let live_units = LiveSubagentUnits::build(&items);
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
        let live_units = LiveSubagentUnits::build(&items);
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
    fn dominant_kind_reports_the_most_frequent_call() {
        let items = [
            tool(ToolKindView::Execute),
            tool(ToolKindView::Read),
            tool(ToolKindView::Read),
        ];
        assert_eq!(dominant_kind(&items, 0..3), ToolKindView::Read);
    }

    #[test]
    fn dominant_kind_breaks_a_tie_on_first_occurrence() {
        let items = [tool(ToolKindView::Search), tool(ToolKindView::Edit)];
        assert_eq!(dominant_kind(&items, 0..2), ToolKindView::Search);
    }

    #[test]
    fn dominant_kind_of_an_empty_run_is_other() {
        assert_eq!(dominant_kind(&[], 0..0), ToolKindView::Other);
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

    #[test]
    fn a_step_without_prose_has_no_title() {
        let items = [tool(ToolKindView::Read)];
        assert!(step_title(&items, 0..0, TitleSource::Thinking).is_none());
        assert!(step_title(&items, 0..0, TitleSource::Assistant).is_none());
    }
}
