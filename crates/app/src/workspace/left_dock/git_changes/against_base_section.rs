//! The against-base section of the Git Changes list: one header row, then
//! the files the lane committed since its base. Read-only — no checkbox, no
//! staging — and never on the keyboard cursor's path.

use gpui::{
    AnyElement, ClickEvent, Context, IntoElement, MouseButton, MouseDownEvent, div, prelude::*, px,
};

use crate::lane::git::base::RangeFile;
use crate::surface::strings as app_strings;
use crate::ui::theme;
use crate::workspace::layout::{Dock, LeftDockSnapshot};
use crate::workspace::left_dock::git_ops::{git_status_color, git_status_symbol};
use crate::workspace::main_area::file_view_pane::DiffSource;

use super::unified_list::BaseHeaderRow;

pub(super) fn base_header(
    idx: usize,
    row: &BaseHeaderRow,
    snap: &LeftDockSnapshot,
    cx: &mut Context<Dock>,
) -> AnyElement {
    let t = theme::current(cx);
    let label_color = t.text_subtle;
    let label_hover = t.text_muted;
    let target = snap.active;
    let workspace = snap.workspace.clone();

    let row_box = div()
        .id(("git-base-header", idx))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::GIT_FILE_ROW_GAP))
        .px(px(theme::GIT_HEADER_PAD_X))
        .w_full()
        // One height for every row: `uniform_list` measures a single item.
        .h(px(theme::GIT_FILE_ROW_HEIGHT))
        .text_size(px(theme::GIT_DIR_HEADER_FONT_SIZE))
        .text_color(label_color);

    match row {
        BaseHeaderRow::Summary {
            base,
            files,
            commits,
            collapsed,
        } => {
            let chevron = if *collapsed {
                crate::ui::icons::CHEVRON_RIGHT
            } else {
                crate::ui::icons::EXPAND_MORE
            };
            row_box
                .cursor_pointer()
                .hover(move |d| d.text_color(label_hover))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |_dock, _: &MouseDownEvent, _window, cx| {
                        if let Some(ws) = workspace.upgrade() {
                            ws.update(cx, |ws, cx| ws.toggle_against_base_collapse(target, cx));
                        }
                    }),
                )
                .child(crate::ui::icons::icon(chevron).text_color(label_color))
                .child(
                    div()
                        .flex_1()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .child(app_strings::git_against_base_header(base)),
                )
                .child(
                    div()
                        .flex_none()
                        .whitespace_nowrap()
                        .child(app_strings::git_against_base_counts(*files, *commits)),
                )
                .into_any_element()
        }
        BaseHeaderRow::BaseMissing(name) => row_box
            .child(notice(app_strings::git_against_base_missing(name)))
            .into_any_element(),
        BaseHeaderRow::NoMergeBase => row_box
            .child(notice(app_strings::git_against_base_no_merge_base()))
            .into_any_element(),
    }
}

fn notice(text: String) -> impl IntoElement {
    div()
        .flex_1()
        .overflow_hidden()
        .whitespace_nowrap()
        .child(text)
}

pub(super) fn base_file_row(
    idx: usize,
    file: &RangeFile,
    snap: &LeftDockSnapshot,
    cx: &mut Context<Dock>,
) -> AnyElement {
    let target = snap.active;
    let lane_id = target.lane;
    let abs = snap
        .lanes
        .iter()
        .find(|w| w.id == lane_id)
        .map(|w| w.paths().from_git_status(&file.path));
    // Only a pane pinned to the commits this row was listed from is this
    // row's; one left open across a later commit shows an older range.
    let listed = match snap.git_against_base.as_deref() {
        Some(Ok(found)) => Some((found.merge_base.as_str(), found.tips.head.as_str())),
        _ => None,
    };
    let is_selected = matches!(
        (&snap.focused_file_selection, &abs, listed),
        (Some((lane, path, DiffSource::Range { from, to, .. })), Some(abs), Some((mb, head)))
            if *lane == lane_id && path == abs && from == mb && to == head
    );

    let t = theme::current(cx);
    let row_selected_bg = t.git_file_row_selected_bg;
    let row_hover_bg = t.git_file_row_hover_bg;
    let filename_color = t.text_muted;
    let diff_add_color = t.file_diff_stat_add;
    let diff_del_color = t.file_diff_stat_del;
    // Always committed — the answer `DiffSource::reads_as_committed` gives the
    // range pane this row opens, so the row and its pane's badge agree.
    let status_color = git_status_color(file.status, true, cx);

    // The section is flat — no directory groups — so the row carries the
    // repo-relative path; a bare file name would lose which `mod.rs` it is.
    let file_name = match &file.old_path {
        Some(old) => format!("{} ← {}", file.path.display(), old.display()),
        None => file.path.display().to_string(),
    };
    let (added, removed) = (file.added, file.removed);
    let repo_rel = file.path.clone();
    let workspace = snap.workspace.clone();

    div()
        .id(("git-base-file", idx))
        .flex()
        .flex_row()
        .items_center()
        .w_full()
        .h(px(theme::GIT_FILE_ROW_HEIGHT))
        .px(px(theme::GIT_FILE_ROW_PAD_X))
        .gap(px(theme::GIT_FILE_ROW_GAP))
        .cursor_pointer()
        .when(is_selected, move |d| d.bg(row_selected_bg))
        .when(!is_selected, move |d| d.hover(move |d| d.bg(row_hover_bg)))
        .on_click(cx.listener(move |_dock, ev: &ClickEvent, window, cx| {
            if let Some(ws) = workspace.upgrade() {
                let (path, clicks) = (repo_rel.clone(), ev.click_count());
                ws.update(cx, |ws, cx| {
                    ws.on_against_base_row_click(target, path, clicks, window, cx)
                });
            }
        }))
        .child(
            div()
                .flex_none()
                .w(px(theme::GIT_STATUS_CHAR_W))
                .text_size(px(theme::GIT_SECTION_FONT_SIZE))
                .text_color(status_color)
                .child(git_status_symbol(file.status)),
        )
        .child(
            div()
                .flex_1()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(theme::GIT_FILE_ROW_GAP))
                .overflow_hidden()
                .child(
                    div()
                        .flex_shrink()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_size(px(theme::LANE_SUB_FONT_SIZE))
                        .text_color(filename_color)
                        .child(file_name),
                )
                .when(added > 0 || removed > 0, |d| {
                    d.child(
                        div()
                            .flex_none()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(theme::FILE_DIFF_STAT_GAP))
                            .text_size(px(theme::FILE_DIFF_STAT_FONT_SIZE))
                            .child(div().text_color(diff_add_color).child(format!("+{added}")))
                            .child(
                                div()
                                    .text_color(diff_del_color)
                                    .child(format!("-{removed}")),
                            ),
                    )
                }),
        )
        .into_any_element()
}
