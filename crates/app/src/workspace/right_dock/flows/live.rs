//! The Flows panel's second section: the runs going now.
//!
//! A run's summary line and its Stop, plus — when the run parks a
//! permission request — the question and one button per answer the adapter
//! offered.

use gpui::{AnyElement, IntoElement, SharedString, div, prelude::*, px};

use crate::surface::strings;
use crate::ui::theme;
use crate::workspace::flow_rows::FlowRunRow;
use crate::workspace::layout::RightDockSnapshot;

/// No run in this lane. Deliberately not "no runs anywhere" — a run in
/// another lane is still going, and the chip is saying so.
pub(super) fn empty_state(cx: &gpui::App) -> AnyElement {
    crate::ui::placeholder_text(strings::right_panel_flow_empty())
        .py(px(theme::RIGHT_PANEL_PAD_Y))
        .text_size(px(theme::DOCK_PLACEHOLDER_FONT_SIZE))
        .text_color(theme::current(cx).text_subtle)
        .into_any_element()
}

/// One live run: what it is doing, a Stop, and — when it is waiting on a
/// person — the question and its answers.
///
/// The question sits in the panel rather than in the chip's popover
/// because a popover dismisses on an outside click, and a question with a
/// clock running must not be something you can lose by clicking away.
pub(super) fn run_row(
    run: &FlowRunRow,
    snap: &RightDockSnapshot,
    cx: &gpui::App,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(px(theme::GAP_SM))
        .child(run_summary(run, snap, cx))
        .children(
            run.asking
                .as_ref()
                .map(|ask| ask_block(run.lane, ask, run.also_waiting, snap, cx)),
        )
}

/// The question, what it is about, and one button per option the adapter
/// offered — `AllowAlways` included. The engine never *selects* that one
/// (it outlives the session), but a person looking at it choosing it is an
/// informed decision, and hiding an option the agent offered would be us
/// answering on their behalf.
fn ask_block(
    lane: daruda_store::project::LaneRef,
    ask: &crate::workspace::flow_rows::AskRowData,
    also_waiting: usize,
    snap: &RightDockSnapshot,
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
        .px(px(theme::SKILL_ROW_PAD_X))
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
                    .child(strings::flow_more_questions_waiting(also_waiting)),
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
    snap: &RightDockSnapshot,
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

fn run_summary(run: &FlowRunRow, snap: &RightDockSnapshot, cx: &gpui::App) -> impl IntoElement {
    let t = theme::current(cx);
    let workspace = snap.workspace.clone();
    let lane = run.lane;
    div()
        .flex()
        .flex_row()
        // Without this the row sizes to its content instead of the dock, so
        // `justify_between` has no slack to distribute and the button is laid
        // out past the dock's right edge — where `overflow_hidden` cuts it.
        .w_full()
        .items_center()
        .justify_between()
        .w_full()
        .min_w_0()
        .gap(px(theme::RIGHT_PANEL_ROW_GAP))
        .px(px(theme::SKILL_ROW_PAD_X))
        .py(px(theme::SKILL_ROW_PAD_Y))
        .child(
            // `min_w_0` overrides the flex default `min-width: auto`, which
            // otherwise holds this column at its unwrapped content width and
            // pushes the Stop button out of the dock — visibly clipped, and
            // overlapping the text it was pushed past. A stage line naming a
            // node and a tool is easily wider than 290px.
            div()
                .flex()
                .flex_col()
                // The trio this needs, and all three are load-bearing:
                // `flex_1` gives the column the leftover width to wrap into,
                // `min_w_0` lets it actually go below its content width (the
                // flex default is `min-width: auto`), and `flex_none` on the
                // button keeps the button out of the shrinking. With any one
                // missing, a stage line naming a node and a tool pushes Stop
                // out of a 290px dock — clipped, and overlapping the text.
                .flex_1()
                .min_w_0()
                .child(div().text_color(t.text_body).child(run.lane_label.clone()))
                .child(
                    div()
                        .text_size(px(theme::RIGHT_PANEL_LABEL_FONT_SIZE))
                        .text_color(t.text_muted)
                        .child(run.doing.clone()),
                ),
        )
        .child(
            // `button`, not `button_chip`: the chip is a fixed 20x20 icon box
            // (`BUTTON_CHIP_SIZE`), so a text label overflows it and gets cut
            // at the dock's edge. The other half of `min_w_0` above is the
            // `flex_none` — the text may shrink, this must not.
            div().flex_none().child(
                crate::ui::button(
                    // Both halves of the ref: two projects each have a lane
                    // `0`, and one id per row is what keeps their clicks apart.
                    SharedString::from(format!("flow-panel-stop-{}-{}", lane.project, lane.lane)),
                    strings::right_panel_flow_stop(),
                )
                .on_click(move |_, _window, cx| {
                    match workspace.update(cx, |ws, cx| ws.stop_flow_run_in(lane, cx)) {
                        Ok(()) => {}
                        Err(e) => daruda_store::observability::log_writer::LogWriter::log(
                            daruda_store::observability::error_report::ErrorReport::new(
                                "Flows panel: workspace gone while stopping a flow",
                            )
                            .severity(
                                daruda_store::observability::error_report::ErrorSeverity::Warning,
                            )
                            .at(file!(), line!())
                            .with_context("error", format!("{e}"))
                            .dedup("right_dock.flow.stop")
                            .build(),
                        ),
                    }
                }),
            ),
        )
}
