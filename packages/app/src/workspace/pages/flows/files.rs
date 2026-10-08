//! File row actions retain the same authoring and execution entry points.

use crate::surface::strings;
use crate::ui::{DropdownMenu as _, theme, tooltip};
use crate::workspace::flow_browser::FlowPageSnapshot;
use crate::workspace::root_menu::RootContextMenuExt as _;
use gpui::{IntoElement, SharedString, div, prelude::*};

const ICON_PLAY: &str = "icons/ui/play-arrow.svg";

fn flow_row_menu(
    path: std::path::PathBuf,
    name: String,
    origin: crate::workspace::flow_paths::FlowOrigin,
    ws: gpui::WeakEntity<crate::workspace::Workspace>,
    lane: daruda_store::project::LaneRef,
) -> Vec<crate::ui::PopupMenuItem> {
    use crate::workspace::render::ws_popup_menu_item;

    let rename_path = path.clone();
    let rename_from = crate::workspace::flow_paths::flow_label(&path);
    let rename = ws_popup_menu_item(
        ws.clone(),
        strings::flow::rename_file(),
        false,
        move |_, window, cx| {
            let weak = cx.entity().downgrade();
            let path = rename_path.clone();
            let initial = rename_from.clone();
            crate::workspace::dialog_helpers::open_single_field_dialog(
                weak,
                strings::flow::rename_title(),
                strings::flow::file_name_placeholder(),
                Some(&initial),
                move |ws, value, window, cx| {
                    let Some(to) = value else {
                        return;
                    };
                    ws.rename_flow(&path, &to, window, cx);
                },
                window,
                cx,
            );
        },
    );

    let edit_path = path.clone();
    let edit_name = ws_popup_menu_item(
        ws.clone(),
        strings::flow::row_menu_rename(),
        false,
        move |ws, window, cx| ws.edit_browsed_flow_name(lane, &edit_path, window, cx),
    );
    let delete_path = path;
    let delete_name = name;
    let delete = ws_popup_menu_item(
        ws,
        strings::flow::row_menu_delete(),
        false,
        move |_, window, cx| {
            let weak = cx.entity().downgrade();
            crate::workspace::flow_file_ops::ask_before_deleting(
                delete_path.clone(),
                &delete_name,
                origin,
                weak,
                window,
                cx,
            );
        },
    );
    vec![edit_name, rename, delete]
}

fn run_button(
    found: &crate::workspace::flow_paths::FoundFlow,
    snap: &FlowPageSnapshot,
    cx: &gpui::App,
) -> impl IntoElement {
    let workspace = snap.workspace.clone();
    let path = found.path.clone();
    let lane = snap.flow_lane;
    let id = SharedString::from(format!("flow-run-{}", path.display()));
    let selector = id.to_string();
    div()
        .flex_none()
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .debug_selector(move || selector)
        .child(
            crate::ui::button_icon(id, ICON_PLAY, cx)
                .tooltip(strings::flow::run_tooltip())
                .tab_stop(true)
                .on_click(move |_, window, cx| {
                    if let Some(ws) = workspace.upgrade() {
                        ws.update(cx, |ws, cx| ws.run_browsed_flow(lane, &path, window, cx));
                    }
                }),
        )
}

pub(super) fn flow_row(
    found: &crate::workspace::flow_paths::FoundFlow,
    snap: &FlowPageSnapshot,
    layout: &crate::ui::list_table::ListTable<super::rows::FileColumn>,
    cx: &gpui::App,
) -> impl IntoElement {
    use super::rows::FileColumn;
    let t = theme::current(cx);
    let workspace = snap.workspace.clone();
    let ws_for_menu = snap.workspace.clone();
    let path = found.path.clone();
    let menu_path = path.clone();
    let name = found.name.clone();
    let menu_name = name.clone();
    let origin = found.origin;
    let lane = snap.flow_lane;
    let selector = format!("flow-file-{}", path.display());
    let modified = snap
        .flow_browser
        .modified
        .iter()
        .find(|(held, _)| *held == path)
        .map(|(_, time)| strings::flow::run_started_at((*time).into()))
        .unwrap_or_else(strings::flow::not_recorded);
    layout
        .row(|column, cell| match column {
            FileColumn::Title => cell
                .id("flow-name")
                .debug_selector({
                    let path = path.clone();
                    move || format!("flow-title-{}", path.display())
                })
                .tooltip(tooltip::text(path.display().to_string()))
                .child(
                    div()
                        .w_full()
                        .truncate()
                        .text_color(t.text_body)
                        .child(name.clone()),
                )
                .into_any_element(),
            FileColumn::Source => cell
                .id("flow-origin")
                .debug_selector({
                    let path = path.clone();
                    move || format!("flow-source-{}", path.display())
                })
                .text_color(t.text_muted)
                .child(super::controls::origin_label(Some(origin)))
                .into_any_element(),
            FileColumn::Modified => cell
                .text_color(t.text_muted)
                .child(modified.clone())
                .into_any_element(),
            FileColumn::Actions => {
                let dropdown_path = path.clone();
                let dropdown_name = name.clone();
                let dropdown_workspace = ws_for_menu.clone();
                super::rows::actions(cell)
                    .child(run_button(found, snap, cx))
                    .child(
                        crate::ui::button_icon(
                            SharedString::from(format!("flow-menu-{}", path.display())),
                            crate::ui::icons::EXPAND_MORE,
                            cx,
                        )
                        .tooltip(strings::flow::column_actions())
                        .tab_stop(true)
                        .dropdown_menu(crate::ui::menu_builder(move |menu, _, _| {
                            flow_row_menu(
                                dropdown_path.clone(),
                                dropdown_name.clone(),
                                origin,
                                dropdown_workspace.clone(),
                                lane,
                            )
                            .into_iter()
                            .fold(menu, |menu, item| menu.item(item))
                        })),
                    )
                    .into_any_element()
            }
        })
        .id(SharedString::from(selector.clone()))
        .debug_selector(move || selector)
        .hover(move |style| style.bg(t.overlay_hover))
        .cursor_pointer()
        .on_click(move |_, window, cx| {
            if let Some(ws) = workspace.upgrade() {
                ws.update(cx, |ws, cx| ws.open_browsed_flow(lane, &path, window, cx));
            }
        })
        .root_context_menu(ws_for_menu.clone(), move |menu, _, _| {
            flow_row_menu(
                menu_path.clone(),
                menu_name.clone(),
                origin,
                ws_for_menu.clone(),
                lane,
            )
            .into_iter()
            .fold(menu, |menu, item| menu.item(item))
        })
}
