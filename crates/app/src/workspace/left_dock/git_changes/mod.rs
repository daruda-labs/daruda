//! Git Changes dock view — unified file list with commit / push controls.
//!
//! All changed files (staged and unstaged) appear in a single sorted list.
//! Each row has a checkbox: checked = staged, unchecked = unstaged.
//! Checking/unchecking stages or unstages the file without changing its
//! position in the list.

mod against_base_section;
mod file_row;
pub(super) mod unified_list;

use std::path::PathBuf;

use crate::ui::theme;
use daruda_store::project::LaneId;
use gpui::{
    AnyElement, ClickEvent, Context, ElementId, IntoElement, MouseButton, MouseDownEvent, div,
    prelude::*, px, uniform_list,
};

use crate::surface::strings as app_strings;
use crate::ui::{SectionHeader, Sizable as _, button};
use crate::workspace::layout::Dock;
use crate::workspace::layout::LeftDockSnapshot;
use unified_list::{
    DirStageState, GitChangesRow, GitDirHeaderRow, count_conflicts, tracking_indicator_text,
};

pub(in crate::workspace) use unified_list::visible_file_rows;

// ----------------------------------------------------------------
// Entry point
// ----------------------------------------------------------------

#[cfg(test)]
thread_local! {
    /// Row elements actually built, across all renders. `uniform_list` asks
    /// only for the range it will paint, so this must stay bounded by the
    /// viewport no matter how large the change set is.
    pub(in crate::workspace) static ROWS_BUILT: std::cell::Cell<usize> =
        const { std::cell::Cell::new(0) };
}

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

    let status = snap.git_worktree_cache.get(&snap.active);
    let stage_in_flight = snap.git_stage_in_flight;

    // `key_context("GitChanges")` + `track_focus(...)` route arrow / Space /
    // Enter to GitChangesSelectNext / Prev / ToggleStage / Activate only
    // when this panel holds focus, so terminal panes still see those keys
    // by default.
    let panel_focus = snap.git_changes_panel_focus.clone();
    let mut body = crate::workspace::left_dock::left_panel_body()
        .key_context("GitChanges")
        .track_focus(&panel_focus);

    body = body.child(view_header(active_id, &branch, snap, cx));

    let base_rows = unified_list::base_rows(
        snap.git_against_base.as_deref(),
        snap.git_against_base_collapsed,
    );

    match status {
        None => {
            body = body.child(loading_placeholder(active_id, snap, cx));
        }
        // A clean working tree still lists what the lane committed.
        Some(s) if s.staged.is_empty() && s.unstaged.is_empty() && base_rows.is_empty() => {
            body = body.child(clean_placeholder(cx));
        }
        Some(s) => {
            let staged_count = s.staged.len();
            let unstaged_count = s.unstaged.len();

            // Fixed above the list rather than scrolling with it (zed's git
            // panel does the same): the list is virtualized now, and a
            // uniform-height list cannot carry a taller widget as row 0. The
            // stage counts staying put is the point of the trade.
            body = body.child(summary_bar(
                staged_count,
                unstaged_count,
                stage_in_flight,
                active_id,
                snap,
                cx,
            ));

            // Conflict banner — fixed above the scroll area so it stays
            // visible regardless of where the user is in the file list.
            // Conflict entries land in `unstaged` (parse_git_status_output
            // routes UU/AA/DD there), so counting that side is enough.
            let conflict_count = count_conflicts(&s.unstaged);
            if conflict_count > 0 {
                body = body.child(conflict_banner(conflict_count));
            }

            // Flatten here, not in `prepare_left_dock_snapshot`: this render
            // runs only when the dock is actually dirty, while the snapshot is
            // staged on every workspace render. The flattening is ~0.2 ms per
            // 1000 files, so where it runs matters more than what it costs.
            // The list closure outlives this body, so the rows are shared into
            // it rather than borrowed.
            // Safe to unwrap: `is_git()` was checked at the top of this fn.
            let wt_paths = active_wt.unwrap().paths();
            let mut rows = unified_list::build_rows(s, &snap.git_collapsed_dirs, &wt_paths);
            rows.extend(base_rows);
            let rows = std::rc::Rc::new(rows);
            let count = rows.len();
            let rows_for_list = rows.clone();
            let scroll_handle = snap.git_changes_scroll_handle.clone();
            let list = uniform_list(
                "git-changes-rows",
                count,
                cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                    let crate::workspace::layout::DockSnapshot::Left(snap) = &this.snap else {
                        return Vec::new();
                    };
                    let snap = snap.as_ref();
                    let active_id = snap.active.lane;
                    let Some(wt) = snap.lanes.iter().find(|w| w.id == active_id) else {
                        return Vec::new();
                    };
                    let wt_paths = wt.paths();
                    let selected = snap.focused_file_selection.clone();
                    range
                        .filter_map(|ix| {
                            #[cfg(test)]
                            ROWS_BUILT.with(|n| n.set(n.get() + 1));
                            let row = rows_for_list.get(ix)?;
                            Some(match row {
                                GitChangesRow::DirHeader(dir) => {
                                    dir_header(ix, dir, active_id, snap, cx)
                                }
                                GitChangesRow::File(entry) => {
                                    let is_cursor = snap
                                        .git_changes_cursor
                                        .as_ref()
                                        .is_some_and(|c| c == &entry.path);
                                    file_row::unified_file_row(
                                        ix,
                                        entry,
                                        active_id,
                                        &wt_paths,
                                        selected.as_ref(),
                                        is_cursor,
                                        snap,
                                        cx,
                                    )
                                }
                                GitChangesRow::BaseHeader(header) => {
                                    against_base_section::base_header(ix, header, snap, cx)
                                }
                                GitChangesRow::BaseFile(file) => {
                                    against_base_section::base_file_row(ix, file, snap, cx)
                                }
                            })
                        })
                        .collect()
                }),
            )
            .track_scroll(&scroll_handle)
            .size_full();

            let scroll_handle_bar = scroll_handle.clone();
            body = body.child(
                div()
                    .id("git-changes-scroll")
                    .flex_1()
                    .relative()
                    .overflow_hidden()
                    .child(list)
                    .children(git_changes_scrollbar(&scroll_handle_bar, count, cx)),
            );

            body = body.child(commit_footer(snap, cx));
        }
    }

    body.into_any_element()
}

// ----------------------------------------------------------------
// Header — branch label + remote action buttons
// ----------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RemotePrimaryAction {
    Fetch,
    Pull,
}

fn remote_primary_action(behind: u32) -> RemotePrimaryAction {
    if behind > 0 {
        RemotePrimaryAction::Pull
    } else {
        RemotePrimaryAction::Fetch
    }
}

fn view_header(
    _lane_id: LaneId,
    branch: &str,
    snap: &LeftDockSnapshot,
    cx: &mut Context<Dock>,
) -> impl IntoElement {
    use crate::ui::Disableable as _;

    let label = app_strings::git_changes_header(branch);
    let workspace_refresh = snap.workspace.clone();
    let workspace_remote = snap.workspace.clone();
    let workspace_push = snap.workspace.clone();
    let in_flight = snap.git_op_in_flight;
    let active_ref = snap.active;

    let (ahead, behind) = snap
        .git_tracking_cache
        .get(&snap.active)
        .map(|t| (t.ahead, t.behind))
        .unwrap_or((0, 0));

    let refresh_icon = crate::ui::button_icon("git-refresh", crate::ui::icons::REFRESH, cx)
        .tooltip(app_strings::usage_refresh())
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

    let remote_btn = match remote_primary_action(behind) {
        RemotePrimaryAction::Fetch => button("git-fetch", app_strings::git_fetch_btn())
            .xsmall()
            .loading(in_flight)
            .disabled(in_flight)
            .on_click(cx.listener(move |_dock, _: &ClickEvent, _window, cx| {
                if let Some(ws) = workspace_remote.upgrade() {
                    ws.update(cx, |ws, cx| ws.on_fetch(cx));
                }
            })),
        RemotePrimaryAction::Pull => button("git-pull", app_strings::git_pull_btn())
            .xsmall()
            .loading(in_flight)
            .disabled(in_flight)
            .on_click(cx.listener(move |_dock, _: &ClickEvent, _window, cx| {
                if let Some(ws) = workspace_remote.upgrade() {
                    ws.update(cx, |ws, cx| ws.on_pull(cx));
                }
            })),
    };

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
        .child(remote_btn)
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
    row: &GitDirHeaderRow,
    lane_id: LaneId,
    snap: &LeftDockSnapshot,
    cx: &mut Context<Dock>,
) -> AnyElement {
    let workspace_toggle = snap.workspace.clone();
    let workspace_stage = snap.workspace.clone();
    let dir = row.dir.as_str();
    let is_collapsed = row.collapsed;
    let state = row.state;
    // Already narrowed to the side a click moves; see `GitDirHeaderRow`.
    let stage_paths: Vec<PathBuf> = row.stage_paths.clone();
    let dir_owned = dir.to_string();
    let dir_for_toggle = dir_owned.clone();
    let in_flight = snap.git_stage_in_flight;

    let t = theme::current(cx);
    let checkbox_checked_bg = t.git_stage_checkbox_checked_bg;
    let dir_label_color = t.text_subtle;
    let dir_label_hover = t.text_muted;

    let chevron_icon = if is_collapsed {
        crate::ui::icons::CHEVRON_RIGHT
    } else {
        crate::ui::icons::EXPAND_MORE
    };

    let checkbox_id: ElementId = ("git-dir-checkbox", dir_idx).into();
    let checkbox = div()
        .id(checkbox_id)
        .flex_none()
        .w(px(theme::CONTROL_TARGET_SIZE))
        .h(px(theme::CONTROL_TARGET_SIZE))
        .flex()
        .items_center()
        .justify_center()
        .text_color(if state == DirStageState::NoneStaged {
            t.text_muted
        } else {
            checkbox_checked_bg
        })
        .child(crate::ui::icons::icon(match state {
            DirStageState::AllStaged => crate::ui::icons::CHECKBOX_ON,
            DirStageState::Mixed => crate::ui::icons::CHECKBOX_MIXED,
            DirStageState::NoneStaged => crate::ui::icons::CHECKBOX_OFF,
        }))
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

    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::GIT_FILE_ROW_GAP))
        .px(px(theme::GIT_HEADER_PAD_X))
        // Claims its width for the same reason the file row does.
        .w_full()
        // Same height as a file row: `uniform_list` measures one item and
        // applies it to every row, so a header that sized itself from its
        // padding would clip or leave a gap. The label keeps its own smaller
        // font — only the box grows (this is how zed's git panel does it).
        .h(px(theme::GIT_FILE_ROW_HEIGHT))
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
                .child(crate::ui::icons::icon(chevron_icon).text_color(dir_label_color))
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
    handle: &gpui::UniformListScrollHandle,
    item_count: usize,
    cx: &gpui::App,
) -> Option<crate::ui::scrollbar::Thumb> {
    if item_count == 0 {
        return None;
    }
    // `UniformListScrollHandle` exposes no public API for geometry; `.0`
    // reaches the internal `ListState`, the only stable source (mirrors
    // `files::build_files_scrollbar`).
    let state = handle.0.borrow();
    let viewport_h = state.base_handle.bounds().size.height;
    let max_offset = state.base_handle.max_offset().y;
    let offset_y = state.base_handle.offset().y;
    drop(state);
    let t = theme::current(cx);
    crate::ui::scrollbar::vertical_thumb(
        "git-changes-scrollbar-thumb",
        viewport_h,
        viewport_h + max_offset,
        offset_y,
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

#[cfg(test)]
mod tests {
    use super::{RemotePrimaryAction, remote_primary_action};

    #[test]
    fn remote_primary_action_pulls_once_fetch_finds_remote_work() {
        assert_eq!(remote_primary_action(0), RemotePrimaryAction::Fetch);
        assert_eq!(remote_primary_action(1), RemotePrimaryAction::Pull);
    }
}
