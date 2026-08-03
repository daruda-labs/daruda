//! The agent's live execution plan: progress count, per-entry colours, and the
//! collapsible bottom plan region.

use daruda_acp::{PlanEntryView, PlanStatus};
use gpui::{AnyElement, App, Hsla, IntoElement, SharedString, div, prelude::*, px};

use super::fold_header::{FoldHeader, FoldRow, FoldToggle, SummaryLine};
use super::pulse_opacity;
use crate::surface::strings as s;
use crate::ui::theme;
use crate::ui::{ButtonVariants as _, IconName, button_bare};
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

    // The live step, previewed only while folded — the stretch slot is
    // collapsed-only by construction, so no manual gate is needed. No `·` prefix:
    // the stretch slot's own gap separates it from the label, same as every other
    // header's preview.
    let active_step = plan
        .iter()
        .find(|e| e.status == PlanStatus::InProgress)
        .map(|e| e.content.clone());
    let mut header = FoldHeader::with_summary({
        let active_step = active_step.clone();
        move || active_step.map(SummaryLine::plain)
    })
    .leading(
        div()
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(theme::AGENT_CHAT_MSG_GAP))
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
            )
            .into_any_element(),
    );

    // The live-step dot is this header's status badge, so it sits right-anchored
    // in the trailing slot like every other header's rollup glyph — and, like
    // them, it reads the same in both fold states.
    if active_step.is_some() {
        header = header.trailing(
            div()
                .flex_none()
                .opacity(pulse_opacity(cx))
                .text_color(t.status_executing_tool_dark)
                .text_size(px(theme::agent_chat_font_size(cx)))
                .child(SharedString::from("●"))
                .into_any_element(),
        );
    }

    // Once complete, offer an explicit dismiss control; stop propagation so it
    // does not also toggle the header.
    if done == total {
        header = header.trailing(
            button_bare(SharedString::from(format!(
                "agent-chat-plan-dismiss-{pane_id}"
            )))
            .ghost()
            .icon(IconName::Close)
            .tooltip(s::agent_chat_plan_dismiss())
            .on_click(cx.listener(|this, _ev, _window, cx| {
                cx.stop_propagation();
                this.dismiss_plan(cx);
            }))
            .into_any_element(),
        );
    }

    // The plan collapses its own view flag rather than a `FoldKey`, so it is the
    // one `FoldToggle::external` header — the rest of the assembly (slot geometry,
    // chevron, collapsed-only preview) is the shared one.
    let block = FoldRow::block(
        SharedString::from(format!("agent-chat-plan-{pane_id}")),
        FoldToggle::external(|view, cx| view.toggle_plan_collapsed(cx)),
        !collapsed,
        header,
        |cx| plan_list(pane_id, plan, plan_scroll, t, dim, cx),
    )
    .chrome(|row| {
        row.px(px(theme::AGENT_CHAT_PAD_X))
            .py(px(theme::AGENT_CHAT_PAD_Y))
    })
    .render(dim, cx);

    Some(
        div()
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
            .child(block)
            .into_any_element(),
    )
}

/// The expanded plan body: the scrollable checklist plus its thumb.
fn plan_list(
    pane_id: PaneId,
    plan: &[PlanEntryView],
    plan_scroll: &gpui::ScrollHandle,
    t: &theme::DarudaTheme,
    dim: f32,
    cx: &mut Context<AgentChatView>,
) -> AnyElement {
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
    div()
        .relative()
        .flex_none()
        .w_full()
        .child(list)
        .children(thumb)
        .into_any_element()
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
