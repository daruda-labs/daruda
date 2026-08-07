//! File-viewer top toolbar — path label on the left, diff stats +
//! status badge + view toggles + close button on the right.
//!
//! Toolbar toggle button is the only inner widget; the helper lives here too
//! so the visual style and the toolbar that uses it move together.

use crate::ui::theme;
use gpui::{Context, IntoElement, MouseButton, MouseDownEvent, Window, div, prelude::*, px};

use crate::path_ext::PathExt;
use crate::surface::strings;
use crate::ui::{ContextMenuExt as _, Icon, Sizable as _, menu_builder};
use crate::workspace::Workspace;
use crate::workspace::left_dock::git_ops::git_status_color;
use crate::workspace::main_area::file_view_pane::{FileViewMode, PaneFileContent, PaneFileView};
use crate::workspace::main_area::pane_tree::PaneId;
use crate::workspace::render::ws_popup_clipboard_item;

const ICON_CODE: &str = "icons/ui/code.svg";
const ICON_PREVIEW: &str = "icons/ui/preview.svg";
const ICON_DIFFERENCE: &str = "icons/ui/difference.svg";
const ICON_FILTER_ALT: &str = "icons/ui/filter-alt.svg";
const ICON_FILTER_ALT_OFF: &str = "icons/ui/filter-alt-off.svg";

/// Toolbar: path label on the left, view toggles + optional controls + × on the right.
pub(super) fn render_file_viewer_toolbar(
    pane_id: PaneId,
    fv: &PaneFileView,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    let t = theme::current(cx);
    let header_bg = theme::file_viewer_pane_tint(cx);
    let header_border = theme::file_viewer_pane_border_tint(cx);
    let header_text = theme::file_viewer_pane_fg(cx);
    let stat_add = t.file_diff_stat_add;
    let stat_del = t.file_diff_stat_del;
    let button_text = theme::file_viewer_pane_fg_muted(cx);
    let button_active_bg = theme::file_viewer_pane_active_tint(cx);
    let button_active_text = header_text;
    let close_hover = header_text;

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

    let is_preview = fv.view_mode == FileViewMode::Preview;
    let is_changes = fv.view_mode == FileViewMode::Changes;
    // Use the path extension so the Preview button persists across mode switches
    // (content type changes to LoadedDiff in Changes mode, which would otherwise
    // hide the button for markdown files).
    let is_markdown = fv
        .path
        .extension_lower()
        .is_some_and(|e| e == "md" || e == "markdown");
    let hide_unchanged = fv.hide_unchanged;

    // Extract diff stats when available (Changes mode, file already loaded).
    let diff_stats = match &fv.content {
        PaneFileContent::LoadedDiff { added, removed, .. } => Some((*added, *removed)),
        _ => None,
    };
    let file_status = fv.file_status;
    let staged = fv.staged;
    let file_status_color = file_status.map(|status| git_status_color(status, staged, cx));
    let (filter_label, filter_icon) = if hide_unchanged {
        (strings::file_viewer_show_all(), ICON_FILTER_ALT_OFF)
    } else {
        (strings::file_viewer_hide_unchanged(), ICON_FILTER_ALT)
    };
    let (mode_label, mode_icon, mode_target) = if is_changes {
        (strings::file_viewer_tab_raw(), ICON_CODE, FileViewMode::Raw)
    } else {
        (
            strings::file_viewer_tab_changes(),
            ICON_DIFFERENCE,
            FileViewMode::Changes,
        )
    };

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
                .when_some(diff_stats, |d, (added, removed)| {
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
                .child(if is_changes {
                    toolbar_toggle_button(
                        filter_label,
                        filter_icon,
                        hide_unchanged,
                        button_text,
                        button_active_bg,
                        button_active_text,
                        cx.listener(move |this, _: &MouseDownEvent, _window, cx| {
                            this.toggle_hide_unchanged_for_pane(pane_id, cx);
                        }),
                    )
                    .into_any_element()
                } else {
                    toolbar_toggle_spacer().into_any_element()
                })
                .child(toolbar_toggle_button(
                    mode_label,
                    mode_icon,
                    is_changes,
                    button_text,
                    button_active_bg,
                    button_active_text,
                    cx.listener(move |this, _: &MouseDownEvent, _window, cx| {
                        this.set_file_view_mode_for_pane(pane_id, mode_target, cx);
                    }),
                ))
                .when(is_markdown, |d| {
                    d.child(toolbar_toggle_button(
                        strings::file_viewer_tab_preview(),
                        ICON_PREVIEW,
                        is_preview,
                        button_text,
                        button_active_bg,
                        button_active_text,
                        cx.listener(move |this, _: &MouseDownEvent, _window, cx| {
                            this.set_file_view_mode_for_pane(pane_id, FileViewMode::Preview, cx);
                        }),
                    ))
                })
                .child(
                    div()
                        .id("file-viewer-close")
                        .flex_none()
                        .px(px(theme::FILE_VIEWER_CLOSE_PAD_X))
                        .text_size(px(theme::FILE_VIEWER_CLOSE_FONT_SIZE))
                        .text_color(button_text)
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
    label: impl Into<gpui::SharedString>,
    icon: &'static str,
    active: bool,
    button_text: gpui::Hsla,
    button_active_bg: gpui::Hsla,
    button_active_text: gpui::Hsla,
    on_click: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let label: gpui::SharedString = label.into();
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
        .when(active, |d| {
            d.bg(button_active_bg).text_color(button_active_text)
        })
        .when(!active, |d| {
            d.text_color(button_text)
                .hover(move |d| d.text_color(button_active_text))
        })
        .on_mouse_down(MouseButton::Left, on_click)
        .tooltip(crate::ui::tooltip::text(label))
        .child(Icon::empty().path(icon).small())
}
