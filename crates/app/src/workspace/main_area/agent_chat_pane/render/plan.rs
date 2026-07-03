//! The agent's live execution plan: progress count, per-entry colours, and the
//! collapsible bottom plan region.

use daruda_acp::{PlanEntryView, PlanStatus};
use gpui::{AnyElement, App, Hsla, IntoElement, SharedString, div, prelude::*, px};

use super::pulse_opacity;
use crate::surface::strings as s;
use crate::ui::theme;
use crate::ui::{Disclosure, disclosure};
use crate::workspace::main_area::agent_chat_pane::view::AgentChatView;
use crate::workspace::main_area::pane_tree::PaneId;

/// `(completed, total)` over the plan entries.  Returns `(0, 0)` for an empty
/// plan — callers must guard on `plan.is_empty()` before using the ratio for
/// colour logic (0 == 0 would otherwise read as "all done").
fn plan_progress(plan: &[PlanEntryView]) -> (usize, usize) {
    let done = plan
        .iter()
        .filter(|e| e.status == PlanStatus::Completed)
        .count();
    (done, plan.len())
}

/// The status glyph + its colour for one plan entry: ● done (green),
/// ● in progress (running amber), ● pending (muted). All three states share
/// the filled ● glyph and are distinguished by colour alone. Mirrors
/// [`super::rollup_glyph`]'s `(glyph, color)` shape; the completed green reuses
/// the diff-stat add colour (there is no dedicated `success` theme token).
fn plan_status_glyph(status: PlanStatus, t: &theme::DarudaTheme, cx: &App) -> (&'static str, Hsla) {
    match status {
        // file_diff_stat_add == SUCCESS (green); no dedicated plan-complete token.
        PlanStatus::Completed => ("●", t.file_diff_stat_add),
        PlanStatus::InProgress => ("●", t.status_executing_tool_dark),
        PlanStatus::Pending => ("●", theme::agent_chat_fg_muted(cx)),
    }
}

/// The content text colour for one plan entry: completed dims (muted, no
/// strikethrough), in-progress emphasizes (full foreground), pending stays
/// foreground. All derive from the terminal-mirrored agent-chat foreground.
fn plan_entry_color(status: PlanStatus, cx: &App) -> Hsla {
    match status {
        PlanStatus::Completed => theme::agent_chat_fg_muted(cx),
        PlanStatus::InProgress => theme::agent_chat_fg(cx),
        PlanStatus::Pending => theme::agent_chat_fg(cx),
    }
}

/// Dedicated bottom region rendering the agent's live execution plan as a
/// collapsible checklist. `None` (no element, no space) when the plan is empty.
/// A top hairline separates it from the conversation above (the region sits
/// below the body). The header — a disclosure row mirroring the section bars —
/// shows a chevron, the "Plan" label, and the `{done}/{total}` count; collapsed
/// it also trails the current in-progress entry's content. Clicking the header
/// toggles [`AgentChatView::toggle_plan_collapsed`]. Expanded, the checklist
/// lists each entry (status glyph + content) and scrolls internally past
/// `AGENT_CHAT_PLAN_MAX_H`.
pub(super) fn plan_region(
    pane_id: PaneId,
    plan: &[PlanEntryView],
    collapsed: bool,
    plan_scroll: &gpui::ScrollHandle,
    t: &theme::DarudaTheme,
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
        theme::agent_chat_fg_muted(cx)
    };

    // Header: same chevron + clickable-row scaffold as the section bars, but the
    // click routes to the plan toggle (not a `FoldKey`), so it's built inline
    // rather than via `disclosure_row`. The chevron carries no `on_toggle` — it's
    // a pure indicator; the whole row is the click target.
    let chevron: Disclosure = disclosure(
        SharedString::from(format!("agent-chat-plan-chevron-{pane_id}")),
        !collapsed,
    )
    .color(theme::agent_chat_fg_subtle(cx));
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
                .text_color(theme::agent_chat_fg(cx))
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

    // Collapsed: trail the current in-progress entry's content (muted,
    // ellipsized) after a `·` separator, so a folded plan still hints at the
    // live step. A pulsing ● glyph precedes the separator so a folded plan
    // still shows the live pulse. Nothing trails when no entry is in progress.
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
                    .text_color(theme::agent_chat_fg_subtle(cx))
                    .text_size(px(theme::agent_chat_font_size(cx)))
                    .child(SharedString::from(format!("· {}", active.content))),
            );
    }

    let mut region = div()
        .flex_none()
        .w_full()
        .flex()
        .flex_col()
        .border_t_1()
        // Background-derived tint + hairline (not the fixed UI `dock_bg` /
        // `border`): the plan region sits on the terminal-mirrored pane, so a
        // fixed dark surface would clash with a light/custom terminal theme —
        // and its terminal-foreground text would render dark-on-dark there. The
        // translucent lift steps it one above the pane on any theme, matching
        // the tool cards.
        .border_color(theme::agent_chat_border_tint(cx))
        .bg(theme::agent_chat_tint(cx))
        .child(header);

    if !collapsed {
        // The plan list is a plain `Div` (not `list()`) — no virtualisation, which
        // is fine for a typically small dataset (< 50 entries). It still gets the
        // project's 4px daruda thumb via the `Div`/`ScrollHandle` variant
        // (`vertical_thumb`), tracked through `plan_scroll`, matching the file
        // viewer and conversation. If 100+ entries become common, migrate to
        // `list()` to bound full-tree repaint cost.
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
            let (glyph, glyph_color) = plan_status_glyph(entry.status, t, cx);
            let in_progress = entry.status == PlanStatus::InProgress;
            list = list.child(
                div()
                    .w_full()
                    .min_w_0()
                    .flex()
                    .flex_row()
                    .items_baseline()
                    .gap(px(theme::AGENT_CHAT_MSG_GAP))
                    // In-progress row: accent-28% tint + slight rounding so the
                    // highlight reads as a bar (mirrors the selection token used
                    // by the completion menu and text selection).
                    .when(in_progress, |row| row.bg(theme::SELECTION_BG).rounded_sm())
                    .child(
                        div()
                            .flex_none()
                            .text_color(glyph_color)
                            .text_size(px(theme::agent_chat_font_size(cx)))
                            // In-progress glyph pulses in lockstep with the section
                            // header rollup dot; settled glyphs stay solid.
                            .when(in_progress, |g| g.opacity(pulse_opacity(cx)))
                            .child(SharedString::from(glyph)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_color(plan_entry_color(entry.status, cx))
                            .text_size(px(theme::agent_chat_font_size(cx)))
                            .child(SharedString::from(entry.content.clone())),
                    ),
            );
        }
        // The thumb sources its geometry from `plan_scroll`: the viewport height
        // from the scroll container's bounds, the total content height as
        // `viewport_h + max_offset().y` (the `ScrollHandle` convention noted in
        // `crate::ui::scrollbar::vertical_thumb`), and the offset from
        // `offset().y`. `top_offset` is `px(0.)` — the list starts at the
        // relative wrapper's origin. The wrapper is `.relative()` so the absolute
        // thumb (`.right(...)`) positions against it.
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
