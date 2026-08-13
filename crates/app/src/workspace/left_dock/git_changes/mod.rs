//! Git Changes dock view — unified file list with commit / push controls.
//!
//! All changed files (staged and unstaged) appear in a single sorted list.
//! Each row has a checkbox: checked = staged, unchecked = unstaged.
//! Checking/unchecking stages or unstages the file without changing its
//! position in the list.

mod unified_list;

use std::path::PathBuf;

use crate::ui::theme;
use daruda_store::project::LaneId;
use gpui::{
    AnyElement, ClickEvent, Context, ElementId, IntoElement, MouseButton, MouseDownEvent, div,
    prelude::*, px,
};

use crate::lane::paths::LanePaths;
use crate::path_ext::PathExt;
use crate::surface::strings as app_strings;
use crate::ui::{
    ButtonVariants as _, ContextMenuExt as _, Icon, IconName, PopupMenuItem, SectionHeader,
    Sizable as _, button, button_bare, menu_builder,
};
use crate::workspace::layout::Dock;
use crate::workspace::layout::LeftDockSnapshot;
use crate::workspace::left_dock::git_ops::git_status_color;
use crate::workspace::path_drag::PathDrag;
use unified_list::{
    DirStageState, UnifiedEntry, build_unified_list, compute_dir_state, count_conflicts,
    discard_disabled, group_by_dir, tracking_indicator_text,
};

pub(in crate::workspace) use unified_list::ordered_visible_paths;

// ----------------------------------------------------------------
// Entry point
// ----------------------------------------------------------------

pub(in crate::workspace) fn render(snap: &LeftDockSnapshot, cx: &mut Context<Dock>) -> AnyElement {
    let active_id = snap.active.lane;
    let active_wt = snap.lanes.iter().find(|w| w.id == active_id);

    if !active_wt.map(|w| w.is_git()).unwrap_or(false) {
        return non_git_placeholder(active_id, snap, cx).into_any_element();
    }

    let branch = active_wt
        .and_then(|wt| match &wt.kind {
            daruda_store::project::LaneKind::Git { branch, .. } => branch.clone(),
            daruda_store::project::LaneKind::Default => None,
        })
        .unwrap_or_else(|| app_strings::git_detached_label().to_string());

    let status = snap.git_status_cache.get(&snap.active);
    let stage_in_flight = snap.git_stage_in_flight;

    let selected: Option<(LaneId, PathBuf, bool)> = snap.focused_file_selection.clone();

    // `key_context("GitChanges")` + `track_focus(...)` route arrow / Space /
    // Enter to GitChangesSelectNext / Prev / ToggleStage / Activate only
    // when this panel holds focus, so terminal panes still see those keys
    // by default.
    let panel_focus = snap.git_changes_panel_focus.clone();
    let mut body = crate::workspace::left_dock::left_panel_body()
        .key_context("GitChanges")
        .track_focus(&panel_focus);

    body = body.child(view_header(active_id, &branch, snap, cx));

    match status {
        None => {
            body = body.child(loading_placeholder(active_id, snap, cx));
        }
        Some(s) if s.staged.is_empty() && s.unstaged.is_empty() => {
            body = body.child(clean_placeholder(cx));
        }
        Some(s) => {
            // git status --porcelain paths are repo-root-relative. LanePaths
            // resolves them to absolute and back to wt-relative for display/git ops.
            // Safe to unwrap: is_git() was checked at the top of this function.
            let wt_paths = active_wt.unwrap().paths();

            // Conflict banner — fixed above the scroll area so it stays
            // visible regardless of where the user is in the file list.
            // Conflict entries land in `unstaged` (parse_git_status_output
            // routes UU/AA/DD there), so counting that side is enough.
            let conflict_count = count_conflicts(&s.unstaged);
            if conflict_count > 0 {
                body = body.child(conflict_banner(conflict_count));
            }

            let unified = build_unified_list(&s.staged, &s.unstaged);
            let staged_count = s.staged.len();
            let unstaged_count = s.unstaged.len();

            let scroll_handle = snap.git_changes_scroll_handle.clone();
            let mut scroll_area = div()
                .id("git-changes-scroll")
                .flex()
                .flex_col()
                .size_full()
                .overflow_y_scroll()
                .track_scroll(&scroll_handle);

            scroll_area = scroll_area.child(summary_bar(
                staged_count,
                unstaged_count,
                stage_in_flight,
                active_id,
                snap,
                cx,
            ));

            // Group entries by parent directory for per-dir stage headers.
            let groups = group_by_dir(unified, &wt_paths);
            let mut flat_idx = 0usize;
            for (dir_idx, (dir_opt, entries)) in groups.iter().enumerate() {
                let is_collapsed = dir_opt
                    .as_deref()
                    .map(|d| snap.git_collapsed_dirs.contains(d))
                    .unwrap_or(false);
                if let Some(d) = dir_opt.as_deref() {
                    let dir_state = compute_dir_state(entries);
                    scroll_area = scroll_area.child(dir_header(
                        dir_idx,
                        d,
                        is_collapsed,
                        dir_state,
                        entries,
                        active_id,
                        &wt_paths,
                        snap,
                        cx,
                    ));
                }
                if is_collapsed {
                    // Skip rendering the dir's rows but keep flat_idx in
                    // sync so future expands re-use the same row ids.
                    flat_idx += entries.len();
                    continue;
                }
                for entry in entries {
                    let is_cursor = snap
                        .git_changes_cursor
                        .as_ref()
                        .map(|c| c == &entry.path)
                        .unwrap_or(false);
                    scroll_area = scroll_area.child(unified_file_row(
                        flat_idx,
                        entry,
                        active_id,
                        &wt_paths,
                        selected.as_ref(),
                        is_cursor,
                        snap,
                        cx,
                    ));
                    flat_idx += 1;
                }
            }

            let scroll_handle_bar = snap.git_changes_scroll_handle.clone();
            body = body.child(
                div()
                    .flex_1()
                    .relative()
                    .overflow_hidden()
                    .child(scroll_area)
                    .children(git_changes_scrollbar(&scroll_handle_bar, cx)),
            );

            body = body.child(commit_footer(snap, cx));
        }
    }

    body.into_any_element()
}

// ----------------------------------------------------------------
// Header — branch label + remote action buttons
// ----------------------------------------------------------------

fn view_header(
    _lane_id: LaneId,
    branch: &str,
    snap: &LeftDockSnapshot,
    cx: &mut Context<Dock>,
) -> impl IntoElement {
    use crate::ui::Disableable as _;

    let label = app_strings::git_changes_header(branch);
    let workspace_refresh = snap.workspace.clone();
    let workspace_fetch = snap.workspace.clone();
    let workspace_push = snap.workspace.clone();
    let in_flight = snap.git_op_in_flight;
    let active_ref = snap.active;

    let (ahead, behind) = snap
        .git_status_cache
        .get(&snap.active)
        .map(|s| (s.ahead, s.behind))
        .unwrap_or((0, 0));

    let refresh_icon = button_bare("git-refresh")
        .xsmall()
        .ghost()
        .icon(IconName::Refresh)
        .on_click(cx.listener(move |_dock, _: &ClickEvent, _window, cx| {
            if let Some(ws) = workspace_refresh.upgrade() {
                ws.update(cx, |ws, cx| ws.refresh_git_status(active_ref, cx));
            }
        }));

    let tracking_color = theme::current(cx).text_muted;
    let mut header_actions = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::GIT_REMOTE_BTN_GAP));
    if let Some(text) = tracking_indicator_text(ahead, behind) {
        header_actions = header_actions.child(
            div()
                .text_size(px(theme::GIT_DIR_HEADER_FONT_SIZE))
                .text_color(tracking_color)
                .child(text),
        );
    }
    let header_actions = header_actions.child(refresh_icon);

    let fetch_btn = button("git-fetch", app_strings::git_fetch_btn())
        .xsmall()
        .loading(in_flight)
        .disabled(in_flight)
        .on_click(cx.listener(move |_dock, _: &ClickEvent, _window, cx| {
            if let Some(ws) = workspace_fetch.upgrade() {
                ws.update(cx, |ws, cx| ws.on_fetch(cx));
            }
        }));

    let push_btn = button("git-push", app_strings::git_push_btn())
        .xsmall()
        .loading(in_flight)
        .disabled(in_flight)
        .on_click(cx.listener(move |_dock, _: &ClickEvent, window, cx| {
            if let Some(ws) = workspace_push.upgrade() {
                ws.update(cx, |ws, cx| ws.trigger_push(window, cx));
            }
        }));

    let actions_row = div()
        .flex()
        .flex_row()
        .items_center()
        .justify_end()
        .gap(px(theme::GIT_REMOTE_BTN_GAP))
        .pt(px(theme::GIT_HEADER_PAD_Y / 2.0))
        .child(fetch_btn)
        .child(push_btn);

    div()
        .flex()
        .flex_col()
        .px(px(theme::GIT_HEADER_PAD_X))
        .pt(px(theme::GIT_HEADER_PAD_Y))
        .pb(px(theme::GIT_HEADER_PAD_Y / 2.0))
        .child(
            SectionHeader::new(label)
                .truncate_label(true)
                .actions(header_actions),
        )
        .child(actions_row)
}

// ----------------------------------------------------------------
// Summary bar — file counts + Stage All / Unstage All toggle
// ----------------------------------------------------------------

fn summary_bar(
    staged_count: usize,
    unstaged_count: usize,
    in_flight: bool,
    lane_id: LaneId,
    snap: &LeftDockSnapshot,
    cx: &mut Context<Dock>,
) -> impl IntoElement {
    let workspace = snap.workspace.clone();
    let t = theme::current(cx);
    let summary_text_color = t.text_muted;
    let toggle_inflight = t.text_subtle;
    let toggle_idle = t.text_muted;
    let toggle_hover = t.text_primary;

    let label = match (staged_count, unstaged_count) {
        (s, 0) => format!("{s} staged"),
        (0, u) => format!("{u} unstaged"),
        (s, u) => format!("{s} staged, {u} unstaged"),
    };

    // Single-toggle action: when every change is already staged, the
    // affordance is "Unstage All"; otherwise "Stage All". One label at
    // a time keeps the bar visually quiet — partial-staged states still
    // surface "Stage All" so the user has a one-click way to finish.
    // (Reaching "Unstage All" from a partial state requires individual
    // unchecks first; that's an accepted trade-off for the cleaner
    // single-button look.)
    let all_staged = unstaged_count == 0 && staged_count > 0;
    let btn_label = if all_staged {
        app_strings::git_unstage_all()
    } else {
        app_strings::git_stage_all()
    };

    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .px(px(theme::GIT_HEADER_PAD_X))
        .py(px(theme::LANE_SECTION_PAD_Y))
        .text_size(px(theme::GIT_SECTION_FONT_SIZE))
        .text_color(summary_text_color)
        .child(label)
        .child(
            div()
                .id("git-stage-toggle")
                .text_color(if in_flight {
                    toggle_inflight
                } else {
                    toggle_idle
                })
                .when(!in_flight, move |d| {
                    d.cursor_pointer()
                        .hover(move |d| d.text_color(toggle_hover))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |_dock, _: &MouseDownEvent, _window, cx| {
                                let Some(ws) = workspace.upgrade() else {
                                    return;
                                };
                                if all_staged {
                                    ws.update(cx, |ws, cx| ws.unstage_all(lane_id, cx));
                                } else {
                                    ws.update(cx, |ws, cx| ws.stage_all(lane_id, cx));
                                }
                            }),
                        )
                })
                .child(btn_label),
        )
}

// ----------------------------------------------------------------
// Conflict banner
// ----------------------------------------------------------------

fn conflict_banner(count: usize) -> impl IntoElement {
    let msg = if count == 1 {
        app_strings::git_conflict_banner_single().to_string()
    } else {
        format!("{count} conflicts — resolve before committing.")
    };
    div()
        .px(px(theme::GIT_HEADER_PAD_X))
        .pb(px(theme::GIT_HEADER_PAD_Y / 2.0))
        .child(crate::ui::alert::warning("git-conflict-banner", msg))
}

// ----------------------------------------------------------------
// Directory group header — chevron, dir name, per-dir stage checkbox
// ----------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn dir_header(
    dir_idx: usize,
    dir: &str,
    is_collapsed: bool,
    state: DirStageState,
    entries: &[UnifiedEntry],
    lane_id: LaneId,
    wt_paths: &LanePaths<'_>,
    snap: &LeftDockSnapshot,
    cx: &mut Context<Dock>,
) -> AnyElement {
    let workspace_toggle = snap.workspace.clone();
    let workspace_stage = snap.workspace.clone();
    let dir_owned = dir.to_string();
    let dir_for_toggle = dir_owned.clone();
    let in_flight = snap.git_stage_in_flight;

    let t = theme::current(cx);
    let checkbox_border = t.border;
    let checkbox_checked_bg = t.git_stage_checkbox_checked_bg;
    let checkbox_unchecked_bg = t.git_stage_checkbox_unchecked_bg;
    // Checkmark sits on the accent fill → accent-fg for contrast; the
    // indeterminate dash sits on the unchecked surface → theme foreground.
    let checkbox_tick_color = theme::ACCENT_FG;
    let dash_color = t.text_primary;
    let dir_label_color = t.text_subtle;
    let dir_label_hover = t.text_muted;

    let chevron_icon = if is_collapsed {
        IconName::ChevronRight
    } else {
        IconName::ChevronDown
    };

    // Paths to operate on — always the git-status form (repo-root
    // relative) so they round-trip into git_add / git_restore_staged.
    let stage_paths: Vec<PathBuf> = match state {
        DirStageState::AllStaged => entries
            .iter()
            .filter(|e| e.staged.is_some())
            .map(|e| e.path.clone())
            .collect(),
        DirStageState::NoneStaged | DirStageState::Mixed => entries
            .iter()
            .filter(|e| e.unstaged.is_some())
            .map(|e| e.path.clone())
            .collect(),
    };

    let checkbox_id: ElementId = ("git-dir-checkbox", dir_idx).into();
    let checkbox = div()
        .id(checkbox_id)
        .flex_none()
        .w(px(theme::GIT_STAGE_CHECKBOX_SIZE))
        .h(px(theme::GIT_STAGE_CHECKBOX_SIZE))
        .rounded(px(theme::GIT_STAGE_CHECKBOX_RADIUS))
        .border_1()
        // Checked → border matches the accent fill (seamless, like the
        // gpui_component checkbox); otherwise a hairline rim.
        .border_color(match state {
            DirStageState::AllStaged => checkbox_checked_bg,
            DirStageState::Mixed | DirStageState::NoneStaged => checkbox_border,
        })
        .bg(match state {
            DirStageState::AllStaged => checkbox_checked_bg,
            DirStageState::Mixed | DirStageState::NoneStaged => checkbox_unchecked_bg,
        })
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(theme::GIT_STAGE_CHECKBOX_TICK_SIZE))
        .when(state == DirStageState::AllStaged, |d| {
            d.text_color(checkbox_tick_color)
                .child(app_strings::UI_CHECKMARK)
        })
        .when(state == DirStageState::Mixed, |d| {
            // Indeterminate: dash glyph at the same size as the tick.
            d.text_color(dash_color).child("–")
        })
        .when(!in_flight && !stage_paths.is_empty(), |d| {
            d.cursor_pointer().on_mouse_down(
                MouseButton::Left,
                cx.listener(move |_dock, _: &MouseDownEvent, _window, cx| {
                    cx.stop_propagation();
                    let Some(ws) = workspace_stage.upgrade() else {
                        return;
                    };
                    let paths = stage_paths.clone();
                    ws.update(cx, |ws, cx| match state {
                        DirStageState::AllStaged => ws.unstage_paths(lane_id, paths, cx),
                        DirStageState::NoneStaged | DirStageState::Mixed => {
                            ws.stage_paths(lane_id, paths, cx)
                        }
                    });
                }),
            )
        });

    // Touching wt_paths inside the closure would borrow it across an
    // FnMut boundary; the dir name alone is enough to look up the
    // collapse set on the workspace side.
    let _ = wt_paths;

    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::GIT_FILE_ROW_GAP))
        .px(px(theme::GIT_HEADER_PAD_X))
        .py(px(theme::GIT_DIR_HEADER_PAD_Y))
        .text_size(px(theme::GIT_DIR_HEADER_FONT_SIZE))
        .text_color(dir_label_color)
        // Chevron + dir name on the left (clickable to toggle collapse),
        // checkbox right-aligned to mirror the file rows below.
        .child(
            div()
                .id(("git-dir-toggle", dir_idx))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(theme::GIT_FILE_ROW_GAP))
                .flex_1()
                .overflow_hidden()
                .cursor_pointer()
                .hover(move |d| d.text_color(dir_label_hover))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |_dock, _: &MouseDownEvent, _window, cx| {
                        if let Some(ws) = workspace_toggle.upgrade() {
                            let dir = dir_for_toggle.clone();
                            ws.update(cx, |ws, cx| ws.toggle_git_dir_collapse(lane_id, dir, cx));
                        }
                    }),
                )
                .child(Icon::new(chevron_icon).xsmall().text_color(dir_label_color))
                .child(
                    div()
                        .flex_1()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .child(dir.to_string()),
                ),
        )
        .child(checkbox)
        .into_any_element()
}

// ----------------------------------------------------------------
// Unified file row
// ----------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn unified_file_row(
    idx: usize,
    entry: &UnifiedEntry,
    lane_id: LaneId,
    wt_paths: &LanePaths<'_>,
    selected: Option<&(LaneId, PathBuf, bool)>,
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
    let has_unstaged = entry.unstaged.is_some();
    let is_untracked = entry.unstaged.as_ref().is_some_and(|u| u.x == '?');

    let is_selected = selected
        .map(|(wt, p, _s)| *wt == lane_id && *p == abs_path_for_open)
        .unwrap_or(false);

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
        .git_status_cache
        .get(&snap.active)
        .and_then(|s| s.diffstat.get(&entry.path))
        .copied();

    let workspace = snap.workspace.clone();
    let workspace_for_checkbox = snap.workspace.clone();
    let workspace_for_ctx = snap.workspace.clone();
    let panel_focus = snap.git_changes_panel_focus.clone();
    let in_flight = snap.git_stage_in_flight;

    // Snapshot every row chrome colour from the live theme.
    let t = theme::current(cx);
    let checkbox_border = t.border;
    let checkbox_checked_bg = t.git_stage_checkbox_checked_bg;
    let checkbox_unchecked_bg = t.git_stage_checkbox_unchecked_bg;
    // Tick only renders on the checked (accent) fill → accent-fg.
    let checkbox_tick_color = theme::ACCENT_FG;
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
        .w(px(theme::GIT_STAGE_CHECKBOX_SIZE))
        .h(px(theme::GIT_STAGE_CHECKBOX_SIZE))
        .rounded(px(theme::GIT_STAGE_CHECKBOX_RADIUS))
        .border_1()
        // Staged → border matches the accent fill (seamless, like the
        // gpui_component checkbox); otherwise a hairline rim.
        .border_color(if is_staged {
            checkbox_checked_bg
        } else {
            checkbox_border
        })
        .bg(if is_staged {
            checkbox_checked_bg
        } else {
            checkbox_unchecked_bg
        })
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(theme::GIT_STAGE_CHECKBOX_TICK_SIZE))
        .when(is_staged, |d| {
            d.text_color(checkbox_tick_color)
                .child(app_strings::UI_CHECKMARK)
        })
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
                            ws.unstage_file(lane_id, path_for_checkbox.clone(), cx);
                        } else {
                            ws.stage_file(lane_id, path_for_checkbox.clone(), cx);
                        }
                    });
                }),
            )
        });

    div()
        .id(("git-unified", idx))
        .flex()
        .flex_row()
        .items_center()
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
                let fh = panel_focus.clone();
                let click_count = ev.click_count();
                let abs_path = abs_path_for_open.clone();
                let cursor_path = path_for_cursor.clone();
                ws.update(cx, |ws, cx| {
                    if click_count >= 2 {
                        ws.open_file_externally(lane_id, abs_path, cx);
                    } else {
                        ws.set_git_changes_cursor(lane_id, cursor_path, cx);
                        ws.open_git_file_diff(lane_id, abs_path, is_staged, window, cx);
                        fh.focus(window, cx);
                    }
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
        .context_menu(menu_builder(move |menu, _window, _cx| {
            let ws_stage = workspace_for_ctx.clone();
            let ws_discard = workspace_for_ctx.clone();
            let ws_diff = workspace_for_ctx.clone();
            let path_stage = path_for_ctx.clone();
            let path_discard = path_for_ctx.clone();
            let path_diff = abs_path_for_ctx_diff.clone();

            let menu = if is_staged {
                menu.item(PopupMenuItem::new(app_strings::ctx_git_unstage()).on_click(
                    move |_, _, cx| {
                        if let Some(w) = ws_stage.upgrade() {
                            w.update(cx, |ws, cx| {
                                ws.unstage_file(lane_id, path_stage.clone(), cx)
                            });
                        }
                    },
                ))
            } else {
                menu.item(PopupMenuItem::new(app_strings::ctx_git_stage()).on_click(
                    move |_, _, cx| {
                        if let Some(w) = ws_stage.upgrade() {
                            w.update(cx, |ws, cx| ws.stage_file(lane_id, path_stage.clone(), cx));
                        }
                    },
                ))
            };

            let menu = menu.separator().item(
                PopupMenuItem::new(app_strings::ctx_git_open_diff()).on_click(
                    move |_, window, cx| {
                        if let Some(w) = ws_diff.upgrade() {
                            w.update(cx, |ws, cx| {
                                ws.open_git_file_diff(
                                    lane_id,
                                    path_diff.clone(),
                                    is_staged,
                                    window,
                                    cx,
                                )
                            });
                        }
                    },
                ),
            );

            menu.separator().item(
                PopupMenuItem::new(app_strings::ctx_git_discard())
                    .on_click(move |_, window, cx| {
                        if let Some(w) = ws_discard.upgrade() {
                            w.update(cx, |ws, cx| {
                                ws.on_discard_file(
                                    lane_id,
                                    path_discard.clone(),
                                    is_untracked,
                                    window,
                                    cx,
                                )
                            });
                        }
                    })
                    // Discard is dangerous — disable only when the file has no
                    // working-tree changes to discard (purely staged, like `M `).
                    // For `MM` (staged + unstaged) and untracked, leave it enabled.
                    .disabled(discard_disabled(is_staged, has_unstaged)),
            )
        }))
        .into_any_element()
}

// ----------------------------------------------------------------
// Commit footer — InputPanel (TextArea + Commit / Push buttons)
// ----------------------------------------------------------------

fn commit_footer(snap: &LeftDockSnapshot, cx: &mut Context<Dock>) -> impl IntoElement {
    let border = theme::current(cx).border;
    div()
        .flex()
        .flex_col()
        .flex_none()
        .h(px(theme::GIT_COMMIT_FOOTER_H))
        .border_t_1()
        .border_color(border)
        .child(snap.git_commit_input.clone())
}

// ----------------------------------------------------------------
// Scrollbar overlay
// ----------------------------------------------------------------

fn git_changes_scrollbar(
    handle: &gpui::ScrollHandle,
    cx: &gpui::App,
) -> Option<crate::ui::scrollbar::Thumb> {
    let viewport_h = handle.bounds().size.height;
    let max_offset = handle.max_offset().y;
    let t = theme::current(cx);
    crate::ui::scrollbar::vertical_thumb(
        "git-changes-scrollbar-thumb",
        viewport_h,
        viewport_h + max_offset,
        handle.offset().y,
        px(0.),
        t.scrollbar_thumb,
        t.dock_scrollbar_thumb_hover,
    )
}

// ----------------------------------------------------------------
// Placeholder states
// ----------------------------------------------------------------

fn loading_placeholder(
    _lane_id: LaneId,
    snap: &LeftDockSnapshot,
    cx: &mut Context<Dock>,
) -> impl IntoElement {
    let text_color = theme::current(cx).text_subtle;
    let workspace = snap.workspace.clone();
    let active_ref = snap.active;
    let refresh_btn = button("git-refresh-fallback", app_strings::git_refresh_btn()).on_click(
        cx.listener(move |_dock, _: &ClickEvent, _window, cx| {
            if let Some(ws) = workspace.upgrade() {
                ws.update(cx, |ws, cx| ws.refresh_git_status(active_ref, cx));
            }
        }),
    );
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(theme::GIT_COMMIT_PAD))
        .p(px(theme::LANE_PLACEHOLDER_PAD))
        .text_size(px(theme::LANE_SUB_FONT_SIZE))
        .text_color(text_color)
        .child(crate::ui::placeholder_text(
            app_strings::git_loading_changes(),
        ))
        .child(refresh_btn)
}

fn clean_placeholder(cx: &gpui::App) -> impl IntoElement {
    div()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(theme::DOCK_PLACEHOLDER_FONT_SIZE))
        .text_color(theme::current(cx).text_subtle)
        .child(crate::ui::placeholder_text(app_strings::git_no_changes()))
}

fn non_git_placeholder(
    lane_id: LaneId,
    snap: &LeftDockSnapshot,
    cx: &mut Context<Dock>,
) -> impl IntoElement {
    let workspace = snap.workspace.clone();
    let in_flight = snap.git_op_in_flight;

    let init_btn = button("git-init", app_strings::git_init_btn()).on_click(cx.listener(
        move |_dock, _: &ClickEvent, _window, cx| {
            if let Some(ws) = workspace.upgrade() {
                ws.update(cx, |ws, cx| ws.init_git_repo(lane_id, cx));
            }
        },
    ));
    use crate::ui::Disableable as _;
    let init_btn = init_btn.disabled(in_flight).loading(in_flight);

    let text_color = theme::current(cx).text_subtle;
    div()
        .flex_1()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(theme::GIT_COMMIT_PAD))
        .text_size(px(theme::DOCK_PLACEHOLDER_FONT_SIZE))
        .text_color(text_color)
        .child(crate::ui::placeholder_text(
            app_strings::git_not_a_repository(),
        ))
        .child(init_btn)
}
