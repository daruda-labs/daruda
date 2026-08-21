//! The Flows panel's third section: the runs that already finished.
//!
//! A row says when a run started and how it ended, opens its report on
//! click, and offers a way back into one that was killed. The note under
//! the list says the list is capped.

use gpui::{IntoElement, SharedString, div, prelude::*, px};

use crate::surface::strings;
use crate::ui::theme;
use crate::workspace::layout::RightDockSnapshot;

/// Says the list is capped so a run leaving it reads as retention rather
/// than as something lost. The number comes from the engine's own default
/// — the sweep is what enforces it.
pub(super) fn retention_note(cx: &gpui::App) -> impl IntoElement {
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
pub(super) fn past_row(
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
    // Taken from the same snapshot as `run_dir`, so the directory and the
    // lane it lives in are one lane's worth even if the active lane moves
    // on before the click.
    let lane = snap.flow_lane;
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
                        match workspace.update(cx, |ws, cx| ws.resume_flow_run(lane, &run_dir, cx))
                        {
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
