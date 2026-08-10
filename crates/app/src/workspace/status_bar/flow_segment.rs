//! Status bar's Flow segment — what the flow runs this app started are
//! doing, and the one thing there is to do about them.
//!
//! Until this existed a run was invisible unless you reopened the palette,
//! which is where design §14 wanted a stop affordance and decision L had
//! to put one for want of a surface. This is that surface: the chip says a
//! run is going, and the dropdown gives each one a Stop.
//!
//! A run another *process* holds is not here — this lists what this app can
//! actually stop. The lock is what answers the wider question, and the
//! picker is where that answer is already given.

use super::StatusBarDensity;
use crate::ui::theme;
use crate::ui::{Divider, Popover, button_status_pill};
use crate::workspace::Workspace;
use gpui::{AnyElement, App, IntoElement, SharedString, WeakEntity, div, prelude::*, px};

use daruda_store::project::LaneRef;

const FLOW_PANEL_WIDTH: f32 = 300.0;
const FLOW_SECTION_GAP: f32 = 4.0;
const FLOW_ROW_GAP: f32 = 10.0;

/// One running flow, flattened for the render pass.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::workspace) struct FlowRunRow {
    pub lane: LaneRef,
    /// `<project>/<lane>` — a run in another lane is the case this whole
    /// segment exists for, so the row has to say which lane it is.
    pub lane_label: SharedString,
    /// What that run is doing, already worded (see `RunStage`).
    pub doing: SharedString,
}

/// `None` when nothing is running: the chip disappears rather than
/// reading `0`, because unlike ports there is no "currently none" state a
/// user is watching for.
pub(super) fn render(
    runs: &[FlowRunRow],
    density: StatusBarDensity,
    workspace: WeakEntity<Workspace>,
    cx: &App,
) -> Option<impl IntoElement> {
    if runs.is_empty() {
        return None;
    }
    let label = SharedString::from(trigger_label(runs, density));
    let summary = SharedString::from(crate::surface::strings::status_bar_flow_summary(runs.len()));
    let rows = runs.to_vec();
    Some(
        Popover::new("status-flow-popover")
            .trigger(
                button_status_pill("status-flow", label, cx)
                    .text_size(px(theme::STATUS_BAR_FONT_SIZE))
                    .debug_selector(|| "status-flow-trigger".into())
                    .tooltip(summary.clone()),
            )
            .content(move |_, _window, cx| {
                flow_panel(&rows, summary.clone(), workspace.clone(), cx)
            }),
    )
}

/// One run names what it is doing; several name only how many, because a
/// status bar has no room for two and the dropdown is one click away.
fn trigger_label(runs: &[FlowRunRow], density: StatusBarDensity) -> String {
    match runs {
        [only] if density == StatusBarDensity::Full => {
            crate::surface::strings::status_bar_flow_chip_one(&only.doing)
        }
        _ => crate::surface::strings::status_bar_flow_chip_many(runs.len()),
    }
}

fn flow_panel(
    runs: &[FlowRunRow],
    summary: SharedString,
    workspace: WeakEntity<Workspace>,
    cx: &App,
) -> AnyElement {
    let t = theme::current(cx);
    div()
        .flex()
        .flex_col()
        .w(px(FLOW_PANEL_WIDTH))
        .gap(px(FLOW_SECTION_GAP))
        .text_size(px(theme::STATUS_BAR_FONT_SIZE))
        .child(div().child(summary))
        .child(Divider::horizontal())
        .children(runs.iter().map(|run| {
            let workspace = workspace.clone();
            let lane = run.lane;
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(FLOW_ROW_GAP))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .child(div().child(run.lane_label.clone()))
                        .child(
                            div()
                                .text_color(t.text_subtle)
                                .child(run.doing.clone()),
                        ),
                )
                .child(
                    crate::ui::button_chip(
                        // Both halves: a `LaneRef` is (project, lane), and two
                        // projects each have a lane `0` — one id per row is
                        // what keeps their clicks apart.
                        SharedString::from(format!(
                            "status-flow-stop-{}-{}",
                            lane.project, lane.lane
                        )),
                        crate::surface::strings::status_bar_flow_stop(),
                    )
                    .on_click(move |_, _window, cx| {
                        match workspace.update(cx, |ws, cx| ws.stop_flow_run_in(lane, cx)) {
                            Ok(()) => {}
                            Err(e) => daruda_store::observability::log_writer::LogWriter::log(
                                daruda_store::observability::error_report::ErrorReport::new(
                                    "Status bar: workspace gone while stopping a flow",
                                )
                                .severity(
                                    daruda_store::observability::error_report::ErrorSeverity::Warning,
                                )
                                .at(file!(), line!())
                                .with_context("error", format!("{e}"))
                                .dedup("status_bar.flow.stop")
                                .build(),
                            ),
                        }
                    }),
                )
        }))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(doing: &str) -> FlowRunRow {
        FlowRunRow {
            lane: LaneRef {
                project: 1,
                lane: 0,
            },
            lane_label: "temp / main".into(),
            doing: doing.to_string().into(),
        }
    }

    /// One run has room to say what it is doing; several do not, and a
    /// count is the honest thing to show instead of one of them.
    #[test]
    fn the_chip_names_the_node_only_when_there_is_one_run() {
        let one = trigger_label(&[row("verdict")], StatusBarDensity::Full);
        assert!(one.contains("verdict"), "{one}");

        let many = trigger_label(&[row("verdict"), row("gate")], StatusBarDensity::Full);
        assert!(!many.contains("verdict"), "{many}");
        assert!(many.contains('2'), "{many}");
    }
}
