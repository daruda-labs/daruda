//! One working-tree row of the Git Changes list: status letter, file name
//! and diffstat, the stage checkbox, and the row's context menu.

use std::path::PathBuf;

use daruda_store::project::{LaneId, LaneRef};
use gpui::{
    AnyElement, ClickEvent, Context, ElementId, MouseButton, MouseDownEvent, div, prelude::*, px,
};

use crate::lane::paths::LanePaths;
use crate::path_ext::PathExt;
use crate::surface::strings as app_strings;
use crate::ui::{PopupMenuItem, theme};
use crate::workspace::layout::{Dock, LeftDockSnapshot};
use crate::workspace::left_dock::git_ops::git_status_color;
use crate::workspace::main_area::file_view_pane::DiffSource;
use crate::workspace::main_area::tab_ops::OpenIntent;
use crate::workspace::path_drag::PathDrag;
use crate::workspace::root_menu::RootContextMenuExt as _;

use super::unified_list::UnifiedEntry;

#[allow(clippy::too_many_arguments)]
pub(super) fn unified_file_row(
    idx: usize,
    entry: &UnifiedEntry,
    target: LaneRef,
    wt_paths: &LanePaths<'_>,
    selected: Option<&(LaneId, PathBuf, DiffSource)>,
    is_cursor: bool,
    snap: &LeftDockSnapshot,
    cx: &mut Context<Dock>,
) -> AnyElement {
    let path = entry.path.clone();
    let path_for_checkbox = path.clone();
    let path_for_cursor = path.clone();
    let path_for_ctx = path.clone();
    let abs_path_for_open = wt_paths.from_git_status(&path);
    let abs_path_for_ctx_diff = abs_path_for_open.clone();
    let is_staged = entry.staged.is_some();

    // A range pane shows the same path from commits, not this row's change.
    let is_selected = selected.is_some_and(|(wt, p, source)| {
        *wt == target.lane && *p == abs_path_for_open && source.is_live()
    });

    // Renamed entries (`R` / `C` status) carry the original path —
    // surface it as `old → new` so the user can see what was renamed
    // without opening the diff. The destination basename is what
    // alphabetically sorts the row, so it stays first.
    let original_path = entry
        .staged
        .as_ref()
        .and_then(|e| e.original_path.clone())
        .or_else(|| {
            entry
                .unstaged
                .as_ref()
                .and_then(|e| e.original_path.clone())
        });
    let file_name = match original_path {
        Some(orig) => format!(
            "{} ← {}",
            entry.path.file_name_lossy(),
            orig.file_name_lossy()
        ),
        None => entry.path.file_name_lossy(),
    };

    // Single-char source-of-truth for the row's status: staged side
    // wins if both are populated (matches the unified list's display
    // priority for `MM` etc.).
    let status_char = if let Some(ref se) = entry.staged {
        se.x
    } else if let Some(ref ue) = entry.unstaged {
        ue.y
    } else {
        ' '
    };
    let status_color = if entry.staged.is_some() {
        git_status_color(status_char, true, cx)
    } else {
        git_status_color(status_char, false, cx)
    };
    let status_symbol = crate::workspace::left_dock::git_ops::git_status_symbol(status_char);

    // `git diff HEAD --numstat` cache for this file. Untracked / fresh
    // repos with no HEAD have no entry — render the row without a
    // diffstat tail in that case.
    let diffstat = snap
        .git_worktree_cache
        .get(&snap.active)
        .and_then(|s| s.diffstat.get(&entry.path))
        .copied();

    let workspace = snap.workspace.clone();
    let workspace_for_checkbox = snap.workspace.clone();
    let workspace_for_ctx = snap.workspace.clone();
    let in_flight = snap.git_stage_in_flight;

    // Snapshot every row chrome colour from the live theme.
    let t = theme::current(cx);
    let checkbox_checked_bg = t.git_stage_checkbox_checked_bg;
    let cursor_border_color = theme::PRIMARY;
    let row_selected_bg = t.git_file_row_selected_bg;
    let row_hover_bg = t.git_file_row_hover_bg;
    let filename_color = t.text_muted;
    let diff_add_color = t.file_diff_stat_add;
    let diff_del_color = t.file_diff_stat_del;

    let checkbox_id: ElementId = ("git-unified-cb", idx).into();
    let checkbox = div()
        .id(checkbox_id)
        .flex_none()
        .w(px(theme::CONTROL_TARGET_SIZE))
        .h(px(theme::CONTROL_TARGET_SIZE))
        .flex()
        .items_center()
        .justify_center()
        .text_color(if is_staged {
            checkbox_checked_bg
        } else {
            t.text_muted
        })
        .child(crate::ui::icons::icon(if is_staged {
            crate::ui::icons::CHECKBOX_ON
        } else {
            crate::ui::icons::CHECKBOX_OFF
        }))
        .when(!in_flight, |d| {
            d.cursor_pointer().on_mouse_down(
                MouseButton::Left,
                cx.listener(move |_dock, _: &MouseDownEvent, _window, cx| {
                    cx.stop_propagation();
                    let Some(ws) = workspace_for_checkbox.upgrade() else {
                        return;
                    };
                    ws.update(cx, |ws, cx| {
                        if is_staged {
                            ws.unstage_file(target, path_for_checkbox.clone(), cx);
                        } else {
                            ws.stage_file(target, path_for_checkbox.clone(), cx);
                        }
                    });
                }),
            )
        });

    div()
        .id(("git-unified", idx))
        .debug_selector(|| "git-changes-row".into())
        .flex()
        .flex_row()
        .items_center()
        // A `uniform_list` item does not inherit the cross-axis stretch the
        // old `flex_col` scroll area gave it, so the row has to claim the
        // width itself — without it `flex_1` below has no slack and the
        // right-aligned checkbox collapses back against the filename.
        .w_full()
        .h(px(theme::GIT_FILE_ROW_HEIGHT))
        .px(px(theme::GIT_FILE_ROW_PAD_X))
        .gap(px(theme::GIT_FILE_ROW_GAP))
        .cursor_pointer()
        .when(is_cursor, move |d| {
            d.border_l_2().border_color(cursor_border_color)
        })
        .when(is_selected, move |d| d.bg(row_selected_bg))
        .when(!is_selected, move |d| d.hover(move |d| d.bg(row_hover_bg)))
        // on_click fires only when mousedown + mouseup happen at the same
        // position (no drag past DRAG_THRESHOLD), so dragging the row to
        // drop its path elsewhere doesn't also open the diff view.
        .on_click(cx.listener(move |_dock, ev: &ClickEvent, window, cx| {
            if let Some(ws) = workspace.upgrade() {
                let click_count = ev.click_count();
                let cursor_path = path_for_cursor.clone();
                ws.update(cx, |ws, cx| {
                    ws.on_git_changes_row_click(
                        target,
                        cursor_path,
                        is_staged,
                        click_count,
                        window,
                        cx,
                    )
                });
            }
        }))
        // Status badge — single letter (M / A / D / R / ?) coloured by
        // stage state. Same shape as the file-viewer toolbar's status
        // badge (`file_viewer/render/toolbar.rs`) so the left dock and
        // toolbar share one visual vocabulary.
        .child(
            div()
                .flex_none()
                .w(px(theme::GIT_STATUS_CHAR_W))
                .text_size(px(theme::GIT_SECTION_FONT_SIZE))
                .text_color(status_color)
                .child(status_symbol),
        )
        // Filename + optional `+N −M` diffstat tail. The filename clips
        // first when the row is too narrow; the diffstat is `flex_none`
        // so it stays attached to the right edge of the filename
        // column. The `+N` and `−M` parts are coloured separately
        // (green / red) — same theme constants the file-viewer toolbar
        // uses for its diff stats.
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
                .when_some(
                    diffstat.filter(|(a, r)| *a > 0 || *r > 0),
                    |d, (added, removed)| {
                        d.child(
                            div()
                                .flex_none()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(theme::FILE_DIFF_STAT_GAP))
                                .text_size(px(theme::FILE_DIFF_STAT_FONT_SIZE))
                                .child(
                                    div()
                                        .flex_none()
                                        .text_color(diff_add_color)
                                        .child(format!("+{added}")),
                                )
                                .child(
                                    div()
                                        .flex_none()
                                        .text_color(diff_del_color)
                                        .child(format!("-{removed}")),
                                ),
                        )
                    },
                ),
        )
        // Checkbox right-aligned, mirroring SourceTree / GitHub Desktop.
        .child(checkbox)
        .on_drag(
            PathDrag {
                path: wt_paths.from_git_status(&entry.path),
                offset: gpui::Point::default(),
            },
            |drag, pos, _window, cx| {
                cx.new(|_| PathDrag {
                    path: drag.path.clone(),
                    offset: pos,
                })
            },
        )
        .root_context_menu(workspace_for_ctx.clone(), move |menu, _window, _cx| {
            let ws_stage = workspace_for_ctx.clone();
            let ws_discard = workspace_for_ctx.clone();
            let ws_diff = workspace_for_ctx.clone();
            let path_stage = path_for_ctx.clone();
            let path_discard = path_for_ctx.clone();
            let path_diff = abs_path_for_ctx_diff.clone();

            let menu = if is_staged {
                menu.item(
                    PopupMenuItem::new(app_strings::ctx::git_unstage()).on_click(
                        move |_, _, cx| {
                            if let Some(w) = ws_stage.upgrade() {
                                w.update(cx, |ws, cx| {
                                    ws.unstage_file(target, path_stage.clone(), cx)
                                });
                            }
                        },
                    ),
                )
            } else {
                menu.item(PopupMenuItem::new(app_strings::ctx::git_stage()).on_click(
                    move |_, _, cx| {
                        if let Some(w) = ws_stage.upgrade() {
                            w.update(cx, |ws, cx| ws.stage_file(target, path_stage.clone(), cx));
                        }
                    },
                ))
            };

            let menu = menu.separator().item(
                PopupMenuItem::new(app_strings::ctx::git_open_diff()).on_click(
                    move |_, window, cx| {
                        if let Some(w) = ws_diff.upgrade() {
                            w.update(cx, |ws, cx| {
                                // The context menu is a deliberate pick, so
                                // the tab it opens is not a skim's to reuse.
                                ws.open_git_file_diff(
                                    target,
                                    path_diff.clone(),
                                    DiffSource::from_staged(is_staged),
                                    OpenIntent::Commit,
                                    window,
                                    cx,
                                )
                            });
                        }
                    },
                ),
            );

            menu.separator().item(
                PopupMenuItem::new(app_strings::ctx::git_discard()).on_click(
                    move |_, window, cx| {
                        if let Some(w) = ws_discard.upgrade() {
                            w.update(cx, |ws, cx| {
                                ws.on_discard_file(target, path_discard.clone(), window, cx)
                            });
                        }
                    },
                ),
            )
        })
        .into_any_element()
}
