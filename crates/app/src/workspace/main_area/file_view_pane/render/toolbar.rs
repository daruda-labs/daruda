//! File-viewer top toolbar — path label on the left, diff stats +
//! status badge + view toggles + close button on the right.
//!
//! Toolbar toggle button is the only inner widget; the helper lives here too
//! so the visual style and the toolbar that uses it move together.

use crate::ui::theme;
use gpui::{Context, IntoElement, MouseButton, MouseDownEvent, div, prelude::*, px};

use crate::path_ext::PathExt;
use crate::surface::strings;
use crate::ui::{ContextMenuExt as _, Icon, Sizable as _, menu_builder};
use crate::workspace::Workspace;
use crate::workspace::left_dock::git_ops::git_status_color;
use crate::workspace::main_area::file_view_pane::{FileViewMode, PaneFileView};
use crate::workspace::main_area::pane_tree::PaneId;
use crate::workspace::render::ws_popup_clipboard_item;

const ICON_CODE: &str = "icons/ui/code.svg";
const ICON_PREVIEW: &str = "icons/ui/preview.svg";
const ICON_DIFFERENCE: &str = "icons/ui/difference.svg";
const ICON_FILTER_ALT: &str = "icons/ui/filter-alt.svg";
const ICON_FILTER_ALT_OFF: &str = "icons/ui/filter-alt-off.svg";

#[derive(Clone, Copy)]
struct ToolbarButtonColors {
    text: gpui::Hsla,
    active_bg: gpui::Hsla,
    active_text: gpui::Hsla,
}

#[derive(Clone, Copy)]
enum ToolbarAction {
    ToggleHideUnchanged,
    SetMode(FileViewMode),
}

struct ToolbarToggle {
    label: gpui::SharedString,
    icon: &'static str,
    active: bool,
    action: ToolbarAction,
}

impl ToolbarToggle {
    fn new(
        label: impl Into<gpui::SharedString>,
        icon: &'static str,
        active: bool,
        action: ToolbarAction,
    ) -> Self {
        Self {
            label: label.into(),
            icon,
            active,
            action,
        }
    }

    fn render(
        self,
        pane_id: PaneId,
        colors: ToolbarButtonColors,
        cx: &mut Context<Workspace>,
    ) -> impl IntoElement {
        toolbar_toggle_button(self, pane_id, colors, cx)
    }
}

struct ToolbarControls {
    filter_toggle: Option<ToolbarToggle>,
    mode_toggle: ToolbarToggle,
    preview_toggle: Option<ToolbarToggle>,
    diff_stats: Option<(usize, usize)>,
}

impl ToolbarControls {
    fn from_view(fv: &PaneFileView) -> Self {
        let is_preview = fv.view_mode == FileViewMode::Preview;
        let is_changes = fv.view_mode == FileViewMode::Changes;

        let filter_toggle = is_changes.then(|| {
            if fv.hide_unchanged {
                ToolbarToggle::new(
                    strings::file_viewer_show_all(),
                    ICON_FILTER_ALT_OFF,
                    true,
                    ToolbarAction::ToggleHideUnchanged,
                )
            } else {
                ToolbarToggle::new(
                    strings::file_viewer_hide_unchanged(),
                    ICON_FILTER_ALT,
                    false,
                    ToolbarAction::ToggleHideUnchanged,
                )
            }
        });

        let mode_toggle = if is_changes {
            ToolbarToggle::new(
                strings::file_viewer_tab_raw(),
                ICON_CODE,
                true,
                ToolbarAction::SetMode(FileViewMode::Raw),
            )
        } else {
            ToolbarToggle::new(
                strings::file_viewer_tab_changes(),
                ICON_DIFFERENCE,
                false,
                ToolbarAction::SetMode(FileViewMode::Changes),
            )
        };

        let preview_toggle = fv.is_markdown_path().then(|| {
            ToolbarToggle::new(
                strings::file_viewer_tab_preview(),
                ICON_PREVIEW,
                is_preview,
                ToolbarAction::SetMode(FileViewMode::Preview),
            )
        });

        Self {
            filter_toggle,
            mode_toggle,
            preview_toggle,
            diff_stats: fv.loaded_diff_stats(),
        }
    }
}

/// Toolbar: path label on the left, view toggles + optional controls + × on the right.
pub(super) fn render_file_viewer_toolbar(
    pane_id: PaneId,
    fv: &PaneFileView,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    let t = theme::current(cx);
    let surface = theme::PaneSurfaceTokens::file_viewer(cx);
    let header_bg = surface.tint;
    let header_border = surface.border_tint;
    let header_text = surface.foreground;
    let stat_add = t.file_diff_stat_add;
    let stat_del = t.file_diff_stat_del;
    let button_colors = ToolbarButtonColors {
        text: surface.foreground_muted,
        active_bg: surface.active_tint,
        active_text: header_text,
    };
    let close_hover = header_text;
    let controls = ToolbarControls::from_view(fv);

    let file_name = fv.path.file_name_lossy();
    let parent_name = fv
        .path
        .parent()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned());
    let staged_badge = if fv.staged {
        strings::file_viewer_staged_badge()
    } else {
        String::new()
    };
    let label = match parent_name {
        Some(dir) => format!(
            "{dir}{}{file_name}{staged_badge}",
            strings::FILE_VIEWER_PATH_SEP
        ),
        None => format!("{file_name}{staged_badge}"),
    };

    let path_for_menu = fv.path.clone();
    let lane_id_for_menu = fv.lane_id;
    let ws_for_menu = cx.entity().downgrade();

    let file_status = fv.file_status;
    let staged = fv.staged;
    let file_status_color = file_status.map(|status| git_status_color(status, staged, cx));

    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .h(px(theme::FILE_VIEWER_HEADER_H))
        .px(px(theme::FILE_VIEWER_HEADER_PAD_X))
        .bg(header_bg)
        .border_b_1()
        .border_color(header_border)
        .child(
            div()
                .id("file-viewer-path-label")
                .flex_1()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_size(px(theme::FILE_VIEWER_HEADER_FONT_SIZE))
                .text_color(header_text)
                .child(label)
                .context_menu(menu_builder(move |menu, _window, cx| {
                    let Some(ws) = ws_for_menu.upgrade() else {
                        return menu;
                    };
                    let wt = ws
                        .read(cx)
                        .active_lanes()
                        .iter()
                        .find(|wt| wt.id == lane_id_for_menu)
                        .cloned();
                    let worktree_root = wt.as_ref().map(|wt| wt.path.clone());
                    // `path_for_menu` is absolute (set at the left-dock entry point).
                    // For legacy relative paths from old session state use
                    // LanePaths::from_git_status so the repo_root/wt_path
                    // selection is consistent with every other path call site.
                    let abs_pathbuf = if path_for_menu.is_absolute() {
                        path_for_menu.clone()
                    } else {
                        wt.map(|wt| wt.paths().from_git_status(&path_for_menu))
                            .unwrap_or_else(|| path_for_menu.clone())
                    };
                    let abs_path = abs_pathbuf.to_string_lossy().to_string();
                    let rel_path = worktree_root
                        .as_ref()
                        .and_then(|root| abs_pathbuf.strip_prefix(root).ok())
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_else(|| path_for_menu.to_string_lossy().to_string());

                    menu.item(ws_popup_clipboard_item(
                        strings::file_viewer_copy_abs_path(),
                        abs_path,
                    ))
                    .item(ws_popup_clipboard_item(
                        strings::file_viewer_copy_rel_path(),
                        rel_path,
                    ))
                })),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(theme::FILE_VIEWER_TOOLBAR_GAP))
                // Diff line stats: +N -N
                .when_some(controls.diff_stats, |d, (added, removed)| {
                    d.child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(theme::FILE_DIFF_STAT_GAP))
                            .text_size(px(theme::FILE_DIFF_STAT_FONT_SIZE))
                            .child(
                                div()
                                    .flex_none()
                                    .text_color(stat_add)
                                    .child(format!("+{added}")),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .text_color(stat_del)
                                    .child(format!("-{removed}")),
                            ),
                    )
                })
                // File status badge (M / A / D / R / ?)
                .when_some(file_status, |d, status| {
                    let color = file_status_color.unwrap_or(header_text);
                    d.child(
                        div()
                            .flex_none()
                            .text_size(px(theme::FILE_DIFF_STAT_FONT_SIZE))
                            .text_color(color)
                            .child(status.to_string()),
                    )
                })
                .child(if let Some(toggle) = controls.filter_toggle {
                    toggle.render(pane_id, button_colors, cx).into_any_element()
                } else {
                    toolbar_toggle_spacer().into_any_element()
                })
                .child(controls.mode_toggle.render(pane_id, button_colors, cx))
                .when_some(controls.preview_toggle, |d, toggle| {
                    d.child(toggle.render(pane_id, button_colors, cx))
                })
                .child(
                    div()
                        .id("file-viewer-close")
                        .flex_none()
                        .px(px(theme::FILE_VIEWER_CLOSE_PAD_X))
                        .text_size(px(theme::FILE_VIEWER_CLOSE_FONT_SIZE))
                        .text_color(button_colors.text)
                        .cursor_pointer()
                        .hover(move |d| d.text_color(close_hover))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                                this.request_close_pane(pane_id, window, cx);
                            }),
                        )
                        .child(strings::FILE_VIEWER_CLOSE),
                ),
        )
}

/// A fixed-width spacer that keeps the Raw/Changes toggle from shifting when
/// the diff-context toggle is unavailable outside Changes mode.
fn toolbar_toggle_spacer() -> impl IntoElement {
    div()
        .flex_none()
        .w(px(theme::FILE_VIEWER_TOOL_BUTTON_W))
        .h(px(theme::FILE_VIEWER_TOOL_BUTTON_H))
}

/// A small icon button for file-viewer toolbar toggles.
fn toolbar_toggle_button(
    toggle: ToolbarToggle,
    pane_id: PaneId,
    colors: ToolbarButtonColors,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    let label = toggle.label;
    let action = toggle.action;
    div()
        .id(label.clone())
        .flex_none()
        .w(px(theme::FILE_VIEWER_TOOL_BUTTON_W))
        .h(px(theme::FILE_VIEWER_TOOL_BUTTON_H))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(theme::FILE_VIEWER_TOOL_BUTTON_RADIUS))
        .text_size(px(theme::FILE_VIEWER_HEADER_FONT_SIZE))
        .cursor_pointer()
        .when(toggle.active, |d| {
            d.bg(colors.active_bg).text_color(colors.active_text)
        })
        .when(!toggle.active, |d| {
            d.text_color(colors.text)
                .hover(move |d| d.text_color(colors.active_text))
        })
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _: &MouseDownEvent, window, cx| match action {
                ToolbarAction::ToggleHideUnchanged => {
                    this.toggle_hide_unchanged_for_pane(pane_id, window, cx);
                }
                ToolbarAction::SetMode(mode) => {
                    this.set_file_view_mode_for_pane(pane_id, mode, cx);
                }
            }),
        )
        .tooltip(crate::ui::tooltip::text(label))
        .child(Icon::empty().path(toggle.icon).small())
}
