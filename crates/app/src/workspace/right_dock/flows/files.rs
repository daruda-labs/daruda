//! File row actions retain the same authoring and execution entry points.

use crate::surface::strings;
use crate::ui::{Disableable as _, DropdownMenu as _, theme, tooltip};
use crate::workspace::layout::RightDockSnapshot;
use crate::workspace::root_menu::RootContextMenuExt as _;
use gpui::{IntoElement, SharedString, div, prelude::*};

const ICON_PLAY: &str = "icons/ui/play-arrow.svg";

fn flow_row_menu(
    path: std::path::PathBuf,
    name: String,
    origin: crate::workspace::flow_paths::FlowOrigin,
    ws: gpui::WeakEntity<crate::workspace::Workspace>,
) -> Vec<crate::ui::PopupMenuItem> {
    use crate::workspace::render::ws_popup_menu_item;

    let rename_path = path.clone();
    let rename_from = name.clone();
    let rename = ws_popup_menu_item(
        ws.clone(),
        strings::flow::row_menu_rename(),
        false,
        move |_, window, cx| {
            let weak = cx.entity().downgrade();
            let path = rename_path.clone();
            let initial = rename_from.clone();
            crate::workspace::dialog_helpers::open_single_field_dialog(
                weak,
                strings::flow::rename_title(),
                strings::flow::new_placeholder(),
                Some(&initial),
                move |ws, value, _window, cx| {
                    let Some(to) = value else {
                        return;
                    };
                    ws.rename_flow(&path, &to, cx);
                },
                window,
                cx,
            );
        },
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
    vec![rename, delete]
}

fn run_button(
    found: &crate::workspace::flow_paths::FoundFlow,
    snap: &RightDockSnapshot,
    cx: &gpui::App,
) -> impl IntoElement {
    let workspace = snap.workspace.clone();
    let path = found.path.clone();
    let lane = snap.flow_lane;
    let unsaved = snap.flows_with_unsaved_edits.contains(&path);
    let id = SharedString::from(format!("flow-run-{}", path.display()));
    let selector = id.to_string();
    div()
        .flex_none()
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .debug_selector(move || selector)
        .child(
            crate::ui::button_icon(id, ICON_PLAY, cx)
                .tooltip(if unsaved {
                    strings::flow::needs_save()
                } else {
                    strings::flow::run_tooltip()
                })
                .disabled(unsaved)
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
    snap: &RightDockSnapshot,
    cx: &gpui::App,
) -> impl IntoElement {
    let t = theme::current(cx);
    let workspace = snap.workspace.clone();
    let ws_for_menu = snap.workspace.clone();
    let path = found.path.clone();
    let menu_path = path.clone();
    let name = crate::workspace::flow_paths::flow_label(&path);
    let menu_name = name.clone();
    let origin = found.origin;
    let lane = snap.flow_lane;
    let dropdown_path = path.clone();
    let dropdown_name = name.clone();
    let dropdown_workspace = ws_for_menu.clone();
    let selector = format!("flow-file-{}", path.display());
    let modified = snap
        .flow_browser
        .modified
        .iter()
        .find(|(held, _)| *held == path)
        .map(|(_, time)| strings::flow::run_started_at((*time).into()))
        .unwrap_or_else(strings::flow::not_recorded);
    super::rows::frame()
        .id(SharedString::from(selector.clone()))
        .debug_selector(move || selector)
        .hover(move |style| style.bg(t.overlay_hover))
        .cursor_pointer()
        .child(
            super::rows::title_cell()
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
                        .child(name),
                ),
        )
        .child(
            super::rows::cell(theme::FLOW_TABLE_ORIGIN_W)
                .id("flow-origin")
                .debug_selector({
                    let path = path.clone();
                    move || format!("flow-source-{}", path.display())
                })
                .text_color(t.text_muted)
                .child(super::controls::origin_label(Some(origin))),
        )
        .child(
            super::rows::cell(theme::FLOW_TABLE_TIME_W)
                .text_color(t.text_muted)
                .child(modified),
        )
        .child(
            super::rows::actions()
                .child(run_button(found, snap, cx))
                .child(
                    crate::ui::button_icon(
                        SharedString::from(format!("flow-menu-{}", path.display())),
                        crate::ui::icons::EXPAND_MORE,
                        cx,
                    )
                    .tooltip(strings::flow::column_actions())
                    .tab_stop(true)
                    .dropdown_menu(crate::ui::menu_builder(
                        move |menu, _, _| {
                            flow_row_menu(
                                dropdown_path.clone(),
                                dropdown_name.clone(),
                                origin,
                                dropdown_workspace.clone(),
                            )
                            .into_iter()
                            .fold(menu, |menu, item| menu.item(item))
                        },
                    )),
                ),
        )
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
            )
            .into_iter()
            .fold(menu, |menu, item| menu.item(item))
        })
}
