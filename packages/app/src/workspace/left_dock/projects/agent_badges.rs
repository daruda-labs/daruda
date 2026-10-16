//! Leading lane status cells shared by the expanded tree and collapsed rail.

use crate::ui::theme;
use daruda_agent::SessionStatus;
use gpui::{AnyElement, IntoElement, ParentElement, Styled, div, px};

use crate::ui::{AgentStatusBadge, IndicatorSize};

/// Fixed-width cell that holds the leading agent-status indicator.
/// Empty (just the spacer width) when no agent session is associated
/// with the lane.
pub(in crate::workspace::left_dock) fn agent_status_cell(
    state: Option<SessionStatus>,
    metrics: theme::UiMetrics,
    slot_width: f32,
    cx: &gpui::App,
) -> AnyElement {
    let cell = div()
        .flex_none()
        .w(px(slot_width))
        .h_full()
        .flex()
        .items_center()
        .justify_center();
    match state {
        Some(s) => cell
            .child(
                AgentStatusBadge::for_status(s, IndicatorSize::Leading, cx).with_metrics(metrics),
            )
            .into_any_element(),
        None => cell.into_any_element(),
    }
}

#[cfg(test)]
mod tests {
    use crate::ui::theme;
    #[gpui::test]
    fn empty_and_working_cells_render(cx: &mut gpui::TestAppContext) {
        crate::test_support::init_gpui_component(cx);
        cx.update(|cx| {
            let _ =
                super::agent_status_cell(None, Default::default(), theme::CONTROL_TARGET_SIZE, cx);
            let _ = super::agent_status_cell(
                Some(daruda_agent::SessionStatus::Working),
                Default::default(),
                theme::CONTROL_TARGET_SIZE,
                cx,
            );
        });
    }
}
