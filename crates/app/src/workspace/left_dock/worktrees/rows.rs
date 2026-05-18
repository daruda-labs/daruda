//! Visible row primitives for the worktrees view:
//! `section_header`, `worktree_row`, the small `WorktreeId → GitBadge`
//! derivation, and the non-git placeholder hint.

use crate::ui::theme;
use daruda_terminal::ux::strings;
use gpui::{
    ClickEvent, Context, IntoElement, MouseButton, MouseDownEvent, Pixels, Point, Rgba,
    SharedString, div, prelude::*, px,
};

use daruda_store::project::{ProjectId, WorktreeId};

use crate::surface::strings as surface_strings;
use crate::ui::{ButtonVariants as _, IconName, SectionHeader, button_bare};
use crate::workspace::layout::{Dock, GroupSnapshot, LeftDockSnapshot};
use crate::worktree::Worktree;

use super::claude_badges::{claude_badges_row, claude_status_cell};
use super::context_menu::build_context_menu_items;
use super::drag::{DraggedWorktree, DraggedWorktreeGhost};

/// Build the `M3 ?1` change-count badge string for worktree `id` from
/// the cached git status.  Returns `None` when no status is cached yet.
pub(in crate::workspace) fn git_badge_for(
    snap: &LeftDockSnapshot,
    project_id: ProjectId,
    id: WorktreeId,
) -> Option<String> {
    let target = daruda_store::project::WorktreeRef {
        project: project_id,
        worktree: id,
    };
    let status = snap.git_status_cache.get(&target)?;
    // Unstaged entries are identified by the y column, not x.
    let modified = status.staged.len()
        + status
            .unstaged
            .iter()
            .filter(|e| e.y != ' ' && e.y != '?')
            .count();
    let untracked = status.unstaged.iter().filter(|e| e.x == '?').count();
    if modified == 0 && untracked == 0 {
        return None;
    }
    let mut parts = Vec::new();
    if modified > 0 {
        parts.push(format!("M{modified}"));
    }
    if untracked > 0 {
        parts.push(format!("?{untracked}"));
    }
    Some(parts.join(" "))
}

/// Single-row group header that opens / closes the accordion for the
/// projects nested under `group`. Visual leading caret flips on
/// `is_collapsed`; an optional color dot renders when `group.color`
/// parses as a hex RGBA. Clicking the row toggles the group's collapse
/// flag on the workspace.
pub(in crate::workspace) fn group_header_row(
    group: &GroupSnapshot,
    snap: &LeftDockSnapshot,
    cx: &mut Context<Dock>,
) -> impl IntoElement + use<> {
    let t = theme::current(cx);
    let label_color = t.dock_view_tab_active;
    let row_hover_bg = t.worktree_row_hover_bg;

    let caret = if group.is_collapsed {
        surface_strings::FILES_CHEVRON_COLLAPSED
    } else {
        surface_strings::FILES_CHEVRON_EXPANDED
    };

    // Optional color dot — silently skip when the stored value is not a
    // parseable hex string. Group color is purely cosmetic; a malformed
    // value should never break the header render.
    let color_dot = group
        .color
        .as_ref()
        .and_then(|hex| Rgba::try_from(hex.as_ref()).ok())
        .map(|rgba| {
            div()
                .flex_none()
                .w(px(theme::WORKTREE_GROUP_COLOR_DOT_SIZE))
                .h(px(theme::WORKTREE_GROUP_COLOR_DOT_SIZE))
                .rounded(px(theme::WORKTREE_GROUP_COLOR_DOT_RADIUS))
                .bg(rgba)
        });

    let group_id = group.id;
    let workspace = snap.workspace.clone();
    div()
        .id(("group-header", group_id as usize))
        .flex()
        .flex_row()
        .items_center()
        .w_full()
        .gap(px(theme::WORKTREE_LABEL_GAP))
        .px(px(theme::WORKTREE_ROW_PAD_X))
        .py(px(theme::WORKTREE_SECTION_PAD_Y))
        .text_size(px(theme::WORKTREE_LABEL_FONT_SIZE))
        .text_color(label_color)
        .cursor_pointer()
        .hover(move |d| d.bg(row_hover_bg))
        .on_click(cx.listener(move |_dock, _: &ClickEvent, _window, cx| {
            if let Some(ws) = workspace.upgrade() {
                ws.update(cx, |ws, cx| ws.toggle_group_collapse(group_id, cx));
            }
        }))
        .child(div().flex_none().child(caret))
        .when_some(color_dot, |d, dot| d.child(dot))
        .child(
            div()
                .flex_1()
                .overflow_hidden()
                .whitespace_nowrap()
                .child(group.name.clone()),
        )
}

pub(in crate::workspace) fn section_header(
    any_git: bool,
    snap: &LeftDockSnapshot,
    cx: &mut Context<Dock>,
) -> impl IntoElement + use<> {
    let workspace = snap.workspace.clone();
    // `[+]` button is hidden for non-git folders per the plan.
    let add_button = button_bare("worktree-add-button")
        .ghost()
        .icon(IconName::Plus)
        .on_click(cx.listener(move |_dock, _: &ClickEvent, window, cx| {
            if let Some(ws) = workspace.upgrade() {
                let workspace_for_modal = workspace.clone();
                ws.update(cx, |ws, cx| {
                    let Some(repo_root) = ws.git_repo_root() else {
                        return;
                    };
                    crate::workspace::dialog_helpers::open_form_modal(
                        "Create Worktree",
                        None,
                        move |window, cx| {
                            super::create_modal::CreateWorktreeModal::new(
                                workspace_for_modal.clone(),
                                repo_root,
                                window,
                                cx,
                            )
                        },
                        window,
                        cx,
                    );
                });
            }
        }));

    SectionHeader::new(surface_strings::WORKTREES_SECTION_HEADER)
        .padding(theme::WORKTREE_ROW_PAD_X, theme::WORKTREE_SECTION_PAD_Y)
        .when(any_git, |h| h.actions(add_button))
}

pub(in crate::workspace) fn worktree_row(
    wt: &Worktree,
    project_id: ProjectId,
    is_active: bool,
    tab_count: usize,
    git_badge: Option<String>,
    snap: &LeftDockSnapshot,
    cx: &mut Context<Dock>,
) -> impl IntoElement {
    // Snapshot every row colour from the live theme so the closures
    // below (hover, drag_over) capture stable values.
    let t = theme::current(cx);
    let accent_active = t.worktree_accent_active;
    let unread_dot_color = t.worktree_unread;
    let label_active = t.dock_view_tab_active;
    let label_inactive = t.muted_text;
    let git_badge_text = t.git_badge_text;
    let sublabel_color = t.faint_text;
    let row_hover_bg = t.worktree_row_hover_bg;
    let drop_target_bg = t.worktree_drop_target_bg;

    let label = wt.display_name();
    // Sublabel priority: user-set description → path.
    let sublabel = wt
        .description
        .clone()
        .unwrap_or_else(|| wt.path.to_str().map(|s| s.to_string()).unwrap_or_default());

    // Left accent bar — painted only for the active row.
    let accent = div()
        .flex_none()
        .w(px(theme::WORKTREE_ACCENT_W))
        .h_full()
        .when(is_active, |d| d.bg(accent_active));

    // Unread marker sits to the right of the label.
    let unread_dot = div()
        .flex_none()
        .w(px(theme::WORKTREE_UNREAD_DOT_SIZE))
        .h(px(theme::WORKTREE_UNREAD_DOT_SIZE))
        .rounded(px(theme::WORKTREE_UNREAD_DOT_RADIUS))
        .bg(unread_dot_color);

    // Body (label + sublabel).
    let body = div()
        .flex_1()
        .flex()
        .flex_col()
        .overflow_hidden()
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(theme::WORKTREE_LABEL_GAP))
                .text_size(px(theme::WORKTREE_LABEL_FONT_SIZE))
                .text_color(if is_active {
                    label_active
                } else {
                    label_inactive
                })
                .child(
                    div()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .child(label.clone()),
                )
                .when(wt.is_unread, |d| d.child(unread_dot))
                .when_some(git_badge, |d, badge| {
                    d.child(
                        div()
                            .flex_none()
                            .text_size(px(theme::GIT_BADGE_FONT_SIZE))
                            .text_color(git_badge_text)
                            .child(badge),
                    )
                }),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(theme::WORKTREE_SUBLABEL_GAP))
                .text_size(px(theme::WORKTREE_SUB_FONT_SIZE))
                .text_color(sublabel_color)
                .overflow_hidden()
                .whitespace_nowrap()
                .child(sublabel)
                .when(tab_count > 0, |d| {
                    d.child(format!(
                        "• {tab_count} tab{}",
                        if tab_count == 1 { "" } else { "s" }
                    ))
                }),
        )
        .when_some(
            snap.claude_per_session_per_worktree.get(&wt.id),
            |d, sessions| {
                d.child(claude_badges_row(
                    sessions,
                    snap.claude_active_session_id.as_deref(),
                    cx,
                ))
            },
        );

    let wt_id: WorktreeId = wt.id;
    let wt_path: std::path::PathBuf = wt.path.clone();
    let wt_label_shared = SharedString::from(label.clone());
    let wt_description_current: Option<String> = wt.description.clone();
    let wt_name_current: Option<String> = wt.name.clone();
    let removable = crate::workspace::Workspace::worktree_removable(wt);

    // Merge context menu state — captured before closures so all
    // values are 'static (no borrow of `wt` or `snap` inside closures).
    let wt_is_git = matches!(&wt.kind, daruda_store::project::WorktreeKind::Git { .. });
    let wt_is_detached = matches!(
        &wt.kind,
        daruda_store::project::WorktreeKind::Git { branch: None, .. }
    );
    let wt_source_branch: Option<String> = match &wt.kind {
        daruda_store::project::WorktreeKind::Git {
            branch: Some(b), ..
        } => Some(b.clone()),
        _ => None,
    };
    let wt_base_ref: Option<String> = wt.base_ref.clone();
    let wt_is_dirty = snap
        .git_status_cache
        .get(&daruda_store::project::WorktreeRef {
            project: project_id,
            worktree: wt.id,
        })
        .is_some_and(|s| !s.staged.is_empty() || !s.unstaged.is_empty());
    let wt_source_repo_root: Option<std::path::PathBuf> = match &wt.kind {
        daruda_store::project::WorktreeKind::Git { repo_root, .. } => Some(repo_root.clone()),
        _ => None,
    };
    let workspace = snap.workspace.clone();

    // Drag payload — captured once so on_drag + on_drop share the same Arc.
    let drag_payload = DraggedWorktree {
        id: wt_id,
        label: wt_label_shared.clone(),
    };

    let ws_for_click = workspace.clone();
    let ws_for_drop = workspace.clone();
    let ws_for_rclick = workspace.clone();
    let mut row = div()
        .id(("worktree-row", wt_id as usize))
        .flex()
        .flex_row()
        .items_center()
        .min_h(px(theme::WORKTREE_ROW_HEIGHT))
        .px(px(theme::WORKTREE_ROW_PAD_X))
        .gap(px(theme::WORKTREE_ROW_GAP))
        .cursor_pointer()
        .when(!is_active, move |d| d.hover(move |d| d.bg(row_hover_bg)))
        // on_click fires only when the mousedown + mouseup happen at the
        // same position (no drag movement), so it coexists safely with
        // on_drag without a hysteresis guard.
        .on_click(cx.listener(move |_dock, _ev: &ClickEvent, window, cx| {
            if let Some(ws) = ws_for_click.upgrade() {
                let target = daruda_store::project::WorktreeRef {
                    project: project_id,
                    worktree: wt_id,
                };
                ws.update(cx, |ws, cx| ws.activate_worktree(target, window, cx));
            }
        }))
        // Drag source — GPUI's built-in on_drag / on_drop pipeline handles
        // reordering without any manual drag-state tracking.
        .on_drag(drag_payload, |dragged, _offset, _window, cx| {
            cx.new(|_| DraggedWorktreeGhost {
                label: dragged.label.clone(),
            })
        })
        // Drop target — highlight while a worktree is hovering over this row.
        .drag_over::<DraggedWorktree>(move |style, _dragged, _window, _cx| style.bg(drop_target_bg))
        .on_drop(
            cx.listener(move |_dock, dragged: &DraggedWorktree, _window, cx| {
                if let Some(ws) = ws_for_drop.upgrade() {
                    ws.update(cx, |ws, cx| ws.reorder_worktree(dragged.id, wt_id, cx));
                }
            }),
        )
        // Right-click opens the context menu. stop_propagation prevents
        // the event from reaching any ancestor that might close the menu
        // before it has a chance to render.
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |_dock, ev: &MouseDownEvent, _window, cx| {
                cx.stop_propagation();
                let position: Point<Pixels> = ev.position;
                let path_str = wt_path.to_str().map(|s| s.to_string()).unwrap_or_default();
                if let Some(ws) = ws_for_rclick.upgrade() {
                    let items = build_context_menu_items(
                        wt_id,
                        path_str,
                        wt_description_current.clone(),
                        wt_name_current.clone(),
                        ws_for_rclick.clone(),
                        wt_is_git,
                        wt_is_detached,
                        wt_is_dirty,
                        wt_source_branch.clone(),
                        wt_base_ref.clone(),
                        wt_path.clone(),
                        wt_source_repo_root.clone(),
                    );
                    ws.update(cx, |ws, cx| ws.open_context_menu(position, items, cx));
                }
            }),
        )
        .child(accent)
        .child(claude_status_cell(
            snap.claude_status_per_worktree.get(&wt.id).copied(),
            cx,
        ))
        .child(body);

    if removable {
        let ws_for_close = workspace.clone();
        let close = button_bare(("wt-remove", wt_id as usize))
            .ghost()
            .icon(IconName::Close)
            .on_click(cx.listener(move |_dock, _: &ClickEvent, window, cx| {
                // Stop the row's activate handler from firing —
                // clicking × should never also switch to the
                // worktree we're about to remove.
                cx.stop_propagation();
                if let Some(ws) = ws_for_close.upgrade() {
                    let ws_for_modal = ws_for_close.clone();
                    ws.update(cx, |ws, cx| {
                        let target = daruda_store::project::WorktreeRef {
                            project: project_id,
                            worktree: wt_id,
                        };
                        let Some(wt) = ws.worktree_for(target) else {
                            return;
                        };
                        if !crate::workspace::Workspace::worktree_removable(wt) {
                            return;
                        }
                        let label = gpui::SharedString::from(wt.display_name());
                        let path = gpui::SharedString::from(wt.path.to_string_lossy().into_owned());
                        let plan = match ws.validate_remove_worktree(target) {
                            Ok(p) => p,
                            Err(_) => return,
                        };
                        // Pull the branch name so the modal can offer "Also
                        // delete branch X" — None for default / detached
                        // worktrees (modal hides the checkbox).
                        let branch = ws.worktree_for(target).and_then(|w| match &w.kind {
                            daruda_store::project::WorktreeKind::Git {
                                branch: Some(b), ..
                            } => Some(b.clone()),
                            _ => None,
                        });
                        crate::workspace::dialog_helpers::open_form_modal(
                            "Remove Worktree",
                            None,
                            move |window, cx| {
                                super::remove_modal::RemoveWorktreeModal::new(
                                    ws_for_modal.clone(),
                                    wt_id,
                                    label,
                                    path,
                                    plan,
                                    window,
                                    cx,
                                )
                                .with_branch(branch)
                            },
                            window,
                            cx,
                        );
                    });
                }
            }));
        row = row.child(close);
    }

    row
}

pub(in crate::workspace) fn non_git_placeholder(cx: &gpui::App) -> impl IntoElement {
    let t = theme::current(cx);
    let hint_color = t.faint_text;
    let init_color = t.muted_text;
    div()
        .flex()
        .flex_col()
        .gap(px(theme::WORKTREE_PLACEHOLDER_LINE_GAP))
        .p(px(theme::WORKTREE_PLACEHOLDER_PAD))
        .text_size(px(theme::WORKTREE_SUB_FONT_SIZE))
        .text_color(hint_color)
        .child(strings::WORKTREE_NON_GIT_HINT)
        .child(
            div()
                .mt(px(theme::WORKTREE_PLACEHOLDER_GIT_INIT_MT))
                .text_color(init_color)
                .child(strings::WORKTREE_GIT_INIT_LABEL),
        )
}
