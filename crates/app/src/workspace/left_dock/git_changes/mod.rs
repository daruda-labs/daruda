//! Git Changes dock view — unified file list with commit / push controls.
//!
//! All changed files (staged and unstaged) appear in a single sorted list.
//! Each row has a checkbox: checked = staged, unchecked = unstaged.
//! Checking/unchecking stages or unstages the file without changing its
//! position in the list.

use std::path::PathBuf;

use crate::ui::theme;
use daruda_store::project::LaneId;
use gpui::{
    AnyElement, ClickEvent, Context, ElementId, IntoElement, MouseButton, MouseDownEvent, div,
    prelude::*, px,
};

use crate::lane::git::GitFileEntry;
use crate::lane::paths::LanePaths;
use crate::path_ext::PathExt;
use crate::surface::strings as app_strings;
use crate::ui::{
    ButtonVariants as _, ContextMenuItem, Icon, IconName, SectionHeader, Sizable as _, button,
    button_bare,
};
use crate::workspace::layout::Dock;
use crate::workspace::layout::LeftDockSnapshot;
use crate::workspace::left_dock::git_ops::git_status_color;
use crate::workspace::path_drag::PathDrag;

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
    let mut body = div()
        .key_context("GitChanges")
        .track_focus(&panel_focus)
        .flex()
        .flex_col()
        .w_full()
        .h_full()
        .overflow_hidden();

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
// Unified file list builder
// ----------------------------------------------------------------

struct UnifiedEntry {
    path: PathBuf,
    staged: Option<GitFileEntry>,
    unstaged: Option<GitFileEntry>,
}

fn build_unified_list(staged: &[GitFileEntry], unstaged: &[GitFileEntry]) -> Vec<UnifiedEntry> {
    use std::collections::BTreeMap;

    let mut map: BTreeMap<PathBuf, UnifiedEntry> = BTreeMap::new();

    for e in staged {
        map.entry(e.path.clone())
            .or_insert_with(|| UnifiedEntry {
                path: e.path.clone(),
                staged: None,
                unstaged: None,
            })
            .staged = Some(e.clone());
    }
    for e in unstaged {
        map.entry(e.path.clone())
            .or_insert_with(|| UnifiedEntry {
                path: e.path.clone(),
                staged: None,
                unstaged: None,
            })
            .unstaged = Some(e.clone());
    }

    map.into_values().collect()
}

/// Group a unified list by lane-relative parent directory.
///
/// Output order: directory groups first (alphabetical by dir path),
/// then a single root-level group (no parent dir) at the end. Files
/// within each group are sorted alphabetically by their full path.
/// Putting root files last separates "files in a folder" from
/// "loose files at the repo root" visually instead of interleaving
/// them with directory groups by raw string order.
fn group_by_dir(
    entries: Vec<UnifiedEntry>,
    wt_paths: &LanePaths<'_>,
) -> Vec<(Option<String>, Vec<UnifiedEntry>)> {
    use std::collections::BTreeMap;

    let mut dirs: BTreeMap<String, Vec<UnifiedEntry>> = BTreeMap::new();
    let mut root: Vec<UnifiedEntry> = Vec::new();

    for entry in entries {
        let abs = wt_paths.from_git_status(&entry.path);
        let dir = abs
            .strip_prefix_or_self(wt_paths.wt_path)
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(|p| p.to_string_lossy().into_owned());
        match dir {
            Some(d) => dirs.entry(d).or_default().push(entry),
            None => root.push(entry),
        }
    }

    for v in dirs.values_mut() {
        v.sort_by(|a, b| a.path.cmp(&b.path));
    }
    root.sort_by(|a, b| a.path.cmp(&b.path));

    let mut groups: Vec<(Option<String>, Vec<UnifiedEntry>)> =
        dirs.into_iter().map(|(k, v)| (Some(k), v)).collect();
    if !root.is_empty() {
        groups.push((None, root));
    }
    groups
}

/// Repo-root-relative paths in the same order the left dock renders them,
/// minus any rows hidden inside a collapsed dir group. Single source of
/// truth for the keyboard cursor's navigation order — Workspace's
/// `move_git_changes_cursor` calls this so future render-side reordering
/// (sticky conflicts, custom sort) automatically applies to nav.
pub(in crate::workspace) fn ordered_visible_paths(
    status: &crate::lane::git::GitStatusData,
    collapsed: &std::collections::HashSet<String>,
    wt_paths: &LanePaths<'_>,
) -> Vec<PathBuf> {
    let unified = build_unified_list(&status.staged, &status.unstaged);
    let groups = group_by_dir(unified, wt_paths);
    let mut out = Vec::new();
    for (dir_opt, entries) in groups {
        let is_collapsed = dir_opt
            .as_deref()
            .map(|d| collapsed.contains(d))
            .unwrap_or(false);
        if is_collapsed {
            continue;
        }
        for e in entries {
            out.push(e.path);
        }
    }
    out
}

// ----------------------------------------------------------------
// Header — branch label + remote action buttons
// ----------------------------------------------------------------

fn view_header(
    _worktree_id: LaneId,
    branch: &str,
    snap: &LeftDockSnapshot,
    cx: &mut Context<Dock>,
) -> impl IntoElement {
    use crate::ui::Disableable as _;

    let label = format!("Git Changes — {branch}");
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
        .ghost()
        .icon(IconName::Refresh)
        .on_click(cx.listener(move |_dock, _: &ClickEvent, _window, cx| {
            if let Some(ws) = workspace_refresh.upgrade() {
                ws.update(cx, |ws, cx| ws.refresh_git_status(active_ref, cx));
            }
        }));

    let tracking_color = theme::current(cx).muted_text;
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
        .loading(in_flight)
        .disabled(in_flight)
        .on_click(cx.listener(move |_dock, _: &ClickEvent, _window, cx| {
            if let Some(ws) = workspace_fetch.upgrade() {
                ws.update(cx, |ws, cx| ws.on_fetch(cx));
            }
        }));

    let push_btn = button("git-push", app_strings::git_push_btn())
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

/// Tracking indicator for the header — `↑N ↓M` when the local branch
/// diverges from its upstream. Returns `None` when both sides are 0
/// (in sync, no upstream, or detached HEAD) so the caller can omit the
/// element entirely.
fn tracking_indicator_text(ahead: u32, behind: u32) -> Option<String> {
    match (ahead, behind) {
        (0, 0) => None,
        (a, 0) => Some(format!("↑{a}")),
        (0, b) => Some(format!("↓{b}")),
        (a, b) => Some(format!("↑{a} ↓{b}")),
    }
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
    let summary_text_color = t.dock_header_text;
    let toggle_inflight = t.faint_text;
    let toggle_idle = t.muted_text;
    let toggle_hover = t.dock_view_tab_active;

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

/// Count merge-conflict entries in the unstaged list. `parse_git_status_output`
/// routes anything with a `U` column or both-add / both-delete into
/// `unstaged`, so checking that side is sufficient.
fn count_conflicts(unstaged: &[GitFileEntry]) -> usize {
    unstaged
        .iter()
        .filter(|e| matches!((e.x, e.y), ('U', _) | (_, 'U') | ('D', 'D') | ('A', 'A')))
        .count()
}

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

/// Aggregate staging state of a directory group.
#[derive(Clone, Copy, PartialEq, Eq)]
enum DirStageState {
    /// Every file in the dir is fully staged (no working-tree leftover).
    AllStaged,
    /// No file in the dir has any staged change.
    NoneStaged,
    /// Mixed — some staged, some unstaged, or files with both staged
    /// and unstaged changes (e.g. `MM`).
    Mixed,
}

fn compute_dir_state(entries: &[UnifiedEntry]) -> DirStageState {
    let mut any_staged = false;
    let mut any_unstaged = false;
    for e in entries {
        if e.staged.is_some() {
            any_staged = true;
        }
        if e.unstaged.is_some() {
            any_unstaged = true;
        }
    }
    match (any_staged, any_unstaged) {
        (true, false) => DirStageState::AllStaged,
        (false, true) => DirStageState::NoneStaged,
        (true, true) => DirStageState::Mixed,
        // No changes in this dir — render as if NoneStaged (the dir
        // wouldn't be in the list at all if every entry were empty,
        // but be defensive).
        (false, false) => DirStageState::NoneStaged,
    }
}

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
    let checkbox_border = t.git_stage_checkbox_border;
    let checkbox_checked_bg = t.git_stage_checkbox_checked_bg;
    let checkbox_unchecked_bg = t.git_stage_checkbox_unchecked_bg;
    let checkbox_tick_color = t.modal_text_primary;
    let dir_label_color = t.faint_text;
    let dir_label_hover = t.muted_text;

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
        .border_color(checkbox_border)
        .bg(match state {
            DirStageState::AllStaged => checkbox_checked_bg,
            DirStageState::Mixed | DirStageState::NoneStaged => checkbox_unchecked_bg,
        })
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(theme::GIT_STAGE_CHECKBOX_TICK_SIZE))
        .text_color(checkbox_tick_color)
        .when(state == DirStageState::AllStaged, |d| {
            d.child(app_strings::UI_CHECKMARK)
        })
        .when(state == DirStageState::Mixed, |d| {
            // Indeterminate: dash glyph at the same size as the tick.
            d.child("–")
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

/// Whether the "Discard Changes" context-menu item should be disabled
/// for a row in the given (is_staged, has_unstaged) state.
///
/// `git restore` only touches the working tree, so a purely staged row
/// (`M `, `A `, `D `) has nothing to discard without first unstaging —
/// surface that by greying out the item. Untracked (`??`) routes to
/// `git clean -f`, which always has something to discard, so the row is
/// `is_staged = false, has_unstaged = true` and stays enabled. Combined
/// states (`MM`, `MD`, `AM`, etc.) have a working-tree change to discard
/// and stay enabled.
fn discard_disabled(is_staged: bool, has_unstaged: bool) -> bool {
    is_staged && !has_unstaged
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
    let status_char_for_diff = status_char;
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
    let checkbox_border = t.git_stage_checkbox_border;
    let checkbox_checked_bg = t.git_stage_checkbox_checked_bg;
    let checkbox_unchecked_bg = t.git_stage_checkbox_unchecked_bg;
    let checkbox_tick_color = t.modal_text_primary;
    let cursor_border_color = t.dock_view_tab_accent;
    let row_selected_bg = t.git_file_row_selected_bg;
    let row_hover_bg = t.git_file_row_hover_bg;
    let filename_color = t.muted_text;
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
        .border_color(checkbox_border)
        .bg(if is_staged {
            checkbox_checked_bg
        } else {
            checkbox_unchecked_bg
        })
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(theme::GIT_STAGE_CHECKBOX_TICK_SIZE))
        .text_color(checkbox_tick_color)
        .when(is_staged, |d| d.child(app_strings::UI_CHECKMARK))
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
        // position (no drag past DRAG_THRESHOLD), so dragging the row to drop
        // its path elsewhere no longer also opens the diff view.
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
                        ws.open_git_file_diff(
                            lane_id,
                            abs_path,
                            is_staged,
                            Some(status_char_for_diff),
                            window,
                            cx,
                        );
                        fh.focus(window, cx);
                    }
                });
            }
        }))
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |_dock, ev: &MouseDownEvent, _window, cx| {
                cx.stop_propagation();
                let Some(ws) = workspace_for_ctx.upgrade() else {
                    return;
                };

                let ws_stage = ws.clone();
                let ws_discard = ws.clone();
                let ws_diff = ws.clone();
                let path_stage = path_for_ctx.clone();
                let path_discard = path_for_ctx.clone();
                let path_diff = abs_path_for_ctx_diff.clone();

                let mut items: Vec<ContextMenuItem> = Vec::new();

                if is_staged {
                    items.push(ContextMenuItem::new(
                        app_strings::ctx_git_unstage(),
                        move |_, _, cx| {
                            ws_stage.update(cx, |ws, cx| {
                                ws.close_context_menu(cx);
                                ws.unstage_file(lane_id, path_stage.clone(), cx)
                            });
                        },
                    ));
                } else {
                    items.push(ContextMenuItem::new(
                        app_strings::ctx_git_stage(),
                        move |_, _, cx| {
                            ws_stage.update(cx, |ws, cx| {
                                ws.close_context_menu(cx);
                                ws.stage_file(lane_id, path_stage.clone(), cx)
                            });
                        },
                    ));
                }

                items.push(ContextMenuItem::separator());
                items.push(ContextMenuItem::new(
                    app_strings::ctx_git_open_diff(),
                    move |_, window, cx| {
                        ws_diff.update(cx, |ws, cx| {
                            ws.close_context_menu(cx);
                            ws.open_git_file_diff(
                                lane_id,
                                path_diff.clone(),
                                is_staged,
                                Some(status_char_for_diff),
                                window,
                                cx,
                            )
                        });
                    },
                ));

                items.push(ContextMenuItem::separator());
                items.push(
                    ContextMenuItem::new(app_strings::ctx_git_discard(), move |_, window, cx| {
                        ws_discard.update(cx, |ws, cx| {
                            ws.on_discard_file(
                                lane_id,
                                path_discard.clone(),
                                is_untracked,
                                window,
                                cx,
                            )
                        });
                    })
                    // Discard is dangerous — disable only when the file has no
                    // working-tree changes to discard (purely staged, like `M `).
                    // For `MM` (staged + unstaged) and untracked, leave it enabled.
                    .disabled(discard_disabled(is_staged, has_unstaged)),
                );

                ws.update(cx, |ws, cx| ws.open_context_menu(ev.position, items, cx));
            }),
        )
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
        .into_any_element()
}

// ----------------------------------------------------------------
// Commit footer — InputPanel (TextArea + Commit / Push buttons)
// ----------------------------------------------------------------

fn commit_footer(snap: &LeftDockSnapshot, cx: &mut Context<Dock>) -> impl IntoElement {
    let border = theme::current(cx).git_commit_border;
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

fn git_changes_scrollbar(handle: &gpui::ScrollHandle, cx: &gpui::App) -> Option<gpui::AnyElement> {
    let viewport_h = handle.bounds().size.height;
    let max_offset = handle.max_offset().height;
    if max_offset <= px(0.) || viewport_h <= px(0.) {
        return None;
    }
    let content_h = viewport_h + max_offset;
    let thumb_ratio = (viewport_h / content_h).min(1.0_f32);
    let thumb_h = (viewport_h * thumb_ratio).max(px(theme::DOCK_SCROLLBAR_MIN_THUMB_H));
    let track_h = viewport_h - thumb_h;
    let scroll_frac = ((-handle.offset().y) / max_offset).clamp(0.0_f32, 1.0_f32);
    let thumb_top = track_h * scroll_frac;
    let w = px(theme::DOCK_SCROLLBAR_W);
    let r = px(theme::DOCK_SCROLLBAR_MARGIN_R);
    let t = theme::current(cx);
    let thumb_bg = t.dock_scrollbar_thumb;
    let thumb_hover_bg = t.dock_scrollbar_thumb_hover;
    Some(
        div()
            .id("git-changes-scrollbar-thumb")
            .absolute()
            .top(thumb_top)
            .right(r)
            .w(w)
            .h(thumb_h)
            .rounded(w / 2.0)
            .bg(thumb_bg)
            .hover(move |d| d.bg(thumb_hover_bg))
            .into_any_element(),
    )
}

// ----------------------------------------------------------------
// Placeholder states
// ----------------------------------------------------------------

fn loading_placeholder(
    _worktree_id: LaneId,
    snap: &LeftDockSnapshot,
    cx: &mut Context<Dock>,
) -> impl IntoElement {
    let text_color = theme::current(cx).faint_text;
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
        .child(app_strings::git_loading_changes())
        .child(refresh_btn)
}

fn clean_placeholder(cx: &gpui::App) -> impl IntoElement {
    div()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(theme::DOCK_PLACEHOLDER_FONT_SIZE))
        .text_color(theme::current(cx).dock_placeholder_text)
        .child("No changes")
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

    let text_color = theme::current(cx).dock_placeholder_text;
    div()
        .flex_1()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(theme::GIT_COMMIT_PAD))
        .text_size(px(theme::DOCK_PLACEHOLDER_FONT_SIZE))
        .text_color(text_color)
        .child("Not a Git repository")
        .child(init_btn)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lane::git::{GitFileEntry, GitStatusData};
    use std::collections::HashSet;
    use std::path::Path;

    fn entry(x: char, y: char, path: &str) -> GitFileEntry {
        GitFileEntry {
            x,
            y,
            path: PathBuf::from(path),
            ..Default::default()
        }
    }

    /// `LanePaths` borrows from the caller's `Path` storage. The
    /// helper uses `from_git_status(p)` (repo-root-relative → absolute)
    /// and `strip_prefix_or_self(wt_path).parent()` to derive the
    /// dir-group key. For tests we set `wt_path == repo_root` so the
    /// repo-root-relative input round-trips back to the same string the
    /// render pipeline groups by.
    fn paths_for(root: &Path) -> LanePaths<'_> {
        LanePaths {
            wt_path: root,
            repo_root: Some(root),
        }
    }

    #[test]
    fn ordered_visible_paths_alphabetical_across_groups() {
        let root = Path::new("/repo");
        let status = GitStatusData {
            staged: vec![entry('M', ' ', "src/lib.rs")],
            unstaged: vec![
                entry(' ', 'M', "Cargo.toml"),
                entry(' ', 'M', "src/foo/a.rs"),
            ],
            ..Default::default()
        };
        let collapsed: HashSet<String> = HashSet::new();
        let paths = ordered_visible_paths(&status, &collapsed, &paths_for(root));
        assert_eq!(
            paths,
            vec![
                PathBuf::from("src/lib.rs"),
                PathBuf::from("src/foo/a.rs"),
                PathBuf::from("Cargo.toml"),
            ],
            "directory groups first (alphabetical by dir path), then root \
             entries last; alphabetical within each group"
        );
    }

    #[test]
    fn ordered_visible_paths_skips_collapsed_dirs() {
        let root = Path::new("/repo");
        let status = GitStatusData {
            staged: vec![],
            unstaged: vec![
                entry(' ', 'M', "Cargo.toml"),
                entry(' ', 'M', "src/foo/a.rs"),
                entry(' ', 'M', "src/foo/b.rs"),
                entry(' ', 'M', "src/lib.rs"),
            ],
            ..Default::default()
        };
        let mut collapsed: HashSet<String> = HashSet::new();
        collapsed.insert("src/foo".to_string());
        let paths = ordered_visible_paths(&status, &collapsed, &paths_for(root));
        assert_eq!(
            paths,
            vec![PathBuf::from("src/lib.rs"), PathBuf::from("Cargo.toml"),],
            "src/foo entries hidden; src/lib.rs remains as dir group, \
             Cargo.toml stays last as a root entry"
        );
    }

    #[test]
    fn ordered_visible_paths_unifies_staged_and_unstaged() {
        // A file with both staged + unstaged changes (`MM`) appears
        // once in the visible list.
        let root = Path::new("/repo");
        let status = GitStatusData {
            staged: vec![entry('M', 'M', "file.rs")],
            unstaged: vec![entry('M', 'M', "file.rs")],
            ..Default::default()
        };
        let collapsed: HashSet<String> = HashSet::new();
        let paths = ordered_visible_paths(&status, &collapsed, &paths_for(root));
        assert_eq!(paths, vec![PathBuf::from("file.rs")]);
    }

    #[test]
    fn discard_disabled_truth_table() {
        // (is_staged, has_unstaged) → disabled
        let cases = [
            // `M ` / `A ` / `D ` — staged only, nothing in working tree.
            ((true, false), true, "staged-only must be disabled"),
            // `MM` / `MD` / `AM` — staged + working-tree change.
            ((true, true), false, "staged + unstaged must be enabled"),
            // ` M` — unstaged modification.
            ((false, true), false, "unstaged-only must be enabled"),
            // `??` — untracked, `has_unstaged = true` via porcelain Y='?'.
            ((false, true), false, "untracked must be enabled"),
            // Defensive: nothing on either side shouldn't happen, but
            // it logically has nothing to discard either.
            (
                (false, false),
                false,
                "no changes — enabled (no-op fallback)",
            ),
        ];
        for ((is_staged, has_unstaged), expected, msg) in cases {
            assert_eq!(
                discard_disabled(is_staged, has_unstaged),
                expected,
                "{msg}: ({is_staged}, {has_unstaged})"
            );
        }
    }

    #[test]
    fn ordered_visible_paths_empty_when_no_changes() {
        let root = Path::new("/repo");
        let status = GitStatusData::default();
        let collapsed: HashSet<String> = HashSet::new();
        let paths = ordered_visible_paths(&status, &collapsed, &paths_for(root));
        assert!(paths.is_empty());
    }
}
