//! Expanded-sidebar pane summaries and shallow, single-line pane rows.

use daruda_store::project::LaneRef;
use gpui::{AnyElement, App, IntoElement, SharedString, div, prelude::*, px};

use super::RowTarget;
use crate::ui::{IndicatorSize, Sizable as _, theme};
use crate::workspace::{layout::LeftDockSnapshot, left_dock::rail::PaneSnapshot};

pub(in crate::workspace) fn lane_panes(
    lane: LaneRef,
    snap: &LeftDockSnapshot,
    cx: &App,
) -> AnyElement {
    let panes = snap
        .rail_panes
        .get(&lane)
        .map(Vec::as_slice)
        .unwrap_or_default();
    if snap.individual_pane_lanes.contains(&lane) {
        div()
            .flex()
            .flex_col()
            .pl(px(snap.metrics.space(theme::PAD_XS)))
            .children(
                panes
                    .iter()
                    .map(|pane| individual_row(lane, pane, snap, cx)),
            )
            .into_any_element()
    } else if panes.len() > 1 {
        badges(lane, panes, snap, cx)
    } else {
        div().into_any_element()
    }
}

/// Named sessions enter through their session identity; other panes by id.
fn pane_target(
    lane: LaneRef,
    pane: &PaneSnapshot,
    snap: &LeftDockSnapshot,
) -> (RowTarget, SharedString) {
    let target = snap
        .named_sessions
        .get(&lane)
        .and_then(|sessions| sessions.iter().find(|session| session.pane == pane.pane))
        .map(|session| RowTarget::Session(lane, session.clone()))
        .unwrap_or_else(|| RowTarget::Pane(lane, pane.clone()));
    let title = match &target {
        RowTarget::Session(_, session) => session.title.clone(),
        RowTarget::Pane(_, pane) => pane.title.clone(),
    };
    (target, title)
}

fn individual_row(
    lane: LaneRef,
    pane: &PaneSnapshot,
    snap: &LeftDockSnapshot,
    cx: &App,
) -> impl IntoElement + use<> {
    let metrics = snap.metrics;
    let t = theme::current(cx);
    let current = snap.active == lane && snap.focused_pane == pane.pane;
    let (target, title) = pane_target(lane, pane, snap);
    let workspace = snap.workspace.clone();
    let pane_id = pane.pane;
    div()
        .id(SharedString::from(format!(
            "dock-pane-row-{}-{}-{pane_id}",
            lane.project, lane.lane
        )))
        .debug_selector(move || format!("dock-pane-row-{}-{}-{pane_id}", lane.project, lane.lane))
        .flex()
        .items_center()
        .min_w_0()
        .min_h(px(metrics.row(theme::DOCK_TREE_ROW_HEIGHT)))
        .gap(px(metrics.space(theme::GAP_STANDARD)))
        .rounded(px(theme::RADIUS_SM))
        .text_size(px(metrics.text(theme::FONT_SIZE_SM)))
        .text_color(t.text_body)
        .hover(|style| style.bg(t.lane_row_hover_bg))
        .cursor_pointer()
        .child(super::super::rail::pane_indicator(pane, metrics, cx))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_ellipsis()
                .child(title.clone()),
        )
        .when(current, |row| {
            row.child(
                crate::ui::icons::icon(crate::ui::icons::CHECK)
                    .with_size(px(metrics.icon(theme::CONTROL_ICON_SIZE)))
                    .text_color(t.text_primary),
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

/// Expanded grouped panes: each badge enters its own pane directly. The
/// pane chooser is reserved for grouped worktrees in the collapsed rail.
fn badges(lane: LaneRef, panes: &[PaneSnapshot], snap: &LeftDockSnapshot, cx: &App) -> AnyElement {
    let metrics = snap.metrics;
    let t = theme::current(cx);
    div()
        .id(SharedString::from(format!(
            "dock-pane-badges-{}-{}",
            lane.project, lane.lane
        )))
        .debug_selector(move || format!("dock-pane-badges-{}-{}", lane.project, lane.lane))
        .flex()
        .flex_wrap()
        .items_center()
        .min_h(px(metrics.target(theme::CONTROL_TARGET_SIZE)))
        .gap(px(metrics.space(theme::STATUS_BADGES_ROW_GAP)))
        .pl(px(metrics.space(theme::PAD_XS)))
        .text_size(px(metrics.text(theme::STATUS_BADGES_LABEL_FONT_SIZE)))
        .text_color(t.text_muted)
        .child(crate::surface::strings::dock::pane_count(panes.len()))
        .children(panes.iter().map(|pane| badge(lane, pane, snap, cx)))
        .into_any_element()
}

fn badge(lane: LaneRef, pane: &PaneSnapshot, snap: &LeftDockSnapshot, cx: &App) -> AnyElement {
    let metrics = snap.metrics;
    let t = theme::current(cx);
    let current = snap.active == lane && snap.focused_pane == pane.pane;
    let badge = super::super::projects::agent_badges::resolved_badge(
        pane.status,
        pane.outcome,
        IndicatorSize::Badge,
        cx,
    );
    let badge = badge.or_else(|| {
        (pane.neutral_icon == crate::ui::icons::SESSION)
            .then(|| crate::ui::AgentStatusBadge::disconnected(IndicatorSize::Badge, cx))
    });
    let indicator = if let Some(status_badge) = badge {
        let status_badge = status_badge.with_metrics(metrics);
        if current {
            status_badge.active()
        } else {
            status_badge
        }
        .into_any_element()
    } else {
        crate::ui::icons::icon(pane.neutral_icon)
            .with_size(px(metrics.icon(theme::STATUS_INDICATOR_BADGE_SIZE)))
            .text_color(if current {
                t.text_primary
            } else {
                t.text_muted
            })
            .into_any_element()
    };
    let (target, title) = pane_target(lane, pane, snap);
    let key_target = target.clone();
    let workspace = snap.workspace.clone();
    let key_workspace = workspace.clone();
    let pane_id = pane.pane;
    div()
        .id(SharedString::from(format!(
            "dock-pane-badge-{}-{}-{pane_id}",
            lane.project, lane.lane
        )))
        .debug_selector(move || format!("dock-pane-badge-{}-{}-{pane_id}", lane.project, lane.lane))
        .focusable()
        .tab_stop(true)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(theme::RADIUS_SM))
        .hover(|style| style.bg(t.lane_row_hover_bg))
        .cursor_pointer()
        .child(indicator)
        .tooltip(crate::ui::tooltip::text(title))
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            if let Some(ws) = workspace.upgrade() {
                ws.update(cx, |ws, cx| ws.activate_dock_row(&target, window, cx));
            }
        })
        .on_key_down(move |event, window, cx| {
            if let Some(ws) = key_workspace.upgrade() {
                ws.update(cx, |ws, cx| {
                    ws.on_pane_badge_key(&key_target, event, window, cx)
                });
            }
        })
        .into_any_element()
}

#[cfg(test)]
mod tests {
    // Pointer, grouping and selection checks use the sibling Workspace tests.
}
