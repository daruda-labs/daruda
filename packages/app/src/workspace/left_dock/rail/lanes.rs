use std::{cell::Cell, rc::Rc};

use daruda_store::project::LaneRef;
use gpui::{AnyElement, Context, IntoElement, SharedString, div, prelude::*, px};

use crate::lane::Lane;
use crate::ui::{self, theme};
use crate::workspace::layout::{Dock, LeftDockSnapshot, ProjectSnapshot};
use crate::workspace::root_menu::RootContextMenuExt as _;

use super::{PaneSnapshot, ops::RailItem};

pub(super) fn render(
    lane: &Lane,
    project: &ProjectSnapshot,
    snap: &LeftDockSnapshot,
    cx: &mut Context<Dock>,
) -> AnyElement {
    let target = LaneRef {
        project: project.id,
        lane: lane.id,
    };
    let panes = snap
        .rail_panes
        .get(&target)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let tooltip = format!("{} · {}", project.name, lane.display_name());
    let mut block = div()
        .id(SharedString::from(format!(
            "rail-lane-{}-{}",
            target.project, target.lane
        )))
        .debug_selector(move || format!("rail-lane-{}-{}", target.project, target.lane))
        .flex()
        .flex_col();
    if project.rail_grouped_lanes.contains(&lane.id) || panes.is_empty() {
        block = block.child(grouped_row(lane, target, panes.len(), tooltip, snap, cx));
    } else {
        for pane in panes {
            block = block.child(pane_row(lane, target, pane, &tooltip, snap, cx));
        }
    }
    block.into_any_element()
}

fn grouped_row(
    lane: &Lane,
    target: LaneRef,
    count: usize,
    tooltip: String,
    snap: &LeftDockSnapshot,
    cx: &mut Context<Dock>,
) -> AnyElement {
    let bounds = Rc::new(Cell::new(gpui::Bounds::default()));
    let measured = bounds.clone();
    let keyboard_bounds = bounds.clone();
    let workspace = snap.workspace.clone();
    let keyboard_workspace = workspace.clone();
    let theme = theme::current(cx);
    let active = snap.active == target && snap.workspace_page.is_none();
    div()
        .id(SharedString::from(format!(
            "rail-lane-trigger-{}-{}",
            target.project, target.lane
        )))
        .debug_selector(move || format!("rail-lane-trigger-{}-{}", target.project, target.lane))
        .focusable()
        .tab_stop(true)
        .relative()
        .child(
            gpui::canvas(move |value, _, _| measured.set(value), |_, _, _, _| {})
                .absolute()
                .inset_0()
                .size_full(),
        )
        .h(px(snap.metrics.row(theme::DOCK_RAIL_PANE_ROW_H)))
        .flex()
        .items_center()
        .justify_center()
        .gap(px(theme::GAP_XS))
        .border_l(px(theme::LANE_ACTIVE_BORDER_W))
        .border_color(if active {
            theme::PRIMARY
        } else {
            theme::TRANSPARENT
        })
        .when(active, |element| element.bg(theme.lane_card_active_bg))
        .cursor_pointer()
        .when(!active, |element| {
            element.hover(|element| element.bg(theme.lane_row_hover_bg))
        })
        .child(if count == 0 {
            super::empty_lane_indicator(
                snap.metrics,
                snap.metrics.icon(theme::STATUS_INDICATOR_CELL_WIDTH),
                cx,
            )
        } else {
            super::lane_indicator(
                snap.agent_status_per_lane.get(&target).copied(),
                snap.agent_outcome_per_lane.get(&target).copied(),
                snap.metrics,
                cx,
            )
        })
        .when(count > 1, |element| {
            element.child(
                div()
                    .text_size(px(snap.metrics.text(theme::FONT_SIZE_SM)))
                    .text_color(theme.text_primary)
                    .child(count.to_string()),
            )
        })
        .tooltip(ui::tooltip::text(tooltip))
        .on_key_down(move |event, window, cx| {
            if let Some(workspace) = keyboard_workspace.upgrade() {
                workspace.update(cx, |workspace, cx| {
                    workspace.on_rail_key(
                        RailItem::Lane(target, keyboard_bounds.get()),
                        event,
                        window,
                        cx,
                    )
                });
            }
        })
        .on_click(move |_, window, cx| {
            if let Some(workspace) = workspace.upgrade() {
                workspace.update(cx, |workspace, cx| {
                    workspace.choose_lane_from_rail(target, bounds.get(), window, cx)
                });
            }
        })
        .root_context_menu(
            snap.workspace.clone(),
            super::super::projects::context_menu::lane_menu(lane, target.project, snap),
        )
        .into_any_element()
}

fn pane_row(
    lane: &Lane,
    target: LaneRef,
    pane: &PaneSnapshot,
    lane_label: &str,
    snap: &LeftDockSnapshot,
    cx: &mut Context<Dock>,
) -> AnyElement {
    let active =
        snap.active == target && snap.focused_pane == pane.pane && snap.workspace_page.is_none();
    let id = pane.pane;
    let workspace = snap.workspace.clone();
    let keyboard_workspace = workspace.clone();
    let theme = theme::current(cx);
    div()
        .id(("rail-pane", id as usize))
        .debug_selector(move || format!("rail-pane-{id}"))
        .focusable()
        .tab_stop(true)
        .h(px(snap.metrics.row(theme::DOCK_RAIL_PANE_ROW_H)))
        .flex()
        .items_center()
        .justify_center()
        .gap(px(theme::GAP_XS))
        .border_l(px(theme::LANE_ACTIVE_BORDER_W))
        .border_color(if active {
            theme::PRIMARY
        } else {
            theme::TRANSPARENT
        })
        .when(active, |element| element.bg(theme.lane_card_active_bg))
        .when(!active, |element| {
            element.hover(|element| element.bg(theme.lane_row_hover_bg))
        })
        .cursor_pointer()
        .child(super::pane_indicator(pane, snap.metrics, cx))
        .tooltip(ui::tooltip::with_description(
            pane.title.clone(),
            lane_label.to_owned(),
        ))
        .on_key_down(move |event, window, cx| {
            if let Some(workspace) = keyboard_workspace.upgrade() {
                workspace.update(cx, |workspace, cx| {
                    workspace.on_rail_key(RailItem::Pane(target, id), event, window, cx)
                });
            }
        })
        .on_click(move |_, window, cx| {
            if let Some(workspace) = workspace.upgrade() {
                workspace.update(cx, |workspace, cx| {
                    workspace.activate_rail_pane(target, id, window, cx)
                });
            }
        })
        .root_context_menu(
            snap.workspace.clone(),
            super::super::projects::context_menu::lane_menu(lane, target.project, snap),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {}
