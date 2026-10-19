//! Named session rows and a chooser deployed above the dock's scroll clip.

use super::{
    RowTarget,
    entries::{Entry, Target},
};
use crate::ui::Sizable as _;
use crate::{
    surface::strings as s,
    ui::{AgentStatusBadge, IndicatorSize, theme},
    workspace::{Workspace, layout::LeftDockSnapshot},
};
use daruda_store::project::LaneRef;
use gpui::{
    AnyElement, App, Context, IntoElement, WeakEntity, Window, anchored, deferred, div, prelude::*,
    px,
};

fn status_label(status: daruda_agent::SessionStatus) -> String {
    use daruda_agent::SessionStatus;
    match status {
        SessionStatus::Working => s::tab_strip::status_working(),
        SessionStatus::ExecutingTool => s::dock::session_executing(),
        SessionStatus::NeedsAttention => s::tab_strip::status_attention(),
        SessionStatus::Failed => s::tab_strip::status_failed(),
        SessionStatus::Idle => s::dock::session_idle(),
        SessionStatus::Connecting => s::dock::session_connecting(),
    }
}

fn session_row(
    target: RowTarget,
    current: bool,
    selected: bool,
    workspace: WeakEntity<Workspace>,
    metrics: theme::UiMetrics,
    cx: &App,
) -> impl IntoElement + use<> {
    let t = theme::current(cx);
    let (pane, title, status) = match &target {
        RowTarget::Session(_, row) => (row.pane, row.title.clone(), row.status),
        RowTarget::Pane(_, row) => (
            row.pane,
            row.title.clone(),
            row.status.unwrap_or(daruda_agent::SessionStatus::Idle),
        ),
    };
    div()
        .id(("dock-session", pane as usize))
        .flex()
        .items_center()
        .gap(px(theme::GAP_LG))
        .min_h(px(metrics.row(theme::DOCK_SESSION_ROW_H)))
        .px(px(theme::PAD_STANDARD))
        .border_l(px(theme::LANE_ACTIVE_BORDER_W))
        .border_color(if selected {
            theme::PRIMARY
        } else {
            theme::TRANSPARENT
        })
        .bg(if selected || current {
            t.lane_card_active_bg
        } else {
            theme::TRANSPARENT
        })
        .hover(|style| style.bg(t.lane_row_hover_bg))
        .cursor_pointer()
        .child(
            AgentStatusBadge::for_status(status, IndicatorSize::Leading, cx).with_metrics(metrics),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(
                    div()
                        .text_size(px(metrics.text(theme::FONT_SIZE_MD)))
                        .text_color(t.text_primary)
                        .text_ellipsis()
                        .child(title.clone()),
                )
                .child(
                    div()
                        .text_size(px(metrics.text(theme::FONT_SIZE_SM)))
                        .text_color(t.text_muted)
                        .child(status_label(status)),
                ),
        )
        .when(current, |el| {
            el.child(
                crate::ui::icons::icon(crate::ui::icons::CHECK)
                    .with_size(px(metrics.icon(theme::CONTROL_ICON_SIZE))),
            )
        })
        .tooltip(crate::ui::tooltip::text(title))
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            if let Some(ws) = workspace.upgrade() {
                ws.update(cx, |ws, cx| ws.activate_dock_row(&target, window, cx));
            }
        })
}

pub(in crate::workspace) fn named_rows(
    lane: LaneRef,
    snap: &LeftDockSnapshot,
    cx: &App,
) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .children(
            snap.named_sessions
                .get(&lane)
                .into_iter()
                .flatten()
                .map(|row| {
                    session_row(
                        RowTarget::Session(lane, row.clone()),
                        snap.active == lane
                            && snap.agent_active_session_id.as_deref() == Some(&row.id),
                        false,
                        snap.workspace.clone(),
                        snap.metrics,
                        cx,
                    )
                }),
        )
        .into_any_element()
}

pub(in crate::workspace) fn chooser_overlay(
    ws: &Workspace,
    window: &Window,
    cx: &mut Context<Workspace>,
) -> AnyElement {
    let Some(chooser) = &ws.docks.session_chooser else {
        return div().into_any_element();
    };
    let metrics = theme::UiMetrics(ws.mirrors.left_dock_size);
    let rows = ws.rail_chooser_entries(chooser.target, cx);
    let t = theme::current(cx);
    let (title, subtitle) = match chooser.target {
        Target::Panes { lane, .. } => (ws.lane_label_for(lane), s::dock::pane_count(rows.len())),
        Target::Worktrees { project, .. } => (
            ws.project_for(project)
                .map(|p| p.name.clone())
                .unwrap_or_default(),
            s::dock::worktree_count(rows.len()),
        ),
    };
    let weak = cx.entity().downgrade();
    let panel = crate::ui::popover_panel(
        window,
        crate::ui::PanelLimits {
            width: theme::DOCK_SESSION_PANEL_W,
            max_height: theme::DOCK_SESSION_PANEL_H,
            min_height: 0.0,
            margin: theme::POPOVER_WINDOW_MARGIN,
            chrome: theme::INPUT_BORDER_WIDTH * 2.0,
        },
        None,
    )
    .id("dock-session-chooser")
    .occlude()
    .track_focus(&chooser.focus)
    .bg(t.float_panel_bg)
    .text_color(t.text_primary)
    .border_1()
    .border_color(t.border)
    .rounded(px(theme::RADIUS_LG))
    .shadow_lg()
    .on_key_down(cx.listener(Workspace::on_session_chooser_key))
    .on_mouse_down_out(cx.listener(Workspace::on_session_chooser_mouse_down_out))
    .child(
        div()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(theme::GAP_LG))
            .p(px(theme::PAD_STANDARD))
            .border_b_1()
            .border_color(t.border)
            .when_some(
                match chooser.target {
                    Target::Panes { lane, .. } => Some(lane.project),
                    _ => None,
                },
                |header, project| {
                    let bounds = chooser.trigger_bounds;
                    header.child(
                        crate::ui::button_icon_sized(
                            "dock-worktrees-back",
                            crate::ui::icons::BACK,
                            metrics,
                            cx,
                        )
                        .tooltip(s::dock::sidebar_tab_worktrees())
                        .debug_selector(|| "dock-worktrees-back".into())
                        .on_click(cx.listener(move |ws, _, window, cx| {
                            ws.choose_project_from_rail(project, bounds, window, cx)
                        })),
                    )
                },
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .text_size(px(metrics.text(theme::FONT_SIZE_MD)))
                            .text_color(t.text_primary)
                            .text_ellipsis()
                            .child(title),
                    )
                    .child(
                        div()
                            .text_size(px(metrics.text(theme::FONT_SIZE_SM)))
                            .text_color(t.text_muted)
                            .child(subtitle),
                    ),
            )
            .child(crate::ui::button_close("close-dock-sessions", cx).on_click(
                cx.listener(|ws, _, window, cx| ws.close_session_chooser(true, window, cx)),
            )),
    )
    .child(
        div()
            .id("dock-session-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&chooser.scroll)
            .children(rows.iter().map(|entry| {
                match entry {
                    Entry::Pane { lane, row } => session_row(
                        RowTarget::Pane(*lane, row.clone()),
                        ws.active == *lane && ws.active_runtime().focused_pane_id == row.pane,
                        chooser.target.is_selected(entry),
                        weak.clone(),
                        metrics,
                        cx,
                    )
                    .into_any_element(),
                    Entry::Worktree {
                        lane,
                        title,
                        status,
                        outcome,
                        open_tabs,
                    } => {
                        let selected = chooser.target.is_selected(entry);
                        let entry = entry.clone();
                        let weak = weak.clone();
                        div()
                            .id(("dock-worktree", lane.lane as usize))
                            .debug_selector({
                                let lane = *lane;
                                move || format!("dock-worktree-{}-{}", lane.project, lane.lane)
                            })
                            .flex()
                            .items_center()
                            .gap(px(theme::GAP_LG))
                            .min_h(px(metrics.row(theme::DOCK_SESSION_ROW_H)))
                            .px(px(theme::PAD_STANDARD))
                            .border_l(px(theme::LANE_ACTIVE_BORDER_W))
                            .border_color(if selected {
                                theme::PRIMARY
                            } else {
                                theme::TRANSPARENT
                            })
                            .when(selected, |el| el.bg(t.lane_card_active_bg))
                            .hover(|el| el.bg(t.lane_row_hover_bg))
                            .cursor_pointer()
                            .child(super::super::rail::lane_indicator(
                                *status, *outcome, metrics, cx,
                            ))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .child(
                                        div()
                                            .text_size(px(metrics.text(theme::FONT_SIZE_MD)))
                                            .text_ellipsis()
                                            .child(title.clone()),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(metrics.text(theme::FONT_SIZE_SM)))
                                            .text_color(t.text_muted)
                                            .child(if *open_tabs == 0 {
                                                s::dock::no_open_tabs()
                                            } else {
                                                s::dock::open_tab_count(open_tabs)
                                            }),
                                    ),
                            )
                            .child(
                                crate::ui::icons::icon(crate::ui::icons::CHEVRON_RIGHT)
                                    .with_size(px(metrics.icon(theme::CONTROL_ICON_SIZE))),
                            )
                            .tooltip(crate::ui::tooltip::text(title.clone()))
                            .on_click(move |_, window, cx| {
                                if let Some(ws) = weak.upgrade() {
                                    ws.update(cx, |ws, cx| {
                                        ws.activate_rail_choice(&entry, window, cx)
                                    });
                                }
                            })
                            .into_any_element()
                    }
                }
            })),
    );
    deferred(
        anchored()
            .position(gpui::point(
                chooser.trigger_bounds.right() + px(theme::GAP_LG),
                chooser.trigger_bounds.top(),
            ))
            .snap_to_window_with_margin(px(theme::POPOVER_WINDOW_MARGIN))
            .child(panel),
    )
    .with_priority(1)
    .into_any_element()
}

#[cfg(test)]
mod tests {
    // Native chooser behavior is covered by the sibling sessions::tests module.
}
