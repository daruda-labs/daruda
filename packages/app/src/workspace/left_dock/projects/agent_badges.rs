//! Per-lane agent session indicators rendered inside the
//! dock row.
//!
//! Two pieces:
//! - `agent_status_cell` — the leading dot column (always present
//!   so rows align whether or not a session is bound).
//! - `agent_badges_row` — the multi-session sub-row shown only when
//!   a lane has ≥ 2 concurrent agent sessions. The badge
//!   matching the focused tab gets an outline ring so the
//!   user can tell which sibling their terminal is talking to.

use crate::ui::theme;
use daruda_agent::{AgentOutcome, SessionStatus};
use gpui::{AnyElement, IntoElement, ParentElement, Styled, div, prelude::*, px};

use crate::surface::strings as surface_strings;
use crate::ui::{AgentStatusBadge, IndicatorSize};
use crate::workspace::tab_indicator::TabIndicator;

/// Fixed-width cell that holds the leading agent-status indicator.
/// Empty (just the spacer width) when no agent session or unseen outcome
/// is associated with the lane.
pub(super) fn agent_status_cell(
    state: Option<SessionStatus>,
    outcome: Option<AgentOutcome>,
    cx: &gpui::App,
) -> AnyElement {
    let cell = div()
        .flex_none()
        .w(px(theme::STATUS_INDICATOR_CELL_WIDTH))
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
    cell.children(badge).into_any_element()
}

/// Sub-row badge strip shown when a lane has ≥ 2 concurrent agent
/// sessions; each badge is a small per-session indicator
/// (`IndicatorSize::Badge`) so the user can tell which of several
/// agents needs attention.
///
/// `active_session_id` highlights the badge for the focused tab's
/// agent session with an outline ring, and each badge gets a hover
/// tooltip showing the session_id prefix.
pub(super) fn agent_badges_row(
    sessions: &[(String, SessionStatus)],
    active_session_id: Option<&str>,
    cx: &gpui::App,
) -> impl IntoElement + use<> {
    let count = sessions.len();
    let label = format!(
        "{count}{}",
        surface_strings::claude::sessions_label_suffix()
    );
    let faint_text = theme::current(cx).text_subtle;
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::STATUS_BADGES_ROW_GAP))
        .mt(px(theme::STATUS_BADGES_ROW_TOP_MARGIN))
        .text_size(px(theme::STATUS_BADGES_LABEL_FONT_SIZE))
        .text_color(faint_text)
        .child(
            div()
                .flex_none()
                .mr(px(theme::STATUS_BADGES_LABEL_GAP))
                .child(label),
        )
        .children(
            sessions
                .iter()
                .enumerate()
                .map(|(idx, (session_id, state))| {
                    let is_active = active_session_id == Some(session_id.as_str());
                    let prefix: String = session_id
                        .chars()
                        .take(theme::STATUS_BADGE_TOOLTIP_SESSION_PREFIX_LEN)
                        .collect();
                    let tooltip_text = if is_active {
                        format!(
                            "{prefix}{}{}",
                            crate::surface::glyphs::AGENT_BADGE_TOOLTIP_ELLIPSIS,
                            surface_strings::claude::badge_active_suffix()
                        )
                    } else {
                        format!(
                            "{prefix}{}",
                            crate::surface::glyphs::AGENT_BADGE_TOOLTIP_ELLIPSIS
                        )
                    };
                    let mut indicator =
                        AgentStatusBadge::for_status(*state, IndicatorSize::Badge, cx);
                    if is_active {
                        indicator = indicator.active();
                    }
                    div()
                        .id(("agent-badge", idx))
                        .flex_none()
                        .child(indicator)
                        .tooltip(crate::ui::tooltip::text(tooltip_text))
                }),
        )
}

#[cfg(test)]
mod tests;
