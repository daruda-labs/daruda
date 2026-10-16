//! Leading lane status cells shared by the expanded tree and collapsed rail.

use crate::ui::theme;
use daruda_agent::{AgentOutcome, SessionStatus};
use gpui::{AnyElement, IntoElement, ParentElement, Styled, div, px};

use crate::ui::{AgentStatusBadge, IndicatorSize};
use crate::workspace::tab_indicator::TabIndicator;

/// Fixed-width cell that holds the leading agent-status indicator.
/// Empty (just the spacer width) when no agent session or unseen outcome
/// is associated with the lane.
pub(in crate::workspace::left_dock) fn agent_status_cell(
    state: Option<SessionStatus>,
    outcome: Option<AgentOutcome>,
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
    let badge = match TabIndicator::resolve(state, outcome) {
        Some(TabIndicator::Done) => Some(AgentStatusBadge::new(
            SessionStatus::Idle,
            IndicatorSize::Leading,
            theme::current(cx).status_idle_dark,
        )),
        Some(TabIndicator::Errored) => Some(AgentStatusBadge::for_status(
            SessionStatus::Failed,
            IndicatorSize::Leading,
            cx,
        )),
        _ => state.map(|status| AgentStatusBadge::for_status(status, IndicatorSize::Leading, cx)),
    };
    cell.children(badge.map(|badge| badge.with_metrics(metrics)))
        .into_any_element()
}

#[cfg(test)]
mod tests;
