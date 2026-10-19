use gpui::{AnyElement, Context, IntoElement, MouseButton, div, prelude::*, px};

use crate::ui::{self, theme};
use crate::workspace::layout::{Dock, LeftDockSnapshot, ProjectSnapshot};
use crate::workspace::root_menu::RootContextMenuExt as _;

pub(super) fn project(
    project: &ProjectSnapshot,
    snap: &LeftDockSnapshot,
    cx: &mut Context<Dock>,
) -> AnyElement {
    let theme = theme::current(cx);
    let id = project.id;
    let workspace = snap.workspace.clone();
    let keyboard_workspace = workspace.clone();
    let menu_workspace = workspace.clone();
    let last_lane = project.last_active_lane_id;
    let mut block = div().flex().flex_col().w_full().child(
        div()
            .id(("rail-project", id as usize))
            .debug_selector(move || format!("rail-project-{id}"))
            .focusable()
            .tab_stop(true)
            .h(px(snap.metrics.row(theme::DOCK_RAIL_ROW_H)))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .hover(|element| element.bg(theme.lane_row_hover_bg))
            .child(
                div()
                    .rounded(px(theme::RADIUS_SM))
                    .when(!project.is_collapsed, |element| {
                        element.bg(theme.rail_project_bg)
                    })
                    .border_1()
                    .border_color(theme.border.opacity(theme::DOCK_RAIL_INITIAL_BORDER_ALPHA))
                    .size(px(snap.metrics.row(theme::DOCK_RAIL_INITIAL_SIZE)))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(snap.metrics.text(theme::FONT_SIZE_MD)))
                    .text_color(theme.text_primary)
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child(super::project_badge_label(
                        &project.name,
                        project.display_alias.as_deref(),
                    )),
            )
            .tooltip(ui::tooltip::text(project.name.clone()))
            .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
            .on_key_down(move |event, window, cx| {
                if let Some(workspace) = keyboard_workspace.upgrade() {
                    workspace.update(cx, |workspace, cx| {
                        workspace.on_rail_key(super::ops::RailItem::Project(id), event, window, cx)
                    });
                }
            })
            .on_click(move |_, _, cx| {
                if let Some(workspace) = workspace.upgrade() {
                    workspace.update(cx, |workspace, cx| {
                        workspace.toggle_project_collapse(id, cx)
                    });
                }
            })
            .root_context_menu(snap.workspace.clone(), move |menu, _, _| {
                super::super::projects::project_menu::build_project_menu_items(
                    id,
                    last_lane,
                    menu_workspace.clone(),
                )
                .into_iter()
                .fold(menu, |menu, item| menu.item(item))
            }),
    );
    if !project.is_collapsed {
        let mut lanes: Vec<_> = project.lanes.iter().collect();
        lanes.sort_by_key(|lane| lane.tab_order);
        for lane in lanes {
            block = block.child(super::lanes::render(lane, project, snap, cx));
        }
    }
    block.into_any_element()
}

#[cfg(test)]
mod tests {}
