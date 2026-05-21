//! File-viewer top toolbar — path label on the left, mode tabs +
//! diff stats + status badge + close button on the right.
//!
//! Mode-tab pill is the only inner widget; `mode_tab` lives here too
//! so the visual style and the toolbar that uses it move together.

use crate::ui::theme;
use gpui::{Context, IntoElement, MouseButton, MouseDownEvent, Window, div, prelude::*, px};

use crate::path_ext::PathExt;
use crate::surface::strings;
use crate::ui::ContextMenuItem;
use crate::workspace::Workspace;
use crate::workspace::left_dock::git_ops::git_status_color;
use crate::workspace::main_area::file_view_pane::{FileViewMode, PaneFileContent, PaneFileView};

/// Toolbar: path label on the left, Raw/Changes tabs + optional controls + × on the right.
pub(super) fn render_file_viewer_toolbar(
    fv: &PaneFileView,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    let t = theme::current(cx);
    let header_bg = t.file_viewer_header_bg;
    let header_border = t.file_viewer_header_border;
    let header_text = t.file_viewer_header_text;
    let stat_add = t.file_diff_stat_add;
    let stat_del = t.file_diff_stat_del;
    let tab_text = t.file_viewer_tab_text;
    let tab_active_bg = t.file_viewer_tab_active_bg;
    let tab_active_text = t.file_viewer_tab_active_text;
    let close_hover = t.file_viewer_close_hover;

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
    let worktree_id_for_menu = fv.lane_id;

    let is_raw = fv.view_mode == FileViewMode::Raw;
    let is_preview = fv.view_mode == FileViewMode::Preview;
    let is_changes = fv.view_mode == FileViewMode::Changes;
    // Use the path extension so the Preview tab persists across mode switches
    // (content type changes to LoadedDiff in Changes mode, which would otherwise
    // hide the tab for markdown files).
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
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                        let ws = cx.entity().downgrade();
                        let wt = this
                            .active_lanes()
                            .iter()
                            .find(|wt| wt.id == worktree_id_for_menu);
                        let worktree_root = wt.map(|wt| wt.path.clone());
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
                        let make_copy_item = |label: gpui::SharedString, text: String| {
                            let ws = ws.clone();
                            ContextMenuItem::new(label, move |_, _, app| {
                                if let Some(w) = ws.upgrade() {
                                    w.update(app, |this, cx| this.close_context_menu(cx));
                                }
                                app.write_to_clipboard(gpui::ClipboardItem::new_string(
                                    text.clone(),
                                ));
                            })
                        };
                        let items = vec![
                            make_copy_item(strings::file_viewer_copy_abs_path().into(), abs_path),
                            make_copy_item(strings::file_viewer_copy_rel_path().into(), rel_path),
                        ];
                        this.open_context_menu(ev.position, items, cx);
                    }),
                )
                .child(label),
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
                .child(mode_tab(
                    strings::file_viewer_tab_raw(),
                    is_raw,
                    tab_text,
                    tab_active_bg,
                    tab_active_text,
                    cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                        this.set_file_view_mode(FileViewMode::Raw, cx);
                    }),
                ))
                .when(is_markdown, |d| {
                    d.child(mode_tab(
                        strings::file_viewer_tab_preview(),
                        is_preview,
                        tab_text,
                        tab_active_bg,
                        tab_active_text,
                        cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                            this.set_file_view_mode(FileViewMode::Preview, cx);
                        }),
                    ))
                })
                .child(mode_tab(
                    strings::file_viewer_tab_changes(),
                    is_changes,
                    tab_text,
                    tab_active_bg,
                    tab_active_text,
                    cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                        this.set_file_view_mode(FileViewMode::Changes, cx);
                    }),
                ))
                .when(is_changes, |d| {
                    d.child(mode_tab(
                        if hide_unchanged {
                            strings::file_viewer_show_all()
                        } else {
                            strings::file_viewer_hide_unchanged()
                        },
                        true,
                        tab_text,
                        tab_active_bg,
                        tab_active_text,
                        cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                            this.toggle_hide_unchanged(cx);
                        }),
                    ))
                })
                .child(
                    div()
                        .id("file-viewer-close")
                        .flex_none()
                        .px(px(theme::FILE_VIEWER_TAB_PAD_X))
                        .text_size(px(theme::FILE_VIEWER_CLOSE_FONT_SIZE))
                        .text_color(tab_text)
                        .cursor_pointer()
                        .hover(move |d| d.text_color(close_hover))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _: &MouseDownEvent, window, cx| {
                                this.close_focused_file_pane(window, cx);
                            }),
                        )
                        .child(strings::FILE_VIEWER_CLOSE),
                ),
        )
}

/// A small pill button for mode tabs in the file viewer toolbar.
fn mode_tab(
    label: impl Into<gpui::SharedString>,
    active: bool,
    tab_text: gpui::Hsla,
    tab_active_bg: gpui::Hsla,
    tab_active_text: gpui::Hsla,
    on_click: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let label: gpui::SharedString = label.into();
    div()
        .id(label.clone())
        .flex_none()
        .px(px(theme::FILE_VIEWER_TAB_PAD_X))
        .py(px(theme::FILE_VIEWER_TAB_PAD_Y))
        .rounded(px(theme::FILE_VIEWER_TAB_RADIUS))
        .text_size(px(theme::FILE_VIEWER_HEADER_FONT_SIZE))
        .cursor_pointer()
        .when(active, |d| d.bg(tab_active_bg).text_color(tab_active_text))
        .when(!active, |d| {
            d.text_color(tab_text)
                .hover(move |d| d.text_color(tab_active_text))
        })
        .on_mouse_down(MouseButton::Left, on_click)
        .child(label)
}
