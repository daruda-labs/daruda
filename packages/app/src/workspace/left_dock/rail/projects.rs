//! Project tiles and open-worktree rows inside the compact rail.

use super::{lane_indicator, project_initial};
use crate::{
    ui::{self, theme},
    workspace::{
        layout::{Dock, LeftDockSnapshot, ProjectSnapshot},
        root_menu::RootContextMenuExt as _,
    },
};
use daruda_store::project::LaneRef;
use gpui::{AnyElement, Context, IntoElement, div, prelude::*, px};
use std::{cell::Cell, rc::Rc};

pub(super) fn project(
    project: &ProjectSnapshot,
    snap: &LeftDockSnapshot,
    cx: &mut Context<Dock>,
) -> AnyElement {
    let t = theme::current(cx).clone();
    let ws = snap.workspace.clone();
    let id = project.id;
    let target = LaneRef {
        project: id,
        lane: project.last_active_lane_id,
    };
    let trigger_bounds = Rc::new(Cell::new(gpui::Bounds::default()));
    let measured_bounds = trigger_bounds.clone();
    let key_bounds = trigger_bounds.clone();
    let key_ws = ws.clone();
    let active_project = snap.active.project == id;
    let mut block = div().flex().flex_col().w_full().child(
        div()
            .id(("rail-project", id as usize))
            .debug_selector(move || format!("rail-project-{id}"))
            .relative()
            .child(
                gpui::canvas(
                    move |bounds, _, _| measured_bounds.set(bounds),
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0()
                .size_full(),
            )
            .focusable()
            .tab_stop(true)
            .h(px(theme::DOCK_RAIL_ROW_H))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .text_size(px(theme::FONT_SIZE_MD))
            .text_color(t.text_primary)
            .hover(|el| el.bg(t.lane_row_hover_bg))
            .child(
                div()
                    .rounded(px(theme::RADIUS_SM))
                    .bg(t.rail_project_bg)
                    .border_1()
                    .border_color(if active_project {
                        theme::PRIMARY
                    } else {
                        t.border
                    })
                    .size(px(theme::DOCK_RAIL_INITIAL_SIZE))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(theme::CONTROL_ICON_SIZE))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child(project_initial(&project.name)),
            )
            .tooltip(ui::tooltip::text(project.name.clone()))
            .on_key_down(move |event, window, cx| {
                if let Some(ws) = key_ws.upgrade() {
                    ws.update(cx, |ws, cx| {
                        ws.on_rail_key(
                            super::ops::RailItem::Project(id, key_bounds.get()),
                            event,
                            window,
                            cx,
                        )
                    });
                }
            })
            .on_click(move |_, window, cx| {
                if let Some(ws) = ws.upgrade() {
                    ws.update(cx, |ws, cx| {
                        ws.choose_project_from_rail(id, trigger_bounds.get(), window, cx)
                    });
                }
            })
            .root_context_menu(snap.workspace.clone(), {
                let ws = snap.workspace.clone();
                move |menu, _, _| {
                    super::super::projects::project_menu::build_project_menu_items(
                        id,
                        target.lane,
                        ws.clone(),
                    )
                    .into_iter()
                    .fold(menu, |menu, item| menu.item(item))
                }
            }),
    );
    {
        let mut lanes: Vec<_> = project
            .lanes
            .iter()
            .filter(|lane| {
                snap.open_tabs
                    .get(&LaneRef {
                        project: id,
                        lane: lane.id,
                    })
                    .copied()
                    .unwrap_or_default()
                    > 0
            })
            .collect();
        lanes.sort_by_key(|lane| lane.tab_order);
        for lane in lanes {
            let target = LaneRef {
                project: id,
                lane: lane.id,
            };
            let lane_menu = super::super::projects::context_menu::lane_menu(lane, id, snap);
            let count = snap.named_sessions.get(&target).map_or(0, Vec::len);
            let tooltip = format!("{} / {}", project.name, lane.display_name());
            let ws = snap.workspace.clone();
            let active = snap.active == target;
            let trigger_bounds = Rc::new(Cell::new(gpui::Bounds::default()));
            let measured_bounds = trigger_bounds.clone();
            let key_bounds = trigger_bounds.clone();
            let key_ws = ws.clone();
            block = block.child(
                div()
                    .id(gpui::SharedString::from(format!(
                        "rail-lane-{id}-{}",
                        lane.id
                    )))
                    .focusable()
                    .tab_stop(true)
                    .debug_selector(move || format!("rail-lane-{id}-{}", target.lane))
                    .relative()
                    .child(
                        gpui::canvas(
                            move |bounds, _, _| measured_bounds.set(bounds),
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .inset_0()
                        .size_full(),
                    )
                    .h(px(theme::DOCK_RAIL_ROW_H))
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(theme::GAP_XS))
                    .cursor_pointer()
                    .border_l(px(theme::LANE_ACTIVE_BORDER_W))
                    .border_color(if active {
                        theme::PRIMARY
                    } else {
                        theme::TRANSPARENT
                    })
                    .when(active, |el| el.bg(t.lane_card_active_bg))
                    .hover(|el| el.bg(t.lane_row_hover_bg))
                    .child(lane_indicator(
                        snap.agent_status_per_lane.get(&target).copied(),
                        cx,
                    ))
                    .when(count > 1, |el| {
                        el.child(
                            div()
                                .text_size(px(theme::FONT_SIZE_SM))
                                .text_color(t.text_primary)
                                .child(count.to_string()),
                        )
                    })
                    .tooltip(ui::tooltip::text(tooltip))
                    .on_key_down(move |event, window, cx| {
                        if let Some(ws) = key_ws.upgrade() {
                            ws.update(cx, |ws, cx| {
                                ws.on_rail_key(
                                    super::ops::RailItem::Lane(target, key_bounds.get()),
                                    event,
                                    window,
                                    cx,
                                )
                            });
                        }
                    })
                    .on_click(move |_, window, cx| {
                        if let Some(ws) = ws.upgrade() {
                            ws.update(cx, |ws, cx| {
                                ws.choose_lane_from_rail(target, trigger_bounds.get(), window, cx)
                            });
                        }
                    })
                    .root_context_menu(snap.workspace.clone(), lane_menu),
            );
        }
    }
    block.into_any_element()
}

#[cfg(test)]
mod tests {
    // Rail visibility and deployment are exercised by the parent native tests.
}
