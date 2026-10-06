//! History row actions and status styling shared by the run table.
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
        .child(strings::flow::panel_retention(
            daruda_flow::marker::DEFAULT_KEEP_RUNS,
        ))
}

/// What an outcome reads as at a glance. Three statuses that mean very
/// different things rendered identically before this — and the one that
/// matters most (a run that died with the app) looked like a success.
pub(super) fn status_color(status: daruda_flow::marker::RunStatus) -> gpui::Hsla {
    use daruda_flow::marker::RunStatus as S;
    match status {
        S::Done => theme::SUCCESS,
        S::Failed | S::Crashed => theme::ERROR,
        S::Running => theme::WARNING,
        // Stopped rather than failed, and resumable — the same reading a
        // run still going gets, because both are unfinished business.
        S::Stalled => theme::WARNING,
        // Nothing went wrong and nothing succeeded — saying either in
        // colour would be a claim the evidence does not support.
        S::Canceled | S::Unknown => theme::TEXT_SUBTLE,
    }
}

/// The way back into a run that was killed.
///
/// Only for those: `is_resumable` is the engine's own answer, asked here
/// rather than restated, so the button and the refusal cannot disagree
/// about what may be continued.
pub(super) fn resume_button(
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
                strings::flow::resume_action(),
            )
            .on_click(move |_, window, cx| {
                // Asked first: the interrupted node starts over, so whatever
                // it had already done it does again. Nobody should meet that
                // by having clicked a row.
                let workspace = workspace.clone();
                let run_dir = run_dir.clone();
                crate::workspace::dialog_helpers::open_confirm_dialog(
                    strings::flow::resume_confirm_title(),
                    strings::flow::resume_confirm_body(),
                    strings::flow::resume_action(),
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
