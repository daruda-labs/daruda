//! A stable compaction boundary with a locally disclosed retained summary.

use super::card_frame::{CardFrame, CardTone};
use super::fold_header::{FoldHeader, FoldRow};
use super::tool::output::{OutputBlockContext, output_block_view};
use super::*;
use daruda_acp::{CompactionItem, CompactionStatusView};

fn status_label(status: &CompactionStatusView) -> String {
    match status {
        CompactionStatusView::InProgress => s::agent_chat::compaction_running(),
        CompactionStatusView::Completed => s::agent_chat::compaction_completed(),
        CompactionStatusView::Failed => s::agent_chat::compaction_failed(),
        CompactionStatusView::Cancelled => s::agent_chat::compaction_cancelled(),
        CompactionStatusView::Other(_) => s::agent_chat::compaction_other(),
    }
}

pub(super) fn source_label(
    item: &CompactionItem,
    items: &[ChatItem],
    root_session: Option<&str>,
) -> Option<String> {
    if root_session == Some(item.session_id.as_str()) {
        return None;
    }
    let child = items.iter().find_map(|candidate| match candidate {
        ChatItem::ToolCall(call)
            if call.id.strip_prefix("subagent:") == Some(item.session_id.as_str()) =>
        {
            Some(call)
        }
        _ => None,
    });
    child
        .map(|call| s::agent_chat::compaction_child(&call.title))
        .or_else(|| root_session.map(|_| s::agent_chat::compaction_child_unknown()))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn compaction_block(
    ix: usize,
    item: &CompactionItem,
    key: FoldKey,
    expanded: bool,
    source: Option<String>,
    assets: RenderAssets<'_>,
    t: &theme::DarudaTheme,
    dim: f32,
    cx: &mut Context<AgentChatView>,
) -> AnyElement {
    let tone = if item.status == CompactionStatusView::Failed {
        CardTone::Error
    } else {
        CardTone::Standard
    };
    let frame = CardFrame::new(tone);
    let color = frame.status_color(dim, cx);
    let muted = theme::dim_toward_gray(theme::agent_chat_fg_muted(cx), dim);
    let mut details = s::agent_chat::compaction_source(&item.session_id);
    if let CompactionStatusView::Other(value) = &item.status {
        details.push_str(&format!("\n{value}"));
    }
    let leading = div()
        .id(("agent-chat-compaction-status", ix))
        .flex()
        .items_center()
        .gap(px(theme::GAP_SM))
        .text_size(px(theme::agent_chat_font_size(cx)))
        .text_color(color)
        .tooltip(daruda_ui::tooltip::text(details))
        .when(item.status == CompactionStatusView::InProgress, |row| {
            row.child(daruda_ui::spinner().color(color))
        })
        .child(SharedString::from(status_label(&item.status)));
    let has_summary = !item.summary.is_empty();
    let content = if has_summary {
        let header = match source.as_ref() {
            Some(source) => FoldHeader::with_title(
                div()
                    .text_size(px(theme::agent_chat_font_size(cx)))
                    .text_color(muted)
                    .child(SharedString::from(source.clone()))
                    .into_any_element(),
            ),
            None => FoldHeader::bare(),
        }
        .leading(leading.into_any_element());
        let output_key = item.output_key();
        let context = OutputBlockContext {
            assets,
            t,
            dim,
            links: AgentChatMarkdownLinks::new(cx.weak_entity()),
        };
        FoldRow::card(("agent-chat-compaction", ix), key, expanded, header, |cx| {
            let body = div()
                .id(("agent-chat-compaction-summary", ix))
                .debug_selector(move || format!("agent-chat-compaction-summary-{ix}"))
                .w_full()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(theme::GAP_SM))
                .text_size(px(theme::agent_chat_font_size(cx)))
                .text_color(muted)
                .children(item.summary.iter().enumerate().map(|(bi, block)| {
                    output_block_view(&output_key, bi, block, context.clone(), cx)
                }));
            body.into_any_element()
        })
        .render(dim, cx)
    } else {
        div()
            .id(("agent-chat-compaction-passive", ix))
            .debug_selector(|| format!("agent-chat-compaction-passive-{ix}"))
            .w_full()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(theme::GAP_STANDARD))
            .text_size(px(theme::agent_chat_font_size(cx)))
            .text_color(muted)
            .child(leading)
            .children(source.map(SharedString::from))
            .when(item.status == CompactionStatusView::Completed, |row| {
                row.child(SharedString::from(s::agent_chat::compaction_no_summary()))
            })
            .into_any_element()
    };
    frame
        .render(dim, cx)
        .flex()
        .flex_col()
        .gap(px(theme::GAP_SM))
        .child(content)
        .when_some(
            item.error.as_ref().filter(|error| !error.trim().is_empty()),
            |body, error| {
                body.child(
                    div()
                        .id(("agent-chat-compaction-error", ix))
                        .debug_selector(|| format!("agent-chat-compaction-error-{ix}"))
                        .w_full()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(px(theme::agent_chat_font_size(cx)))
                        .text_color(color)
                        .tooltip(daruda_ui::tooltip::text(error.clone()))
                        .child(SharedString::from(
                            error.split_whitespace().collect::<Vec<_>>().join(" "),
                        )),
                )
            },
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_identity_is_hidden_and_child_identity_uses_the_agent_name() {
        let mut items = Vec::new();
        let update = serde_json::from_value(serde_json::json!({
            "sessionUpdate": "tool_call", "toolCallId": "subagent:child",
            "title": "Reviewer", "kind": "think", "status": "in_progress"
        }))
        .unwrap();
        daruda_acp::apply_update(&mut items, &update);
        let child = CompactionItem::new("child", "c");
        assert_eq!(
            source_label(&child, &items, Some("root")),
            Some(s::agent_chat::compaction_child("Reviewer"))
        );
        assert_eq!(
            source_label(&CompactionItem::new("root", "c"), &items, Some("root")),
            None
        );
        assert_eq!(
            source_label(&CompactionItem::new("unknown", "c"), &items, Some("root")),
            Some(s::agent_chat::compaction_child_unknown())
        );
        assert_eq!(source_label(&child, &[], None), None);
    }

    #[test]
    fn unknown_status_is_presented_without_becoming_a_known_lifecycle() {
        let status = CompactionStatusView::Other("_provider/paused".into());
        assert_eq!(status_label(&status), s::agent_chat::compaction_other());
    }
}
