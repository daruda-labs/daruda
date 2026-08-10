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
//!
//! This segment **reports and navigates; it does not answer.** A question is
//! answered in the Flows panel, which is lane-scoped and does not dismiss —
//! a popover does, and a question with a clock running must not be losable
//! by clicking away. Clicking a row goes to the lane that owns it.

use super::StatusBarDensity;
use crate::ui::theme;
use crate::ui::{Divider, Popover, button_status_pill_bare};
use crate::workspace::Workspace;
use gpui::{AnyElement, App, IntoElement, SharedString, WeakEntity, div, prelude::*, px};

const FLOW_PANEL_WIDTH: f32 = 300.0;
const FLOW_SECTION_GAP: f32 = 4.0;
const FLOW_ROW_GAP: f32 = 10.0;

// The row lives with the ops that build it: this segment and the Flows panel
// read the same shape, and a type owned by one of its two consumers would make
// the other import across surfaces.
pub(in crate::workspace) use crate::workspace::flow_ops::FlowRunRow;

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
    let asking = runs.iter().any(|run| run.asking.is_some());
    let label = SharedString::from(trigger_label(runs, density));
    let summary = SharedString::from(crate::surface::strings::status_bar_flow_summary(runs.len()));
    let rows = runs.to_vec();
    let t = theme::current(cx);
    Some(
        Popover::new("status-flow-popover")
            .trigger(
                // Tinted only while a run is waiting on a person: this is
                // the one surface a user cannot hide (Flow is not a
                // `StatusBarItem`), so it is the only place that can be
                // trusted to ask for attention at all.
                button_status_pill_bare("status-flow", cx)
                    .text_size(px(theme::STATUS_BAR_FONT_SIZE))
                    .debug_selector(|| "status-flow-trigger".into())
                    .tooltip(summary.clone())
                    .child(if asking {
                        div().text_color(t.flow_attention).child(label)
                    } else {
                        div().child(label)
                    }),
            )
            .content(move |_, _window, cx| {
                flow_panel(&rows, summary.clone(), workspace.clone(), cx)
            }),
    )
}

/// One run names what it is doing; several name only how many, because a
/// status bar has no room for two and the dropdown is one click away.
fn trigger_label(runs: &[FlowRunRow], density: StatusBarDensity) -> String {
    // A waiting run outranks a working one however many there are: work
    // finishes on its own, a question does not.
    let waiting = runs.iter().filter(|run| run.asking.is_some()).count();
    if waiting > 0 {
        return crate::surface::strings::status_bar_flow_chip_asking(waiting);
    }
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
            let reveal = workspace.clone();
            let lane = run.lane;
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(FLOW_ROW_GAP))
                .child(
                    // The primary click navigates to the lane that owns the
                    // run; Stop stays its own button so reaching for one is
                    // never the other. Answering happens in the panel this
                    // lands on — a popover dismisses, and a question with a
                    // clock running must not be losable by clicking away.
                    div()
                        .id(SharedString::from(format!(
                            "status-flow-go-{}-{}",
                            lane.project, lane.lane
                        )))
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .cursor_pointer()
                        .child(div().child(run.lane_label.clone()))
                        .child(div().text_color(t.text_subtle).child(run.doing.clone()))
                        .on_click(move |_, window, cx| {
                            match reveal.update(cx, |ws, cx| ws.reveal_flow_run(lane, window, cx)) {
                                Ok(()) => {}
                                Err(e) => daruda_store::observability::log_writer::LogWriter::log(
                                    daruda_store::observability::error_report::ErrorReport::new(
                                        "Status bar: workspace gone while revealing a flow run",
                                    )
                                    .severity(
                                        daruda_store::observability::error_report::ErrorSeverity::Warning,
                                    )
                                    .at(file!(), line!())
                                    .with_context("error", format!("{e}"))
                                    .dedup("status_bar.flow.reveal")
                                    .build(),
                                ),
                            }
                        }),
                )
                .child(
                    div().flex_none().child(
                    crate::ui::button(
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
                    ),
                )
        }))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use daruda_store::project::LaneRef;

    fn row(doing: &str) -> FlowRunRow {
        FlowRunRow {
            lane: LaneRef {
                project: 1,
                lane: 0,
            },
            lane_label: "temp / main".into(),
            doing: doing.to_string().into(),
            asking: None,
        }
    }

    fn waiting_row() -> FlowRunRow {
        FlowRunRow {
            asking: Some(crate::workspace::flow_ops::AskRowData {
                ask_id: 1,
                tool: "Bash".into(),
                detail: None,
                options: Vec::new(),
            }),
            ..row("implement")
        }
    }

    /// A run waiting on a person outranks whatever the others are doing,
    /// at any density. Work finishes on its own; a question does not, and
    /// the chip is the only surface a user cannot hide.
    #[test]
    fn a_waiting_run_takes_the_chip_over_a_working_one() {
        for density in [StatusBarDensity::Full, StatusBarDensity::Compact] {
            let label = trigger_label(&[row("verdict"), waiting_row()], density);
            assert!(
                !label.contains("verdict"),
                "a working node held the chip while someone was waiting: {label}"
            );
            assert!(label.contains('1'), "{label}");
        }
        // And with nobody waiting it goes back to reporting.
        let plain = trigger_label(&[row("verdict")], StatusBarDensity::Full);
        assert!(plain.contains("verdict"), "{plain}");
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
