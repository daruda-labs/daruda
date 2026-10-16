//! Status bar's Flow segment — what the flow runs this app started are
//! doing, and the one thing there is to do about them.
//!
//! Until this existed a run was invisible unless you reopened the palette.
//! This is the status surface: the chip says a run is going, and the
//! dropdown gives each one a Stop.
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
use crate::ui::button_status_pill_bare;
use crate::ui::theme;
use crate::workspace::Workspace;
use gpui::{AnyElement, App, IntoElement, SharedString, WeakEntity, div, prelude::*, px};

const FLOW_PANEL_WIDTH: f32 = 300.0;
const FLOW_SECTION_GAP: f32 = 4.0;
const FLOW_ROW_GAP: f32 = 10.0;
const FLOW_PANEL_MAX_HEIGHT: f32 = 360.0;

// The row lives with the ops that build it: this segment and the Flows panel
// read the same shape, and a type owned by one of its two consumers would make
// the other import across surfaces.
pub(super) use crate::workspace::flow_rows::FlowRunRow;

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
    let asking = runs.iter().any(|run| !run.asking.is_empty());
    let label = SharedString::from(trigger_label(runs, density));
    let summary = SharedString::from(crate::surface::strings::flow::chip_summary(runs.len()));
    let rows = runs.to_vec();
    let t = theme::current(cx);
    Some(
        super::menu::popover("status-flow-popover", cx)
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
            .content(move |_, window, cx| flow_panel(&rows, workspace.clone(), window, cx)),
    )
}

/// Waiting questions across every run — what the chip and the panel header
/// put first, since work finishes on its own and a question does not.
fn waiting_count(runs: &[FlowRunRow]) -> usize {
    // Questions, not runs: one run can hold several once its nodes run
    // together.
    runs.iter().map(|run| run.asking.len()).sum()
}

/// The panel header's state and its tone: the waiting count, tinted, while
/// anyone is waiting; otherwise how many runs are going.
fn header_state(runs: &[FlowRunRow]) -> (String, super::menu::Tone) {
    match waiting_count(runs) {
        0 => (
            crate::surface::strings::flow::menu_running(runs.len()),
            super::menu::Tone::Plain,
        ),
        waiting => (
            crate::surface::strings::flow::menu_waiting(waiting),
            super::menu::Tone::Themed(|cx| theme::current(cx).flow_attention),
        ),
    }
}

/// One run names what it is doing; several name only how many, because a
/// status bar has no room for two and the dropdown is one click away.
fn trigger_label(runs: &[FlowRunRow], density: StatusBarDensity) -> String {
    // A waiting run outranks a working one however many there are; a chip
    // saying "1" over two unanswered questions would leave one sitting.
    let waiting = waiting_count(runs);
    if waiting > 0 {
        return crate::surface::strings::flow::chip_asking(waiting);
    }
    match runs {
        [only] if density == StatusBarDensity::Full => {
            crate::surface::strings::flow::chip_one(&only.doing)
        }
        _ => crate::surface::strings::flow::chip_many(runs.len()),
    }
}

fn flow_panel(
    runs: &[FlowRunRow],
    workspace: WeakEntity<Workspace>,
    window: &gpui::Window,
    cx: &App,
) -> AnyElement {
    let t = theme::current(cx);
    let (state, tone) = header_state(runs);
    super::menu::panel(window, FLOW_PANEL_WIDTH, FLOW_PANEL_MAX_HEIGHT)
        .gap(px(FLOW_SECTION_GAP))
        .text_size(px(theme::STATUS_BAR_FONT_SIZE))
        .child(super::menu::panel_header(
            crate::surface::strings::flow::menu_title(),
            Some(state.into()),
            tone,
            cx,
        ))
        .child(super::menu::panel_separator())
        .child(super::menu::panel_body(crate::ui::scroll_area("status-flow-scroll", px(FLOW_PANEL_MAX_HEIGHT),
            div().flex().flex_col().pr(px(theme::POPOVER_ACTION_GUTTER)).gap(px(FLOW_SECTION_GAP)).children(runs.iter().map(|run| {
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
                        crate::surface::strings::flow::stop_action(),
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
                    })
                    .debug_selector(move || format!("status-flow-stop-{}-{}", lane.project, lane.lane)),
                    ),
                )
        }))).fill().max_h(px(FLOW_PANEL_MAX_HEIGHT))))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use daruda_store::project::LaneRef;

    struct PanelProbe {
        rows: Vec<FlowRunRow>,
        workspace: WeakEntity<Workspace>,
    }

    impl gpui::Render for PanelProbe {
        fn render(
            &mut self,
            window: &mut gpui::Window,
            cx: &mut gpui::Context<Self>,
        ) -> impl IntoElement {
            flow_panel(&self.rows, self.workspace.clone(), window, cx)
        }
    }

    #[gpui::test]
    fn scrolling_flow_actions_clear_the_scrollbar_hit_strip(cx: &mut gpui::TestAppContext) {
        let (_, workspace) = crate::workspace::tests::build_workspace(cx);
        let workspace = workspace.downgrade();
        let window = cx.add_window(move |_, _| PanelProbe {
            rows: (0..40)
                .map(|lane| FlowRunRow {
                    lane: LaneRef { project: 1, lane },
                    ..row("implement")
                })
                .collect(),
            workspace,
        });
        let mut vcx = gpui::VisualTestContext::from_window(window.into(), cx);
        vcx.run_until_parked();
        let body = vcx.debug_bounds("status-flow-scroll-wrapper").unwrap();
        let stop = vcx.debug_bounds("status-flow-stop-1-0").unwrap();
        assert!(body.size.height > px(0.));
        assert!(body.size.height <= px(FLOW_PANEL_MAX_HEIGHT));
        assert!(stop.right() <= body.right() - px(theme::POPOVER_ACTION_GUTTER));
    }

    fn row(doing: &str) -> FlowRunRow {
        FlowRunRow {
            run_dir: std::path::PathBuf::from("run"),
            source: crate::workspace::flow_request::FlowSource::File("flow.yaml".into()),
            lane: LaneRef {
                project: 1,
                lane: 0,
            },
            lane_label: "temp / main".into(),
            doing: doing.to_string().into(),
            asking: Vec::new(),
        }
    }

    fn question(ask_id: u64) -> crate::workspace::flow_rows::AskRowData {
        crate::workspace::flow_rows::AskRowData {
            ask_id,
            asked: "implement is waiting".into(),
            tool: "Bash".into(),
            detail: None,
            options: Vec::new(),
        }
    }

    fn waiting_row() -> FlowRunRow {
        FlowRunRow {
            asking: vec![question(1)],
            ..row("implement")
        }
    }

    /// The header counts runs while none waits, and questions — a run's own
    /// plus the ones its parallel nodes hold — in the attention tone once any does.
    #[test]
    fn the_header_counts_runs_until_someone_waits() {
        let (state, tone) = header_state(&[row("verdict"), row("implement")]);
        assert_eq!(state, crate::surface::strings::flow::menu_running(2));
        assert!(matches!(tone, super::super::menu::Tone::Plain));

        let parallel = FlowRunRow {
            asking: (1..=3).map(question).collect(),
            ..waiting_row()
        };
        let (state, tone) = header_state(&[row("verdict"), parallel]);
        assert_eq!(state, crate::surface::strings::flow::menu_waiting(3));
        assert!(matches!(tone, super::super::menu::Tone::Themed(_)));
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
