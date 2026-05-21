//! Per-worktree Claude session indicators rendered inside the
//! dock row.
//!
//! Two pieces:
//! - `claude_status_cell` — the leading dot column (always present
//!   so rows align whether or not a session is bound).
//! - `claude_badges_row` — the multi-session sub-row shown only when
//!   a worktree has ≥ 2 concurrent Claude sessions. The badge
//!   matching the focused tab gets an outline ring (Phase E) so the
//!   user can tell which sibling their terminal is talking to.

use crate::ui::theme;
use daruda_claude::SessionStatus;
use gpui::{AnyElement, IntoElement, ParentElement, Styled, div, prelude::*, px};

use crate::surface::strings as surface_strings;
use crate::ui::{AgentStatusBadge, IndicatorSize};

/// Fixed-width cell that holds the leading Claude-status indicator.
/// Empty (just the spacer width) when no Claude session is associated
/// with the worktree.
pub(super) fn claude_status_cell(state: Option<SessionStatus>, cx: &gpui::App) -> AnyElement {
    let cell = div()
        .flex_none()
        .w(px(theme::STATUS_INDICATOR_CELL_WIDTH))
        .h_full()
        .flex()
        .items_center()
        .justify_center();
    match state {
        Some(s) => cell
            .child(AgentStatusBadge::for_status(s, IndicatorSize::Leading, cx))
            .into_any_element(),
        None => cell.into_any_element(),
    }
}

/// Phase D — sub-row badge strip. Shown when a worktree has ≥ 2
/// concurrent Claude sessions; each badge is a small per-session
/// indicator (`IndicatorSize::Badge`) so the user can tell which of
/// several agents needs attention.
///
/// Phase E — `active_session_id` highlights the badge corresponding
/// to the focused tab's claude session with an outline ring, so the
/// user can tell which sibling session their terminal is talking to.
/// Each badge also gets a hover tooltip showing the session_id
/// prefix.
pub(super) fn claude_badges_row(
    sessions: &[(String, SessionStatus)],
    active_session_id: Option<&str>,
    cx: &gpui::App,
) -> impl IntoElement + use<> {
    let count = sessions.len();
    let label = format!("{count}{}", surface_strings::claude_sessions_label_suffix());
    let faint_text = theme::current(cx).faint_text;
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
                            surface_strings::CLAUDE_BADGE_TOOLTIP_ELLIPSIS,
                            surface_strings::claude_badge_tooltip_active_suffix()
                        )
                    } else {
                        format!("{prefix}{}", surface_strings::CLAUDE_BADGE_TOOLTIP_ELLIPSIS)
                    };
                    let mut indicator =
                        AgentStatusBadge::for_status(*state, IndicatorSize::Badge, cx);
                    if is_active {
                        indicator = indicator.active();
                    }
                    div()
                        .id(("claude-badge", idx))
                        .flex_none()
                        .child(indicator)
                        .tooltip(crate::ui::tooltip::text(tooltip_text))
                }),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn empty_active_id_means_no_outline(cx: &mut gpui::TestAppContext) {
        crate::test_support::init_gpui_component(cx);
        cx.update(|cx| {
            // Sanity: passing None for active_session_id never panics
            // and produces a valid element. We can't assert the visual
            // outcome here without a render harness, but we can guard
            // against shape regressions.
            let sessions = vec![("abc".to_string(), SessionStatus::Idle)];
            let _ = claude_badges_row(&sessions, None, cx);
        });
    }

    #[gpui::test]
    fn cell_width_is_consistent_across_states(cx: &mut gpui::TestAppContext) {
        crate::test_support::init_gpui_component(cx);
        cx.update(|cx| {
            // Both populated and empty cells must reserve the same
            // horizontal space so worktree rows in the list don't visually
            // shift when a session attaches.
            let with = claude_status_cell(Some(SessionStatus::Working), cx);
            let without = claude_status_cell(None, cx);
            // Smoke check — each call returns an AnyElement; the actual
            // width is enforced by `STATUS_INDICATOR_CELL_WIDTH`.
            let _ = (with, without);
        });
    }
}
