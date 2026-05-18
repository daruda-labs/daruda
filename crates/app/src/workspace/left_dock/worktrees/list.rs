//! Worktrees view — list of worktrees plus a section header. Rendered
//! into the left dock when `left_dock_view == Worktrees`.
//!
//! W-2 milestone: bootstrap + list render only. Create / remove /
//! rename flow arrives in W-5, context menu in W-8, git kind upgrade
//! in W-4. Row count is always ≥ 1 when the Workspace has a project.

use crate::ui::theme;
use daruda_terminal::ux::strings;
use gpui::{
    AnyElement, App, ClickEvent, Context, IntoElement, MouseButton, MouseDownEvent, Pixels, Point,
    Render, SharedString, Window, div, prelude::*, px,
};

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::project::WorktreeId;

use crate::ui::dialog::ButtonVariant;

use crate::surface::strings as surface_strings;
use crate::ui::{ButtonVariants as _, ContextMenuItem, IconName, SectionHeader, button_bare};
use crate::workspace::layout::Dock;
use crate::workspace::layout::LeftDockSnapshot;
use crate::worktree::Worktree;

use super::claude_badges::{claude_badges_row, claude_status_cell};

// ----------------------------------------------------------------
// Drag payload — passed through GPUI's on_drag / on_drop chain.
// ----------------------------------------------------------------

/// Data carried by a worktree row during a drag operation. The
/// `label` is used by the ghost preview so it does not need to
/// re-read the worktree list while the drag is in flight.
#[derive(Clone, Debug)]
pub(in crate::workspace) struct DraggedWorktree {
    pub id: WorktreeId,
    pub label: SharedString,
}

/// Minimal ghost element shown under the cursor while a row is being
/// dragged. Renders a single-line label styled to match the left dock
/// row.
struct DraggedWorktreeGhost {
    label: SharedString,
}

impl Render for DraggedWorktreeGhost {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::current(cx);
        div()
            .px(px(theme::WORKTREE_ROW_PAD_X))
            .py(px(theme::WORKTREE_DRAG_GHOST_PAD_Y))
            .text_size(px(theme::WORKTREE_LABEL_FONT_SIZE))
            .text_color(t.dock_view_tab_active)
            .bg(t.worktree_row_hover_bg)
            .rounded(px(theme::MODAL_BUTTON_RADIUS))
            .child(self.label.clone())
    }
}

// ----------------------------------------------------------------
// Public render entry point
// ----------------------------------------------------------------

/// Render the Worktrees view body.
pub(in crate::workspace) fn render(snap: &LeftDockSnapshot, cx: &mut Context<Dock>) -> AnyElement {
    if snap.worktrees.is_empty() {
        return empty_state(cx).into_any_element();
    }

    let any_git = snap.worktrees.iter().any(|w| w.is_git());
    let active_tab_count = snap.active_tab_count;
    let active_id = snap.active.worktree;

    let header = section_header(any_git, snap, cx);

    let mut list = div().flex().flex_col().w_full();
    for wt in &snap.worktrees {
        let is_active = wt.id == active_id;
        let tab_count = if is_active { active_tab_count } else { 0 };
        let git_badge = git_badge_for(snap, wt.id);
        list = list.child(worktree_row(wt, is_active, tab_count, git_badge, snap, cx));
    }

    let mut body = div().flex().flex_col().w_full().overflow_hidden();
    if snap.claude_install_banner_visible {
        body = body.child(claude_install_banner(snap, cx));
    }
    if let Some(name) = snap.active_project_name.clone() {
        body = body.child(project_header_row(name, cx));
    }
    body = body.child(header).child(list);

    // Non-git project hint — surfaces the `git init` path.
    if !any_git {
        body = body.child(non_git_placeholder(cx));
    }

    body.into_any_element()
}

/// "Claude Code integration disabled" banner. Click → install action.
fn claude_install_banner(
    snap: &LeftDockSnapshot,
    cx: &mut Context<Dock>,
) -> impl IntoElement + use<> {
    let workspace = snap.workspace.clone();
    let t = theme::current(cx);
    let bg = t.claude_banner_bg;
    let border = t.claude_banner_border;
    let hover_bg = t.claude_banner_hover_bg;
    let text = t.claude_banner_text;
    let icon = t.claude_banner_icon;
    let hint_text = t.faint_text;
    div()
        .id("claude-install-banner")
        .mx(px(theme::CLAUDE_BANNER_MARGIN_X))
        .my(px(theme::CLAUDE_BANNER_MARGIN_Y))
        .px(px(theme::CLAUDE_BANNER_PAD_X))
        .py(px(theme::CLAUDE_BANNER_PAD_Y))
        .rounded(px(theme::CLAUDE_BANNER_RADIUS))
        .bg(bg)
        .border_1()
        .border_color(border)
        .hover(move |d| d.bg(hover_bg))
        .cursor_pointer()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::CLAUDE_BANNER_GAP))
        .text_size(px(theme::CLAUDE_BANNER_FONT_SIZE))
        .text_color(text)
        .child(
            div()
                .flex_none()
                .text_color(icon)
                .child(surface_strings::CLAUDE_BANNER_ICON),
        )
        .child(
            div()
                .flex_1()
                .flex()
                .flex_col()
                .child(surface_strings::CLAUDE_BANNER_TITLE)
                .child(
                    div()
                        .text_color(hint_text)
                        .child(surface_strings::CLAUDE_BANNER_HINT),
                ),
        )
        .on_click(cx.listener(move |_dock, _: &ClickEvent, window, cx| {
            let Some(_ws) = workspace.upgrade() else {
                return;
            };
            let weak = workspace.clone();
            crate::workspace::dialog_helpers::open_confirm_dialog(
                surface_strings::CLAUDE_CONSENT_TITLE,
                surface_strings::CLAUDE_CONSENT_BODY,
                surface_strings::CLAUDE_CONSENT_CONFIRM,
                ButtonVariant::Primary,
                move |_, window, app_cx| {
                    if let Some(ws) = weak.upgrade() {
                        ws.update(app_cx, |ws, cx| {
                            ws.on_install_claude_hooks(
                                &crate::workspace::InstallClaudeHooks,
                                window,
                                cx,
                            );
                        });
                    }
                },
                window,
                cx,
            );
        }))
}

/// Single-row project header above the flat worktrees list. Temporary
/// placeholder for the W-2 flat view — commit f replaces this with a
/// 2-level Group ▸ Project ▸ Worktree tree.
fn project_header_row(name: SharedString, cx: &mut Context<Dock>) -> impl IntoElement + use<> {
    let t = theme::current(cx);
    div()
        .flex()
        .flex_row()
        .items_center()
        .w_full()
        .px(px(theme::WORKTREE_ROW_PAD_X))
        .py(px(theme::WORKTREE_SECTION_PAD_Y))
        .text_size(px(theme::WORKTREE_LABEL_FONT_SIZE))
        .text_color(t.dock_view_tab_active)
        .child(name)
}

fn section_header(
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

/// Build the `M3 ?1` change-count badge string for worktree `id` from
/// the cached git status.  Returns `None` when no status is cached yet.
fn git_badge_for(snap: &LeftDockSnapshot, id: WorktreeId) -> Option<String> {
    let target = daruda_store::project::WorktreeRef {
        project: snap.active.project,
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

fn worktree_row(
    wt: &Worktree,
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
    let snap_active = snap.active;
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
            project: snap.active.project,
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
                    project: snap_active.project,
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
                            project: snap_active.project,
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

/// Build the context menu item list for a worktree row right-click.
/// Captures path / id by value so the closures are `'static`.
#[allow(clippy::too_many_arguments)]
fn build_context_menu_items(
    wt_id: WorktreeId,
    path_str: String,
    current_description: Option<String>,
    current_name: Option<String>,
    workspace: gpui::WeakEntity<crate::workspace::Workspace>,
    is_git: bool,
    is_detached: bool,
    is_dirty: bool,
    source_branch: Option<String>,
    base_ref: Option<String>,
    source_path: std::path::PathBuf,
    source_repo_root: Option<std::path::PathBuf>,
) -> Vec<ContextMenuItem> {
    let workspace_for_reveal = workspace.clone();
    let path_for_reveal = path_str.clone();
    let reveal_item = ContextMenuItem::new(
        surface_strings::CTX_REVEAL_IN_FINDER,
        move |_ev: &MouseDownEvent, _window, app_cx: &mut App| {
            let path = path_for_reveal.clone();
            let workspace = workspace_for_reveal.clone();
            app_cx
                .background_executor()
                .spawn(async move {
                    std::process::Command::new("open")
                        .args(["-R", &path])
                        .spawn()
                        .ok();
                })
                .detach();
            // Close the context menu.
            if let Some(ws) = workspace.upgrade() {
                ws.update(app_cx, |ws, cx| ws.close_context_menu(cx));
            }
        },
    );

    let workspace_for_copy = workspace.clone();
    let path_for_copy = path_str.clone();
    let copy_item = ContextMenuItem::new(
        surface_strings::CTX_COPY_PATH,
        move |_ev: &MouseDownEvent, _window, app_cx: &mut App| {
            app_cx.write_to_clipboard(gpui::ClipboardItem::new_string(path_for_copy.clone()));
            if let Some(ws) = workspace_for_copy.upgrade() {
                ws.update(app_cx, |ws, cx| ws.close_context_menu(cx));
            }
        },
    );

    let workspace_for_description = workspace.clone();
    let edit_description_item = ContextMenuItem::new(
        surface_strings::CTX_EDIT_DESCRIPTION,
        move |_ev: &MouseDownEvent, window: &mut Window, app_cx: &mut App| {
            if let Some(ws) = workspace_for_description.upgrade() {
                let current = current_description.clone();
                let callback_ws = workspace_for_description.clone();
                ws.update(app_cx, |ws, cx| {
                    ws.close_context_menu(cx);
                    crate::workspace::dialog_helpers::open_single_field_dialog(
                        callback_ws.clone(),
                        surface_strings::EDIT_DESCRIPTION_MODAL_TITLE,
                        surface_strings::EDIT_DESCRIPTION_PLACEHOLDER,
                        current.as_deref(),
                        move |workspace, value, _window, cx| {
                            workspace.set_worktree_description(wt_id, value, cx);
                        },
                        window,
                        cx,
                    );
                });
            }
        },
    );

    let workspace_for_rename = workspace.clone();
    let rename_item = ContextMenuItem::new(
        surface_strings::CTX_RENAME,
        move |_ev: &MouseDownEvent, window: &mut Window, app_cx: &mut App| {
            if let Some(ws) = workspace_for_rename.upgrade() {
                let current = current_name.clone();
                let callback_ws = workspace_for_rename.clone();
                ws.update(app_cx, |ws, cx| {
                    ws.close_context_menu(cx);
                    crate::workspace::dialog_helpers::open_single_field_dialog(
                        callback_ws.clone(),
                        surface_strings::RENAME_MODAL_TITLE,
                        surface_strings::RENAME_PLACEHOLDER,
                        current.as_deref(),
                        move |workspace, value, _window, cx| {
                            workspace.set_worktree_name(wt_id, value, cx);
                        },
                        window,
                        cx,
                    );
                });
            }
        },
    );

    let mut items = vec![reveal_item, copy_item, edit_description_item, rename_item];

    // "Merge into…" — only for git-backed worktrees.
    if is_git {
        let merge_item = if is_detached {
            ContextMenuItem::new(surface_strings::CTX_MERGE_INTO, |_, _, _| {})
                .disabled(true)
                .with_tooltip(surface_strings::CTX_MERGE_DISABLED_DETACHED)
        } else if is_dirty {
            ContextMenuItem::new(surface_strings::CTX_MERGE_INTO, |_, _, _| {})
                .disabled(true)
                .with_tooltip(surface_strings::CTX_MERGE_DISABLED_DIRTY)
        } else {
            // source_branch is guaranteed Some when is_git && !is_detached.
            let branch = source_branch.unwrap_or_default();
            let workspace_for_merge = workspace.clone();
            ContextMenuItem::new(
                surface_strings::CTX_MERGE_INTO,
                move |_ev: &MouseDownEvent, window: &mut Window, app_cx: &mut App| {
                    let Some(ws) = workspace_for_merge.upgrade() else {
                        return;
                    };
                    let branch = branch.clone();
                    let base_ref = base_ref.clone();
                    let workspace_weak = workspace_for_merge.clone();
                    // Clone before ws.update so the outer Fn closure can
                    // be called again without violating the Fn constraint.
                    let src_path = source_path.clone();
                    let src_repo_root = source_repo_root.clone().unwrap_or_default();
                    ws.update(app_cx, |ws, cx| {
                        ws.close_context_menu(cx);

                        // Build target list: other git worktrees with a branch.
                        let target_options: Vec<super::merge_modal::TargetOption> = ws
                            .active_worktrees()
                            .iter()
                            .filter(|w| w.id != wt_id)
                            .filter_map(|w| match &w.kind {
                                daruda_store::project::WorktreeKind::Git {
                                    branch: Some(b),
                                    ..
                                } => Some(super::merge_modal::TargetOption {
                                    wt_id: w.id,
                                    branch: b.clone(),
                                    wt_path: w.path.clone(),
                                }),
                                _ => None,
                            })
                            .collect();

                        if target_options.is_empty() {
                            let report = ErrorReport::new(surface_strings::MERGE_MODAL_NO_TARGETS)
                                .severity(ErrorSeverity::Info)
                                .at(file!(), line!())
                                .dedup("worktree.merge.no_targets")
                                .build();
                            ws.report_error(report, cx);
                            return;
                        }

                        crate::workspace::dialog_helpers::open_form_modal(
                            SharedString::from(format!("Merge \"{branch}\" into")),
                            None,
                            move |window, cx| {
                                super::merge_modal::MergeModal::new(
                                    wt_id,
                                    branch.clone(),
                                    src_path,
                                    src_repo_root,
                                    target_options,
                                    base_ref.clone(),
                                    workspace_weak.clone(),
                                    window,
                                    cx,
                                )
                            },
                            window,
                            cx,
                        );
                    });
                },
            )
        };
        items.push(ContextMenuItem::separator());
        items.push(merge_item);
    }

    items
}

fn non_git_placeholder(cx: &gpui::App) -> impl IntoElement {
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

fn empty_state(cx: &gpui::App) -> impl IntoElement {
    div()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(theme::DOCK_PLACEHOLDER_FONT_SIZE))
        .text_color(theme::current(cx).dock_placeholder_text)
        .child(surface_strings::WORKTREES_EMPTY_STATE)
}
