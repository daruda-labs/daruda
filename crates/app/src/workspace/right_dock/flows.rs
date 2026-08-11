//! Flows tab — what the active lane's flow runs are doing, and the one
//! thing there is to do about them.
//!
//! Scoped to the active lane, unlike the status bar chip. A run's history
//! lives in `<lane>/.daruda/flow-runs/`, so a panel spanning lanes could
//! not show a coherent past beside the present; the chip is what answers
//! the cross-lane question.
//!
//! Unlike the chip's popover this surface does not dismiss, which is why
//! it — and not the popover — is where a run's permission question gets
//! answered.

use gpui::{AnyElement, IntoElement, SharedString, div, prelude::*, px};

use crate::surface::strings;
use crate::ui::theme;
use crate::workspace::flow_ops::FlowRunRow;

use super::super::layout::{Dock, RightDockSnapshot};

pub(in crate::workspace) fn render(
    snap: &RightDockSnapshot,
    cx: &mut gpui::Context<Dock>,
) -> AnyElement {
    // The dock root sets no text colour or size, so each panel states its
    // own — every sibling here does, and the two spots that did not
    // rendered near-black against the dock.
    let mut body = super::right_panel_body().text_size(px(theme::RIGHT_PANEL_BODY_FONT_SIZE));
    body = body.child(crate::ui::SectionHeader::new(
        strings::right_panel_flow_live_heading(),
    ));
    if snap.flows.is_empty() {
        body = body.child(empty_state(cx));
    } else {
        body = body.children(snap.flows.iter().map(|run| run_row(run, snap, cx)));
    }
    if let Some(history) = snap.flow_history.as_ref() {
        body = body.child(crate::ui::Divider::horizontal());
        body = body.child(crate::ui::SectionHeader::new(
            strings::right_panel_flow_past_heading(),
        ));
        if history.runs().is_empty() {
            body = body.child(
                crate::ui::placeholder_text(strings::right_panel_flow_past_empty())
                    .text_size(px(theme::DOCK_PLACEHOLDER_FONT_SIZE))
                    .text_color(theme::current(cx).text_subtle),
            );
        } else {
            body = body
                .children(history.runs().iter().map(|run| past_row(run, snap, cx)))
                .child(retention_note(cx));
        }
    }
    body.into_any_element()
}

/// Says the list is capped so a run leaving it reads as retention rather
/// than as something lost. The number comes from the engine's own default
/// — the sweep is what enforces it.
fn retention_note(cx: &gpui::App) -> impl IntoElement {
    div()
        .text_size(px(theme::DOCK_PLACEHOLDER_FONT_SIZE))
        .text_color(theme::current(cx).text_subtle)
        .child(strings::right_panel_flow_retention(
            daruda_flow::marker::DEFAULT_KEEP_RUNS,
        ))
}

/// What an outcome reads as at a glance. Three statuses that mean very
/// different things rendered identically before this — and the one that
/// matters most (a run that died with the app) looked like a success.
fn status_color(status: daruda_flow::marker::RunStatus) -> gpui::Hsla {
    use daruda_flow::marker::RunStatus as S;
    match status {
        S::Done => theme::SUCCESS,
        S::Failed | S::Crashed => theme::ERROR,
        S::Running => theme::WARNING,
        // Nothing went wrong and nothing succeeded — saying either in
        // colour would be a claim the evidence does not support.
        S::Canceled | S::Unknown => theme::TEXT_SUBTLE,
    }
}

/// One past run: when it started, how it ended, and its report on click.
/// A run with no report is not clickable; which those are was decided when
/// the history was read.
fn past_row(
    run: &crate::workspace::flow_history::FlowRunEntry,
    snap: &RightDockSnapshot,
    cx: &gpui::App,
) -> impl IntoElement {
    let t = theme::current(cx);
    let workspace = snap.workspace.clone();

    let row_hover_bg = t.skill_row_hover_bg;
    div()
        .id(SharedString::from(format!(
            "flow-past-{}",
            run.dir.display()
        )))
        .flex()
        .flex_row()
        .items_center()
        .w_full()
        .min_w_0()
        .overflow_hidden()
        .gap(px(theme::RIGHT_PANEL_ROW_GAP))
        .px(px(theme::SKILL_ROW_PAD_X))
        .py(px(theme::SKILL_ROW_PAD_Y))
        .rounded(px(theme::SKILL_ROW_RADIUS))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_color(t.text_body)
                .child(run.started.clone()),
        )
        .child(
            div()
                .flex_shrink()
                .min_w_0()
                .truncate()
                .text_size(px(theme::RIGHT_PANEL_LABEL_FONT_SIZE))
                .text_color(status_color(run.status))
                .child(strings::flow_run_status(run.status)),
        )
        .children(resume_button(run, snap))
        .when_some(run.report.clone(), |row, report| {
            row.cursor_pointer()
                .hover(move |s| s.bg(row_hover_bg))
                .on_click(move |_, window, cx| {
                    match workspace.update(cx, |ws, cx| ws.open_flow_report(&report, window, cx)) {
                        Ok(()) => {}
                        Err(e) => daruda_store::observability::log_writer::LogWriter::log(
                            daruda_store::observability::error_report::ErrorReport::new(
                                "Flows panel: workspace gone while opening a run report",
                            )
                            .severity(
                                daruda_store::observability::error_report::ErrorSeverity::Warning,
                            )
                            .at(file!(), line!())
                            .with_context("error", format!("{e}"))
                            .dedup("right_dock.flow.report")
                            .build(),
                        ),
                    }
                })
        })
}

/// The way back into a run that was killed.
///
/// Only for those: `is_resumable` is the engine's own answer, asked here
/// rather than restated, so the button and the refusal cannot disagree
/// about what may be continued.
fn resume_button(
    run: &crate::workspace::flow_history::FlowRunEntry,
    snap: &RightDockSnapshot,
) -> Option<impl IntoElement + use<>> {
    if !daruda_flow::resume::is_resumable(run.status) {
        return None;
    }
    let workspace = snap.workspace.clone();
    let run_dir = run.dir.clone();
    Some(
        div().flex_none().child(
            crate::ui::button(
                SharedString::from(format!("flow-resume-{}", run.dir.display())),
                strings::flow_resume_action(),
            )
            .on_click(move |_, window, cx| {
                // Asked first: the interrupted node starts over, so whatever
                // it had already done it does again. Nobody should meet that
                // by having clicked a row.
                let workspace = workspace.clone();
                let run_dir = run_dir.clone();
                crate::workspace::dialog_helpers::open_confirm_dialog(
                    strings::flow_resume_confirm_title(),
                    strings::flow_resume_confirm_body(),
                    strings::flow_resume_action(),
                    crate::ui::ButtonVariant::Primary,
                    move |_, _window, cx| {
                        let run_dir = run_dir.clone();
                        match workspace.update(cx, |ws, cx| ws.resume_flow_run(&run_dir, cx)) {
                            Ok(()) => {}
                            Err(e) => daruda_store::observability::log_writer::LogWriter::log(
                                daruda_store::observability::error_report::ErrorReport::new(
                                    "Flows panel: workspace gone while continuing a run",
                                )
                                .severity(
                                    daruda_store::observability::error_report::ErrorSeverity::Warning,
                                )
                                .at(file!(), line!())
                                .with_context("error", format!("{e}"))
                                .dedup("right_dock.flow.resume")
                                .build(),
                            ),
                        }
                    },
                    window,
                    cx,
                );
            }),
        ),
    )
}

/// No run in this lane. Deliberately not "no runs anywhere" — a run in
/// another lane is still going, and the chip is saying so.
fn empty_state(cx: &gpui::App) -> AnyElement {
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
fn run_row(run: &FlowRunRow, snap: &RightDockSnapshot, cx: &gpui::App) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(px(theme::GAP_SM))
        .child(run_summary(run, snap, cx))
        .children(
            run.asking
                .as_ref()
                .map(|ask| ask_block(run.lane, ask, snap, cx)),
        )
}

/// The question, what it is about, and one button per option the adapter
/// offered — `AllowAlways` included. The engine never *selects* that one
/// (it outlives the session), but a person looking at it choosing it is an
/// informed decision, and hiding an option the agent offered would be us
/// answering on their behalf.
fn ask_block(
    lane: daruda_store::project::LaneRef,
    ask: &crate::workspace::flow_ops::AskRowData,
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
