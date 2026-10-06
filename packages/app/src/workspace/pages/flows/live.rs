//! Inline permission questions and the live run's Stop action.

use crate::surface::strings;
use crate::ui::theme;
use crate::workspace::flow_browser::FlowPageSnapshot;
use crate::workspace::flow_rows::FlowRunRow;
use gpui::{IntoElement, SharedString, div, prelude::*, px};

pub(super) fn ask_block(
    lane: daruda_store::project::LaneRef,
    ask: &crate::workspace::flow_rows::AskRowData,
    also_waiting: usize,
    snap: &FlowPageSnapshot,
    cx: &gpui::App,
) -> impl IntoElement {
    let t = theme::current(cx);
    let ask_id = ask.ask_id;
    // No heading naming the tool: the summary line above already reads
    // "<node> is waiting on you — <tool>", and saying it twice in a 290px
    // column pushes the buttons off the first screenful.
    div()
        .flex()
        .flex_col()
        .gap(px(theme::GAP_SM))
        .px(px(theme::LIST_ROW_PAD_X))
        .children(ask.detail.clone().map(|detail| {
            div()
                .text_size(px(theme::DOCK_PLACEHOLDER_FONT_SIZE))
                .text_color(t.text_subtle)
                .child(detail)
        }))
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap(px(theme::GAP_SM))
                .children(
                    ask.options
                        .iter()
                        .enumerate()
                        .map(|(ix, choice)| answer_button(lane, ask_id, ix, choice, snap)),
                ),
        )
        // Only when there are: a line saying "0 more waiting" under every
        // ordinary question is noise on the common case.
        .when(also_waiting > 0, |block| {
            block.child(
                div()
                    .text_size(px(theme::DOCK_PLACEHOLDER_FONT_SIZE))
                    .text_color(t.text_subtle)
                    .child(strings::flow::more_questions_waiting(also_waiting)),
            )
        })
}

/// One answer. Allow kinds take the primary treatment and reject kinds the
/// danger one — the same reading the agent-chat permission card uses, so a
/// person sees the same shape in both places.
fn answer_button(
    lane: daruda_store::project::LaneRef,
    ask_id: u64,
    ix: usize,
    choice: &daruda_acp::PermissionChoice,
    snap: &FlowPageSnapshot,
) -> impl IntoElement + use<> {
    let id = SharedString::from(format!("flow-answer-{ask_id}-{ix}"));
    let label = SharedString::from(choice.name.clone());
    let option_id = choice.option_id.clone();
    let workspace = snap.workspace.clone();
    let button = match choice.kind {
        daruda_acp::PermissionKindView::AllowOnce | daruda_acp::PermissionKindView::AllowAlways => {
            crate::ui::button_primary(id, label)
        }
        daruda_acp::PermissionKindView::RejectOnce
        | daruda_acp::PermissionKindView::RejectAlways => crate::ui::button_danger(id, label),
    };
    let allow = matches!(
        choice.kind,
        daruda_acp::PermissionKindView::AllowOnce | daruda_acp::PermissionKindView::AllowAlways
    );
    button.on_click(move |_, _window, cx| {
        let decision = if allow {
            daruda_acp::PermissionDecision::Allow {
                option_id: option_id.clone(),
            }
        } else {
            daruda_acp::PermissionDecision::Reject {
                option_id: option_id.clone(),
            }
        };
        match workspace.update(cx, |ws, cx| ws.answer_flow_ask(lane, ask_id, decision, cx)) {
            Ok(()) => {}
            Err(e) => daruda_store::observability::log_writer::LogWriter::log(
                daruda_store::observability::error_report::ErrorReport::new(
                    "Flows panel: workspace gone while answering a permission question",
                )
                .severity(daruda_store::observability::error_report::ErrorSeverity::Warning)
                .at(file!(), line!())
                .with_context("error", format!("{e}"))
                .dedup("right_dock.flow.answer")
                .build(),
            ),
        }
    })
}

pub(super) fn stop_button(run: &FlowRunRow, snap: &FlowPageSnapshot) -> impl IntoElement + use<> {
    let workspace = snap.workspace.clone();
    let lane = run.lane;
    crate::ui::button(
        SharedString::from(format!("flow-panel-stop-{}-{}", lane.project, lane.lane)),
        strings::flow::panel_stop(),
    )
    .tab_stop(true)
    .on_click(move |_, _, cx| {
        if let Some(ws) = workspace.upgrade() {
            ws.update(cx, |ws, cx| ws.stop_flow_run_in(lane, cx));
        }
    })
}
