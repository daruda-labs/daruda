//! File-viewer top toolbar — path label on the left, diff stats +
//! status badge + mode strip + diff-context toggle + close button on the right.
//!
//! The mode strip is a single-select button group over the modes the file
//! actually has; the diff-context toggle is a standalone latch, so it keeps
//! its own hand-rolled button (the helper lives here so its visual style and
//! the toolbar that uses it move together).

use crate::ui::theme;
use gpui::{Context, IntoElement, MouseButton, MouseDownEvent, div, prelude::*, px};

use crate::path_ext::PathExt;
use crate::surface::strings;
use crate::ui::{
    ContextMenuExt as _, Icon, Selectable as _, Sizable as _, button_bare, button_group_on_surface,
    menu_builder,
};
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

struct ToolbarToggle {
    label: gpui::SharedString,
    icon: &'static str,
    active: bool,
}

impl ToolbarToggle {
    fn new(label: impl Into<gpui::SharedString>, icon: &'static str, active: bool) -> Self {
        Self {
            label: label.into(),
            icon,
            active,
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
    mode_options: Vec<FileViewMode>,
    active_mode: FileViewMode,
    diff_stats: Option<(usize, usize)>,
}

impl ToolbarControls {
    fn from_view(fv: &PaneFileView) -> Self {
        let filter_toggle = (fv.view_mode == FileViewMode::Changes).then(|| {
            if fv.hide_unchanged {
                ToolbarToggle::new(strings::file_viewer_show_all(), ICON_FILTER_ALT_OFF, true)
            } else {
                ToolbarToggle::new(
                    strings::file_viewer_hide_unchanged(),
                    ICON_FILTER_ALT,
                    false,
                )
            }
        });

        Self {
            filter_toggle,
            mode_options: mode_options(fv),
            active_mode: fv.view_mode,
            diff_stats: fv.loaded_diff_stats(),
        }
    }
}

/// The mode segments offered for a file, in strip order.
///
/// Raw is always available, Preview only for Markdown, Changes only when the
/// file carries a git status. The live mode is kept regardless of that filter
/// so the strip always marks exactly one segment — a restored pane keeps its
/// persisted mode but not its `file_status` (see `persistence.rs`).
fn mode_options(fv: &PaneFileView) -> Vec<FileViewMode> {
    [
        (FileViewMode::Raw, true),
        (FileViewMode::Preview, fv.is_markdown_path()),
        (FileViewMode::Changes, fv.file_status.is_some()),
    ]
    .into_iter()
    .filter(|(mode, available)| *available || *mode == fv.view_mode)
    .map(|(mode, _)| mode)
    .collect()
}

fn mode_icon(mode: FileViewMode) -> &'static str {
    match mode {
        FileViewMode::Raw => ICON_CODE,
        FileViewMode::Preview => ICON_PREVIEW,
        FileViewMode::Changes => ICON_DIFFERENCE,
    }
}

fn mode_label(mode: FileViewMode) -> String {
    match mode {
        FileViewMode::Raw => strings::file_viewer_tab_raw(),
        FileViewMode::Preview => strings::file_viewer_tab_preview(),
        FileViewMode::Changes => strings::file_viewer_tab_changes(),
    }
}

fn mode_button_id(mode: FileViewMode) -> &'static str {
    match mode {
        FileViewMode::Raw => "file-viewer-mode-raw",
        FileViewMode::Preview => "file-viewer-mode-preview",
        FileViewMode::Changes => "file-viewer-mode-changes",
    }
}

/// Single-select icon strip over `options`; the click handler maps the
/// reported child index back to its mode. Element ids carry `pane_id` so
/// split panes don't share the buttons' keyed focus state.
fn mode_button_group(
    pane_id: PaneId,
    options: Vec<FileViewMode>,
    active: FileViewMode,
    colors: ToolbarButtonColors,
    surface: &theme::PaneSurfaceTokens,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    let values = options.clone();
    button_group_on_surface(("file-viewer-mode-group", pane_id), surface, cx)
        .children(options.into_iter().map(|mode| {
            let is_active = mode == active;
            button_bare((mode_button_id(mode), pane_id))
                .icon(Icon::empty().path(mode_icon(mode)))
                .tooltip(mode_label(mode))
                .selected(is_active)
                // The variant carries one foreground; lift only the selected
                // segment's, matching the diff-context toggle's active state.
                .when(is_active, |b| b.text_color(colors.active_text))
        }))
        .on_click(cx.listener(move |this, ixs: &Vec<usize>, _window, cx| {
            if let Some(&ix) = ixs.first() {
                this.set_file_view_mode_for_pane(pane_id, values[ix], cx);
            }
        }))
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
                // Blocks the pane's own right-click underneath: this label has a
                // menu of its own (the file, its worktree), and `.context_menu()`
                // is a raw window listener that no propagation rule reaches — so
                // the hitbox is what has to say "handled here".
                .occlude()
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
                .child(mode_button_group(
                    pane_id,
                    controls.mode_options,
                    controls.active_mode,
                    button_colors,
                    &surface,
                    cx,
                ))
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

/// A fixed-width spacer that keeps the mode strip from shifting when the
/// diff-context toggle is unavailable outside Changes mode.
fn toolbar_toggle_spacer() -> impl IntoElement {
    div()
        .flex_none()
        .w(px(theme::FILE_VIEWER_TOOL_BUTTON_W))
        .h(px(theme::FILE_VIEWER_TOOL_BUTTON_H))
}

/// A small icon button for the diff-context toggle.
fn toolbar_toggle_button(
    toggle: ToolbarToggle,
    pane_id: PaneId,
    colors: ToolbarButtonColors,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    let label = toggle.label;
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
            cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                this.toggle_hide_unchanged_for_pane(pane_id, window, cx);
            }),
        )
        .tooltip(crate::ui::tooltip::text(label))
        .child(Icon::empty().path(toggle.icon).small())
}

#[cfg(test)]
mod tests {
    use super::{FileViewMode, PaneFileView, mode_options};
    use crate::workspace::main_area::file_view_pane::PaneFileContent;

    fn view(name: &str, file_status: Option<char>, view_mode: FileViewMode) -> PaneFileView {
        let mut fv = PaneFileView::loading(0, name.into(), false, file_status, view_mode);
        fv.content = PaneFileContent::LoadedRaw;
        fv
    }

    #[test]
    fn plain_unchanged_file_offers_raw_only() {
        assert_eq!(
            mode_options(&view("a.txt", None, FileViewMode::Raw)),
            vec![FileViewMode::Raw]
        );
    }

    #[test]
    fn plain_changed_file_offers_raw_and_changes() {
        assert_eq!(
            mode_options(&view("a.txt", Some('M'), FileViewMode::Raw)),
            vec![FileViewMode::Raw, FileViewMode::Changes]
        );
    }

    #[test]
    fn unchanged_markdown_offers_raw_and_preview() {
        assert_eq!(
            mode_options(&view("a.md", None, FileViewMode::Preview)),
            vec![FileViewMode::Raw, FileViewMode::Preview]
        );
    }

    #[test]
    fn changed_markdown_offers_all_three_in_strip_order() {
        assert_eq!(
            mode_options(&view("a.md", Some('M'), FileViewMode::Preview)),
            vec![
                FileViewMode::Raw,
                FileViewMode::Preview,
                FileViewMode::Changes
            ]
        );
    }

    /// A restored pane keeps its persisted mode but loses `file_status`, so
    /// the filter alone would leave the strip with nothing selected.
    #[test]
    fn live_mode_survives_a_missing_file_status() {
        assert_eq!(
            mode_options(&view("a.txt", None, FileViewMode::Changes)),
            vec![FileViewMode::Raw, FileViewMode::Changes]
        );
    }
}
