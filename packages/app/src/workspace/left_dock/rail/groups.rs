use gpui::{AnyElement, Context, IntoElement, MouseButton, div, prelude::*, px};

use crate::ui::{self, theme};
use crate::workspace::layout::{Dock, GroupSnapshot, LeftDockSnapshot};
use crate::workspace::root_menu::RootContextMenuExt as _;
use crate::workspace::tab_indicator::TabIndicator;

pub(super) fn header(
    group: &GroupSnapshot,
    snap: &LeftDockSnapshot,
    cx: &mut Context<Dock>,
) -> AnyElement {
    let theme = theme::current(cx);
    let background = theme.dock_bg.blend(theme.overlay_prominent);
    let hover_background = background.blend(theme.overlay_hover);
    let id = group.id;
    let workspace = snap.workspace.clone();
    let keyboard_workspace = workspace.clone();
    let state = active_indicator(group, snap);
    let tooltip = state.map_or_else(
        || group.name.to_string(),
        |state| crate::surface::strings::tab_strip::tooltip_with_status(&group.name, state.label()),
    );
    let color = state
        .map(|state| match state {
            TabIndicator::Attention => theme::WARNING,
            TabIndicator::Failed | TabIndicator::Errored => theme.status_failed_dark,
            TabIndicator::Working => theme.status_working_dark,
            TabIndicator::Done => theme::SUCCESS,
        })
        .unwrap_or(theme.text_muted);
    let name = group.name.clone();
    let collapsed = group.is_collapsed;
    div()
        .mt(px(theme::GAP_LG))
        .child(
            div()
                .id(("rail-group", id as usize))
                .debug_selector(move || format!("rail-group-{id}"))
                .focusable()
                .tab_stop(true)
                .cursor_pointer()
                .relative()
                .bg(background)
                .h(px(snap.metrics.target(theme::CONTROL_TARGET_SIZE)))
                .w_full()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(snap.metrics.text(theme::FONT_SIZE_XS)))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(theme.text_primary)
                .hover(|element| element.bg(hover_background))
                .child(super::group_abbreviation(&group.name))
                .when(collapsed, |element| {
                    element.child(
                        div()
                            .id(("rail-group-status", id as usize))
                            .debug_selector(move || format!("rail-group-status-{id}"))
                            .absolute()
                            .bottom_0()
                            .left_0()
                            .w_full()
                            .h(px(theme::DOCK_RAIL_GROUP_LINE_H))
                            .bg(color),
                    )
                })
                .tooltip(ui::tooltip::text(tooltip))
                .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                .on_key_down(move |event, window, cx| {
                    if let Some(workspace) = keyboard_workspace.upgrade() {
                        workspace.update(cx, |workspace, cx| {
                            workspace.on_rail_key(
                                super::ops::RailItem::Group(id),
                                event,
                                window,
                                cx,
                            )
                        });
                    }
                })
                .on_click(move |_, _, cx| {
                    if let Some(workspace) = workspace.upgrade() {
                        workspace
                            .update(cx, |workspace, cx| workspace.toggle_group_collapse(id, cx));
                    }
                })
                .root_context_menu(snap.workspace.clone(), {
                    let workspace = snap.workspace.clone();
                    move |menu, _, _| {
                        super::super::projects::group_menu::build_group_menu_items(
                            id,
                            name.clone(),
                            collapsed,
                            workspace.clone(),
                        )
                        .into_iter()
                        .fold(menu, |menu, item| menu.item(item))
                    }
                }),
        )
        .into_any_element()
}

pub(super) fn active_indicator(
    group: &GroupSnapshot,
    snap: &LeftDockSnapshot,
) -> Option<TabIndicator> {
    if !group.is_collapsed
        || snap.workspace_page.is_some()
        || !snap
            .projects
            .iter()
            .any(|project| project.id == snap.active.project && project.group_id == Some(group.id))
    {
        return None;
    }
    snap.rail_panes
        .get(&snap.active)?
        .iter()
        .find(|pane| pane.pane == snap.focused_pane)
        .and_then(|pane| TabIndicator::resolve(pane.status, pane.outcome))
}

#[cfg(test)]
mod tests {}
