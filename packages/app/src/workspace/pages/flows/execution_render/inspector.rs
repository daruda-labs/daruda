//! Attempt tabs and the fixed session continuation footer.

use super::super::execution_detail::EvidenceTab;
use super::evidence::{evidence, metadata};
use super::*;
use crate::ui::{PopupMenuItem, icons, menu_builder, tab, tab_bar};

pub(super) fn render(
    id: FlowDetailId,
    run: &RunDetail,
    history: &RunHistory,
    wide: bool,
    problem: Option<String>,
    cx: &mut Context<Workspace>,
) -> AnyElement {
    let t = theme::current(cx).clone();
    let mut header = div()
        .flex_none()
        .flex()
        .flex_col()
        .gap(px(theme::GAP_LG))
        .p(px(theme::PAD_XL));
    if !wide {
        let mut choices = div().flex().flex_wrap().gap(px(theme::GAP_SM));
        for node in nodes(history) {
            let node_id = node.clone();
            choices = choices.child(
                button(
                    gpui::SharedString::from(format!("compact-node-{node}")),
                    node.to_string(),
                )
                .ghost()
                .when(
                    run.execution
                        .selection
                        .as_ref()
                        .is_some_and(|selection| selection.node() == &node),
                    |b| b.bg(t.lane_card_active_bg),
                )
                .on_click(cx.listener(move |ws, _, _, cx| {
                    ws.select_execution_node(id, node_id.clone(), cx)
                })),
            );
        }
        header = header.child(choices);
    }
    let mut footer = div()
        .debug_selector(|| "execution-inspector-footer".into())
        .flex()
        .flex_col()
        .flex_none()
        .gap(px(theme::GAP_LG))
        .p(px(theme::PAD_XL))
        .border_t_1()
        .border_color(t.border);
    if let Some(selection) = &run.execution.selection {
        header = header.child(div().child(selection.node().to_string()));
        if selection.attempt().is_none() {
            header = header.child(presentation::node_badge(history, selection.node(), cx));
        }
    }
    if let Some((node, evidence)) = run.execution.selection.as_ref().and_then(|s| s.attempt()) {
        if let Some(binding) = history.binding(node, *evidence) {
            header = header.child(
                div()
                    .text_color(t.text_muted)
                    .text_size(px(theme::FONT_SIZE_SM))
                    .child(binding.agent.clone()),
            );
        }
        let attempts = history.attempts(node);
        let selected = *evidence;
        let choices: Vec<_> = attempts
            .iter()
            .map(|(attempt, seq)| {
                (
                    *seq,
                    presentation::attempt_label(history, node, *attempt, *seq),
                )
            })
            .collect();
        let label = attempts
            .iter()
            .find(|(_, seq)| *seq == selected)
            .map(|(attempt, seq)| presentation::attempt_label(history, node, *attempt, *seq))
            .unwrap_or_else(s::flow::run_not_recorded);
        let workspace = cx.weak_entity();
        header = header.child(
            button("execution-attempt-picker", label)
                .w_full()
                .justify_between()
                .child(icons::icon(icons::EXPAND_MORE))
                .dropdown_menu(menu_builder(move |menu, _, _| {
                    choices.iter().fold(menu, |menu, (seq, label)| {
                        let workspace = workspace.clone();
                        let seq = *seq;
                        menu.item(
                            PopupMenuItem::new(label.clone())
                                .checked(seq == selected)
                                .on_click(move |_, _, cx| {
                                    if let Some(ws) = workspace.upgrade() {
                                        ws.update(cx, |ws, cx| {
                                            ws.select_execution_attempt(id, seq, cx)
                                        });
                                    }
                                }),
                        )
                    })
                })),
        );
        if let Some(attempt) = history.attempt(node, *evidence) {
            header = header.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(theme::GAP_LG))
                    .child(presentation::attempt_badge(attempt, cx))
                    .child(
                        div()
                            .text_color(t.text_muted)
                            .child(presentation::duration(Some(attempt.took))),
                    ),
            );
        }
        if !history.is_command(node) {
            footer = footer
                .child(
                    div()
                        .text_color(t.text_muted)
                        .text_size(px(theme::FONT_SIZE_SM))
                        .child(problem.clone().unwrap_or_else(s::flow::run_session_ready)),
                )
                .child(
                    button("flow-session-continue", s::flow::session_continue())
                        .primary()
                        .w_full()
                        .disabled(problem.is_some())
                        .on_click(cx.listener(move |ws, _, window, cx| {
                            ws.continue_flow_session(id, window, cx)
                        })),
                )
                .child(
                    div()
                        .text_color(t.text_muted)
                        .text_size(px(theme::FONT_SIZE_SM))
                        .child(s::flow::run_continue_hint()),
                );
        }
    }
    let selected = match run.execution.tab {
        EvidenceTab::Conversation => 0,
        EvidenceTab::Output => 1,
        EvidenceTab::Metadata => 2,
    };
    let tabs = tab_bar("execution-evidence-tabs")
        .w_full()
        .selected_index(selected)
        .child(tab(s::flow::run_conversation()))
        .child(tab(s::flow::run_output_tab()))
        .child(tab(s::flow::run_metadata()))
        .on_click(
            cx.listener(move |ws, index, _, cx| ws.select_execution_tab_index(id, *index, cx)),
        );
    let content = if run.execution.selection.is_none() {
        div()
            .child(if nodes(history).is_empty() {
                s::flow::session_legacy()
            } else {
                s::flow::run_select_node()
            })
            .child(super::super::detail_render::run_body(id, run, cx))
            .into_any_element()
    } else if run.execution.tab == EvidenceTab::Metadata {
        metadata(run, history, cx)
    } else if run
        .execution
        .selection
        .as_ref()
        .is_some_and(|s| s.attempt().is_none())
    {
        div().child(s::flow::run_no_attempt()).into_any_element()
    } else {
        evidence(id, run, cx)
    };
    div()
        .flex()
        .flex_col()
        .min_h_0()
        .min_w_0()
        .border_l_1()
        .border_color(t.border)
        .when(wide, |d| d.w(px(palette::FLOW_RUN_INSPECTOR_W)).flex_none())
        .when(!wide, |d| d.flex_1())
        .child(header)
        .child(div().flex_none().child(tabs))
        .child(
            div()
                .id(("execution-evidence-scroll", id.0 as usize))
                .debug_selector(|| "execution-evidence-scroll".into())
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .p(px(theme::PAD_XL))
                .child(content),
        )
        .child(footer)
        .into_any_element()
}

#[cfg(test)]
mod tests {}
