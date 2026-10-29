//! Read-only saved evidence and session metadata.

use super::super::execution_detail::{Evidence, EvidenceTab};
use super::*;

pub(super) fn evidence(
    id: FlowDetailId,
    run: &RunDetail,
    cx: &mut Context<Workspace>,
) -> AnyElement {
    let t = theme::current(cx).clone();
    let output = run.execution.tab == EvidenceTab::Output;
    let mut body = div()
        .debug_selector(|| "execution-evidence-body".into())
        .flex()
        .flex_col()
        .gap(px(theme::PAD_XL));
    body = body.child(
        button(
            "execution-evidence-file",
            if output {
                s::flow::run_output()
            } else {
                s::flow::run_transcript_title()
            },
        )
        .ghost()
        .on_click(
            cx.listener(move |ws, _, window, cx| ws.open_selected_execution_file(id, window, cx)),
        ),
    );
    match &run.execution.evidence {
        Evidence::Ready { text, truncated } => {
            body = body
                .when(*truncated, |d| d.child(s::flow::run_evidence_truncated()))
                .child(
                    crate::ui::markdown(("execution-evidence", id.0 as usize), text.clone())
                        .text_size(px(theme::RIGHT_PANEL_BODY_FONT_SIZE))
                        .heading_size(px(theme::FONT_SIZE_LG))
                        .selectable(true)
                        .full_width(true),
                );
        }
        Evidence::Loading => body = body.child(s::common::loading()),
        Evidence::Failed(error) => body = body.child(error.clone()),
        Evidence::Empty | Evidence::Missing => body = body.child(s::flow::run_evidence_missing()),
    }
    body.when(!output, |d| {
        d.child(
            div()
                .border_t_1()
                .border_color(t.border)
                .pt(px(theme::PAD_XL))
                .text_size(px(theme::FONT_SIZE_SM))
                .text_color(t.text_muted)
                .child(s::flow::run_transcript_note()),
        )
    })
    .into_any_element()
}

pub(super) fn metadata(run: &RunDetail, history: &RunHistory, cx: &gpui::App) -> AnyElement {
    let t = theme::current(cx).clone();
    let mut rows = vec![(
        s::flow::run_id_label(),
        run.dir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
    )];
    if let Some(selection) = &run.execution.selection {
        rows.push((s::flow::run_nodes(), selection.node().to_string()));
    }
    if let Some((node, seq)) = run.execution.selection.as_ref().and_then(|s| s.attempt()) {
        if let Some((attempt, _)) = history
            .attempts(node)
            .iter()
            .find(|(_, evidence)| evidence == seq)
        {
            rows.push((
                s::flow::run_attempts(),
                s::flow::run_attempt_label(attempt, seq),
            ));
        }
        if let Some(binding) = history.binding(node, *seq) {
            rows.extend([
                (s::flow::run_agent_label(), binding.agent.clone()),
                (
                    s::flow::run_account_label(),
                    binding
                        .account
                        .as_ref()
                        .map(|a| {
                            a.managed
                                .clone()
                                .unwrap_or_else(s::flow::run_account_system)
                        })
                        .unwrap_or_else(s::flow::run_not_recorded),
                ),
                (s::flow::run_cwd_label(), binding.cwd.display().to_string()),
                (s::flow::run_session_label(), binding.session_id.clone()),
            ]);
        }
    }
    div()
        .children(rows.into_iter().map(|(label, value)| {
            div()
                .flex()
                .flex_col()
                .gap(px(theme::GAP_SM))
                .py(px(theme::PAD_LG))
                .border_b_1()
                .border_color(t.border)
                .child(
                    div()
                        .text_color(t.text_muted)
                        .text_size(px(theme::FONT_SIZE_SM))
                        .child(label),
                )
                .child(div().min_w_0().child(value))
        }))
        .child(
            div()
                .py(px(theme::PAD_XL))
                .text_color(t.text_muted)
                .child(s::flow::run_unknown_metrics()),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {}
