//! The agent's live execution plan: progress count, per-entry colours, and the
//! collapsible bottom plan region.

use daruda_acp::{PlanEntryView, PlanStatus};
use gpui::{AnyElement, App, Hsla, IntoElement, SharedString, div, prelude::*, px};

use super::pulse_opacity;
use crate::surface::strings as s;
use crate::ui::theme;
use crate::ui::{ButtonVariants as _, Disclosure, IconName, button_bare, disclosure};
use crate::workspace::main_area::agent_chat_pane::view::AgentChatView;
use crate::workspace::main_area::pane_tree::PaneId;

/// `(completed, total)`; callers guard empty plans before colour logic.
fn plan_progress(plan: &[PlanEntryView]) -> (usize, usize) {
    let done = plan
        .iter()
        .filter(|e| e.status == PlanStatus::Completed)
        .count();
    (done, plan.len())
}

/// Filled status dot plus colour; completed reuses diff-add green because
/// there is no dedicated success token.
fn plan_status_glyph(
    status: PlanStatus,
    t: &theme::DarudaTheme,
    dim: f32,
    cx: &App,
) -> (&'static str, Hsla) {
    match status {
        // file_diff_stat_add == SUCCESS (green); no dedicated plan-complete token.
        PlanStatus::Completed => ("●", t.file_diff_stat_add),
        PlanStatus::InProgress => ("●", t.status_executing_tool_dark),
        PlanStatus::Pending => (
            "●",
            theme::dim_toward_gray(theme::agent_chat_fg_muted(cx), dim),
        ),
    }
}

/// Content colour for one plan entry.
fn plan_entry_color(status: PlanStatus, dim: f32, cx: &App) -> Hsla {
    match status {
        PlanStatus::Completed => theme::dim_toward_gray(theme::agent_chat_fg_muted(cx), dim),
        PlanStatus::InProgress => theme::dim_toward_gray(theme::agent_chat_fg(cx), dim),
        PlanStatus::Pending => theme::dim_toward_gray(theme::agent_chat_fg(cx), dim),
    }
}

/// Collapsible bottom checklist for the agent's live execution plan.
pub(super) fn plan_region(
    pane_id: PaneId,
    plan: &[PlanEntryView],
    collapsed: bool,
    plan_scroll: &gpui::ScrollHandle,
    t: &theme::DarudaTheme,
    dim: f32,
    cx: &mut Context<AgentChatView>,
) -> Option<AnyElement> {
    if plan.is_empty() {
        return None;
    }
    let (done, total) = plan_progress(plan);
    // All-done reads green (the run finished); otherwise the count stays muted.
    let count_color = if done == total {
        t.file_diff_stat_add
    } else {
        theme::dim_toward_gray(theme::agent_chat_fg_muted(cx), dim)
    };

    // Built inline because this toggles plan state, not a `FoldKey`.
    let chevron: Disclosure = disclosure(
        SharedString::from(format!("agent-chat-plan-chevron-{pane_id}")),
        !collapsed,
    )
    .color(theme::dim_toward_gray(theme::agent_chat_fg_subtle(cx), dim));
    let mut header = div()
        .id(("agent-chat-plan-header", pane_id as usize))
        .w_full()
        .min_w_0()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::AGENT_CHAT_MSG_GAP))
        .px(px(theme::AGENT_CHAT_PAD_X))
        .py(px(theme::AGENT_CHAT_PAD_Y))
        .cursor_pointer()
        .on_click(cx.listener(|this, _ev, _window, cx| this.toggle_plan_collapsed(cx)))
        .child(chevron)
        .child(
            div()
                .flex_none()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(theme::dim_toward_gray(theme::agent_chat_fg(cx), dim))
                .text_size(px(theme::agent_chat_font_size(cx)))
                .child(SharedString::from(s::agent_chat_plan_label())),
        )
        .child(
            div()
                .flex_none()
                .text_color(count_color)
                .text_size(px(theme::agent_chat_font_size(cx)))
                .child(SharedString::from(format!("{done}/{total}"))),
        );

    // Folded plans still hint at the current live step when one exists.
    if collapsed && let Some(active) = plan.iter().find(|e| e.status == PlanStatus::InProgress) {
        header = header
            .child(
                div()
                    .flex_none()
                    .opacity(pulse_opacity(cx))
                    .text_color(t.status_executing_tool_dark)
                    .text_size(px(theme::agent_chat_font_size(cx)))
                    .child(SharedString::from("●")),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_color(theme::dim_toward_gray(theme::agent_chat_fg_subtle(cx), dim))
                    .text_size(px(theme::agent_chat_font_size(cx)))
                    .child(SharedString::from(format!("· {}", active.content))),
            );
    }

    // Once complete, offer an explicit dismiss control; stop propagation so it
    // does not also toggle the header.
    if done == total {
        header = header.child(div().flex_1().min_w_0()).child(
            button_bare(SharedString::from(format!(
                "agent-chat-plan-dismiss-{pane_id}"
            )))
            .ghost()
            .icon(IconName::Close)
            .tooltip(s::agent_chat_plan_dismiss())
            .on_click(cx.listener(|this, _ev, _window, cx| {
                cx.stop_propagation();
                this.dismiss_plan(cx);
            })),
        );
    }

    let mut region = div()
        .flex_none()
        .w_full()
        .flex()
        .flex_col()
        .border_t_1()
        // Use terminal-derived tint, not fixed chrome colours, so custom/light
        // terminal themes keep the plan region legible.
        .border_color(theme::dim_toward_gray(
            theme::agent_chat_border_tint(cx),
            dim,
        ))
        .bg(theme::dim_toward_gray(theme::agent_chat_tint(cx), dim))
        .child(header);

    if !collapsed {
        // Plain `Div`, not `list()`: plans are typically small and this keeps
        // the shared `ScrollHandle` thumb path.
        let mut list = div()
            .id(("agent-chat-plan-list", pane_id as usize))
            .flex()
            .flex_col()
            .gap(px(theme::AGENT_CHAT_MSG_GAP))
            .max_h(px(theme::AGENT_CHAT_PLAN_MAX_H))
            .overflow_y_scroll()
            .track_scroll(plan_scroll)
            .px(px(theme::AGENT_CHAT_PAD_X))
            .pb(px(theme::AGENT_CHAT_PAD_Y));
        for entry in plan {
            let (glyph, glyph_color) = plan_status_glyph(entry.status, t, dim, cx);
            let in_progress = entry.status == PlanStatus::InProgress;
            list = list.child(
                div()
                    .w_full()
                    .min_w_0()
                    .flex()
                    .flex_row()
                    .items_baseline()
                    .gap(px(theme::AGENT_CHAT_MSG_GAP))
                    // In-progress row uses the shared selection tint.
                    .when(in_progress, |row| {
                        row.bg(theme::dim_toward_gray(theme::SELECTION_BG, dim))
                            .rounded_sm()
                    })
                    .child(
                        div()
                            .flex_none()
                            .text_color(glyph_color)
                            .text_size(px(theme::agent_chat_font_size(cx)))
                            // In-progress glyph pulses; settled glyphs stay solid.
                            .when(in_progress, |g| g.opacity(pulse_opacity(cx)))
                            .child(SharedString::from(glyph)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_color(plan_entry_color(entry.status, dim, cx))
                            .text_size(px(theme::agent_chat_font_size(cx)))
                            .child(SharedString::from(entry.content.clone())),
                    ),
            );
        }
        // Thumb geometry follows the `ScrollHandle` convention used by the
        // shared scrollbar helper.
        let viewport_h = plan_scroll.bounds().size.height;
        let content_h = viewport_h + plan_scroll.max_offset().y;
        let thumb = crate::ui::scrollbar::vertical_thumb(
            ("agent-chat-plan-scrollbar", pane_id as usize),
            viewport_h,
            content_h,
            plan_scroll.offset().y,
            px(0.),
            t.scrollbar_thumb,
            t.file_viewer_scrollbar_thumb_hover,
        );
        region = region.child(
            div()
                .relative()
                .flex_none()
                .w_full()
                .child(list)
                .children(thumb),
        );
    }

    Some(region.into_any_element())
}

#[cfg(test)]
mod tests {
    use super::plan_progress;
    use daruda_acp::{PlanEntryView, PlanPriority, PlanStatus};

    fn plan_entry(status: PlanStatus) -> PlanEntryView {
        PlanEntryView {
            content: "step".into(),
            priority: PlanPriority::Medium,
            status,
        }
    }

    #[test]
    fn plan_progress_empty_is_zero_over_zero() {
        assert_eq!(plan_progress(&[]), (0, 0));
    }

    #[test]
    fn plan_progress_counts_only_completed() {
        let plan = [
            plan_entry(PlanStatus::Completed),
            plan_entry(PlanStatus::InProgress),
            plan_entry(PlanStatus::Pending),
            plan_entry(PlanStatus::Completed),
        ];
        assert_eq!(plan_progress(&plan), (2, 4));
    }

    #[test]
    fn plan_progress_all_completed_is_total_over_total() {
        let plan = [
            plan_entry(PlanStatus::Completed),
            plan_entry(PlanStatus::Completed),
            plan_entry(PlanStatus::Completed),
        ];
        assert_eq!(plan_progress(&plan), (3, 3));
    }
}
