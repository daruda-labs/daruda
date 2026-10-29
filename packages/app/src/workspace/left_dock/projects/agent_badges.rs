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
    let badge = resolved_badge(state, outcome, IndicatorSize::Leading, cx);
    cell.children(badge.map(|badge| badge.with_metrics(metrics)))
        .into_any_element()
}

/// Preserve the host's live/outcome priority before choosing either artwork theme.
pub(in crate::workspace::left_dock) fn resolved_badge(
    state: Option<SessionStatus>,
    outcome: Option<AgentOutcome>,
    size: IndicatorSize,
    cx: &gpui::App,
) -> Option<AgentStatusBadge> {
    match TabIndicator::resolve(state, outcome) {
        Some(TabIndicator::Done) => Some(AgentStatusBadge::for_outcome(
            AgentOutcome::Completed,
            size,
            cx,
        )),
        Some(TabIndicator::Errored) => Some(AgentStatusBadge::for_outcome(
            AgentOutcome::Errored,
            size,
            cx,
        )),
        _ => state.map(|status| AgentStatusBadge::for_status(status, size, cx)),
    }
}

#[cfg(test)]
mod tests;
