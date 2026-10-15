//! A stable compaction boundary with a locally disclosed retained summary.

use super::fold_header::{FoldHeader, FoldRow};
use super::tool::output::{OutputBlockContext, output_block_view};
use super::*;
use daruda_acp::{CompactionItem, CompactionStatusView};
use gpui::{App, Hsla};

fn status_label(status: &CompactionStatusView) -> String {
    match status {
        CompactionStatusView::InProgress => s::agent_chat::compaction_running(),
        CompactionStatusView::Completed => s::agent_chat::compaction_completed(),
        CompactionStatusView::Failed => s::agent_chat::compaction_failed(),
        CompactionStatusView::Cancelled => s::agent_chat::compaction_cancelled(),
        CompactionStatusView::Other(_) => s::agent_chat::compaction_other(),
    }
}

fn status_color(status: &CompactionStatusView, cx: &App) -> Hsla {
    if *status != CompactionStatusView::Failed {
        return theme::agent_chat_fg_muted(cx);
    }
    // UI and terminal presets are independent. Resolve the error hue against
    // the pane itself, and let the caller apply inactive-pane dimming once.
    let mut color = theme::current(cx).banner_error_text;
    color.l = if theme::agent_chat_syntax_is_light(cx) {
        color.l.min(theme::PANE_ERROR_MAX_L_ON_LIGHT)
    } else {
        color.l.max(theme::PANE_ERROR_MIN_L_ON_DARK)
    };
    color
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
    full_summary: bool,
    source: Option<String>,
    assets: RenderAssets<'_>,
    t: &theme::DarudaTheme,
    dim: f32,
    cx: &mut Context<AgentChatView>,
) -> AnyElement {
    let color = theme::dim_toward_gray(status_color(&item.status, cx), dim);
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
    let content =
        if has_summary {
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
            FoldRow::block(("agent-chat-compaction", ix), key, expanded, header, |cx| {
                let body = div()
                    .id(("agent-chat-compaction-summary", ix))
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
                let line_height = theme::agent_chat_font_size(cx) * theme::MD_VIEW_LINE_HEIGHT;
                let summary = if full_summary {
                    daruda_ui::scroll_area(
                        SharedString::from(format!("agent-chat-compaction-summary-{ix}")),
                        px(line_height * theme::AGENT_CHAT_EMBED_MAX_ROWS as f32),
                        body,
                    )
                    .w_full()
                    .min_w_0()
                    .into_any_element()
                } else {
                    div()
                        .id(("agent-chat-compaction-preview", ix))
                        .debug_selector(|| format!("agent-chat-compaction-preview-{ix}"))
                        .w_full()
                        .min_w_0()
                        .max_h(px(
                            line_height * theme::AGENT_CHAT_COMPACTION_PREVIEW_ROWS as f32
                        ))
                        .overflow_hidden()
                        .child(body)
                        .into_any_element()
                };
                div()
                    .w_full()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(theme::GAP_SM))
                    .child(summary)
                    .child(
                        div().flex().child(
                            daruda_ui::button_on_surface(
                                SharedString::from(format!("agent-chat-compaction-full-{ix}")),
                                if full_summary {
                                    s::agent_chat::compaction_show_preview()
                                } else {
                                    s::agent_chat::compaction_show_full()
                                },
                                &theme::PaneSurfaceTokens::agent_chat(cx).dimmed(dim),
                                cx,
                            )
                            .on_click(cx.listener(
                                move |view, _, _, cx| view.toggle_compaction_summary(ix, cx),
                            )),
                        ),
                    )
                    .into_any_element()
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
    div()
        .w_full()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(theme::GAP_SM))
        .px(px(theme::AGENT_CHAT_INPUT_INNER_PAD_X))
        .py(px(theme::AGENT_CHAT_INPUT_INNER_PAD_Y))
        .rounded(px(theme::AGENT_CHAT_INPUT_RADIUS))
        .bg(theme::dim_toward_gray(theme::agent_chat_tint(cx), dim))
        .border_1()
        .border_color(if item.status == CompactionStatusView::Failed {
            color
        } else {
            theme::dim_toward_gray(theme::agent_chat_border_tint(cx), dim)
        })
        .when(item.status == CompactionStatusView::Failed, |card| {
            card.border_dashed()
        })
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

    #[gpui::test]
    fn failed_status_is_readable_across_ui_and_terminal_presets(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            theme::init_if_missing(cx);
            for ui in ["daruda_light", "daruda_dark"] {
                assert!(theme::apply_ui_theme(ui, cx));
                for preset in daruda_config::theme_presets::PRESETS {
                    let Some(colors) = daruda_config::theme_presets::colors_for_preset(preset.name)
                    else {
                        continue;
                    };
                    let bg = colors.background;
                    theme::set_agent_chat_bg(cx, bg.r, bg.g, bg.b);
                    let color = status_color(&CompactionStatusView::Failed, cx);
                    let background = theme::agent_chat_bg(cx).blend(theme::agent_chat_tint(cx));
                    let ratio = theme::contrast_ratio(color, background);
                    assert!(ratio >= 4.5, "{ui} / {}: {ratio:.2}:1", preset.name);
                }
                for (r, g, b) in [(250, 250, 250), (224, 228, 240)] {
                    theme::set_agent_chat_bg(cx, r, g, b);
                    let color = status_color(&CompactionStatusView::Failed, cx);
                    let background = theme::agent_chat_bg(cx).blend(theme::agent_chat_tint(cx));
                    let ratio = theme::contrast_ratio(color, background);
                    assert!(ratio >= 4.5, "{ui} / light pane: {ratio:.2}:1");
                }
            }
        });
    }
}
