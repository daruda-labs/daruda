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
use crate::ui::{ButtonVariants as _, ContextMenuItem, IconName, SectionHeader, button_bare};
use crate::workspace::NewGroup;
use crate::workspace::layout::{Dock, GroupSnapshot, LeftDockSnapshot};
use crate::worktree::Worktree;

use super::claude_badges::{claude_badges_row, claude_status_cell};
use super::context_menu::build_context_menu_items;
use super::drag::{DragGhost, DragPayload};
use crate::workspace::dnd_ops::TopRow;

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
    let drop_target_bg = t.worktree_drop_target_bg;
    let drop_target_rejected_bg = t.worktree_drop_target_rejected_bg;

    // Color dot rendered at the left of the group row (right-aligned
    // chevron carries the collapse state, so color is its own glyph
    // again). Silently skipped when the stored value is not parseable
    // hex.
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

    let caret_icon = if group.is_collapsed {
        IconName::ChevronRight
    } else {
        IconName::ChevronDown
    };

    let group_id = group.id;
    let group_name_for_menu = group.name.clone();
    let group_is_collapsed = group.is_collapsed;
    let workspace = snap.workspace.clone();
    let ws_for_drop = workspace.clone();
    let ws_for_menu = workspace.clone();
    let ws_for_chevron = workspace.clone();
    let drag_payload = DragPayload::Group {
        id: group_id,
        label: group.name.clone(),
    };
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
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |_dock, ev: &MouseDownEvent, _window, cx| {
                cx.stop_propagation();
                let position: Point<Pixels> = ev.position;
                let Some(ws) = ws_for_menu.upgrade() else {
                    return;
                };
                let items = super::group_menu::build_group_menu_items(
                    group_id,
                    group_name_for_menu.clone(),
                    group_is_collapsed,
                    ws_for_menu.clone(),
                );
                ws.update(cx, |ws, cx| ws.open_context_menu(position, items, cx));
            }),
        )
        .on_drag(drag_payload, |dragged, _offset, _window, cx| {
            cx.new(|_| DragGhost {
                label: dragged.label(),
            })
        })
        // Group header accepts:
        //   - any Project (becomes the last child of this group)
        //   - a different Group (reorder at top level)
        // Worktree payloads are rejected — a worktree never leaves its
        // project, so a group header is never a valid target for it.
        // Rejected payloads still get a tint (rejected_bg) so the user
        // sees "the row noticed me but won't accept the drop" instead
        // of mistaking silent absence-of-highlight for a working drop
        // target.
        .drag_over::<DragPayload>(move |style, dragged, _window, _cx| match dragged {
            DragPayload::Project { .. } => style.bg(drop_target_bg),
            DragPayload::Group { id, .. } if *id != group_id => style.bg(drop_target_bg),
            _ => style.bg(drop_target_rejected_bg),
        })
        .on_drop(
            cx.listener(move |_dock, dragged: &DragPayload, _window, cx| {
                let Some(ws) = ws_for_drop.upgrade() else {
                    return;
                };
                match dragged {
                    DragPayload::Project { id: from, .. } => {
                        let from = *from;
                        ws.update(cx, |ws, cx| {
                            ws.move_project_to_group_end(from, group_id, cx)
                        });
                    }
                    DragPayload::Group { id: from, .. } if *from != group_id => {
                        let from = *from;
                        ws.update(cx, |ws, cx| {
                            ws.reorder_group_before_top_row(from, TopRow::Group(group_id), cx)
                        });
                    }
                    _ => {}
                }
            }),
        )
        .when_some(color_dot, |d, dot| d.child(dot))
        .child(
            div()
                .flex_1()
                .overflow_hidden()
                .whitespace_nowrap()
                .child(group.name.clone()),
        )
        .child(
            // Chevron uses `button_bare` so its chrome (ghost padding /
            // hover / hit area) matches every `[+]` button on the row.
            // Row click already toggles via the outer `on_click`, but
            // routing the chevron through its own button keeps the
            // header visually uniform with project rows.
            button_bare(("group-chevron", group_id as usize))
                .ghost()
                .icon(caret_icon)
                .on_click(cx.listener(move |_dock, _: &ClickEvent, _window, cx| {
                    cx.stop_propagation();
                    if let Some(ws) = ws_for_chevron.upgrade() {
                        ws.update(cx, |ws, cx| ws.toggle_group_collapse(group_id, cx));
                    }
                })),
        )
}

/// Single-row project header above the worktrees list for one
/// project. Drag source for `DragPayload::Project` and drop target for
/// project / group payloads. The `is_ungrouped` flag gates whether a
/// dragged group can land here — groups only sit at the top level, so
/// dropping a group on a project nested inside another group must
/// silently no-op.
///
/// `is_git` enables the trailing `[+]` button (per-project "create
/// worktree" affordance — the previous `+ new worktree` row at the
/// bottom of the worktree list was folded into this header so the
/// project row carries every project-scoped action).
///
/// `is_collapsed` flips the chevron between `ChevronDown` (expanded)
/// and `ChevronRight` (collapsed). The chevron carries its own click
/// handler that toggles the flag; the rest of the row stays bound to
/// `activate_worktree(last_active)` so a header click still snaps the
/// focus per §5.5.
#[allow(clippy::too_many_arguments)]
pub(in crate::workspace) fn project_header_row(
    project_id: ProjectId,
    name: SharedString,
    is_ungrouped: bool,
    is_active: bool,
    is_git: bool,
    is_collapsed: bool,
    last_active_worktree_id: WorktreeId,
    snap: &LeftDockSnapshot,
    cx: &mut Context<Dock>,
) -> impl IntoElement + use<> {
    let t = theme::current(cx);
    let label_color = if is_active {
        t.dock_view_tab_active
    } else {
        t.muted_text
    };
    let row_hover_bg = t.worktree_row_hover_bg;
    let drop_target_bg = t.worktree_drop_target_bg;
    let drop_target_rejected_bg = t.worktree_drop_target_rejected_bg;

    let workspace = snap.workspace.clone();
    let ws_for_click = workspace.clone();
    let ws_for_drop = workspace.clone();
    let ws_for_menu = workspace.clone();
    let drag_payload = DragPayload::Project {
        id: project_id,
        label: name.clone(),
    };

    let row_group = SharedString::from(format!("project-row-{project_id}"));
    div()
        .id(("project-header", project_id as usize))
        .group(row_group.clone())
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::WORKTREE_LABEL_GAP))
        .w_full()
        .px(px(theme::WORKTREE_ROW_PAD_X))
        .py(px(theme::WORKTREE_SECTION_PAD_Y))
        .text_size(px(theme::WORKTREE_LABEL_FONT_SIZE))
        .text_color(label_color)
        .cursor_pointer()
        .when(!is_active, move |d| d.hover(move |d| d.bg(row_hover_bg)))
        // Header click snaps the workspace focus to this project's
        // last-active worktree (§5.5). No-op when the click lands on
        // the already-active project — the snap target would equal
        // the current focus and `activate_worktree` short-circuits.
        .on_click(cx.listener(move |_dock, _: &ClickEvent, window, cx| {
            if let Some(ws) = ws_for_click.upgrade() {
                let target = daruda_store::project::WorktreeRef {
                    project: project_id,
                    worktree: last_active_worktree_id,
                };
                ws.update(cx, |ws, cx| ws.activate_worktree(target, window, cx));
            }
        }))
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |_dock, ev: &MouseDownEvent, _window, cx| {
                cx.stop_propagation();
                let position: Point<Pixels> = ev.position;
                let Some(ws) = ws_for_menu.upgrade() else {
                    return;
                };
                let items = super::project_menu::build_project_menu_items(
                    project_id,
                    last_active_worktree_id,
                    ws_for_menu.clone(),
                );
                ws.update(cx, |ws, cx| ws.open_context_menu(position, items, cx));
            }),
        )
        .on_drag(drag_payload, |dragged, _offset, _window, cx| {
            cx.new(|_| DragGhost {
                label: dragged.label(),
            })
        })
        // Project header accepts:
        //   - a different Project (reorder, inheriting this project's
        //     group membership);
        //   - a Group, but only when this project is itself ungrouped
        //     (groups live at the top level and may only interleave
        //     with ungrouped projects there).
        // Worktree payloads, group-on-grouped-project, and self-project
        // drops fall through to the rejected tint so the user sees the
        // row noticed the drag but won't accept it.
        .drag_over::<DragPayload>(move |style, dragged, _window, _cx| match dragged {
            DragPayload::Project { id: from, .. } if *from != project_id => {
                style.bg(drop_target_bg)
            }
            DragPayload::Group { .. } if is_ungrouped => style.bg(drop_target_bg),
            _ => style.bg(drop_target_rejected_bg),
        })
        .on_drop(
            cx.listener(move |_dock, dragged: &DragPayload, _window, cx| {
                let Some(ws) = ws_for_drop.upgrade() else {
                    return;
                };
                match dragged {
                    DragPayload::Project { id: from, .. } if *from != project_id => {
                        let from = *from;
                        ws.update(cx, |ws, cx| ws.reorder_project_before(from, project_id, cx));
                    }
                    DragPayload::Group { id: from, .. } if is_ungrouped => {
                        let from = *from;
                        ws.update(cx, |ws, cx| {
                            ws.reorder_group_before_top_row(from, TopRow::Project(project_id), cx)
                        });
                    }
                    _ => {}
                }
            }),
        )
        .child({
            let ws_for_chevron = snap.workspace.clone();
            let chevron_icon = if is_collapsed {
                IconName::ChevronRight
            } else {
                IconName::ChevronDown
            };
            // Same `button_bare` chrome as the group chevron + `[+]`
            // button on this row — chevron toggles the project's
            // collapsed flag while the rest of the row stays bound to
            // `activate_worktree` (snap to last_active).
            button_bare(("project-chevron", project_id as usize))
                .ghost()
                .icon(chevron_icon)
                .on_click(cx.listener(move |_dock, _: &ClickEvent, _window, cx| {
                    cx.stop_propagation();
                    if let Some(ws) = ws_for_chevron.upgrade() {
                        ws.update(cx, |ws, cx| ws.toggle_project_collapse(project_id, cx));
                    }
                }))
        })
        .child(
            div()
                .flex_1()
                .overflow_hidden()
                .whitespace_nowrap()
                .child(name),
        )
        .when(is_git, |row| {
            let ws_for_add = snap.workspace.clone();
            let row_group_for_btn = row_group.clone();
            row.child(
                button_bare(("project-add-worktree", project_id as usize))
                    .ghost()
                    .icon(IconName::Plus)
                    .invisible()
                    .group_hover(row_group_for_btn, |this| this.visible())
                    .on_click(cx.listener(move |_dock, _: &ClickEvent, window, cx| {
                        // Stop the row activate handler so the [+]
                        // doesn't double-fire as a "snap to project"
                        // click.
                        cx.stop_propagation();
                        if let Some(ws) = ws_for_add.upgrade() {
                            let workspace_for_modal = ws_for_add.clone();
                            ws.update(cx, |ws, cx| {
                                // Activate this project first so
                                // `finalize_create_worktree` lands the
                                // new worktree under it (matches the
                                // old `add_worktree_row` semantics).
                                let target = daruda_store::project::WorktreeRef {
                                    project: project_id,
                                    worktree: last_active_worktree_id,
                                };
                                ws.activate_worktree(target, window, cx);
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
                    })),
            )
        })
}

pub(in crate::workspace) fn section_header(
    _any_git: bool,
    snap: &LeftDockSnapshot,
    cx: &mut Context<Dock>,
) -> impl IntoElement + use<> {
    // Section-level `[+]` is a toggle: clicking it opens a flat
    // context menu with "Add Project…" / "New Group…" so the user can
    // pick between adding a project (folder picker routed through
    // `prompt_and_open_folder_with_policy`, policy-aware) and
    // creating a group (previously only reachable via Cmd+Shift+N or
    // the Command Palette). Per-project worktree creation lives on
    // each project's `[+ new worktree]` row.
    let workspace = snap.workspace.clone();
    let add_button = button_bare("section-add-toggle")
        .ghost()
        .icon(IconName::Plus)
        .on_click(cx.listener(move |_dock, ev: &ClickEvent, _window, cx| {
            let position: Point<Pixels> = ev.position();
            let ws_for_project = workspace.clone();
            let ws_for_group = workspace.clone();
            let items = vec![
                ContextMenuItem::new(
                    surface_strings::SECTION_ADD_MENU_PROJECT,
                    move |_, _window, app_cx| {
                        // No window context needed — the global
                        // open-folder flow drives the picker async.
                        let _ = ws_for_project;
                        let config =
                            crate::settings_store::SettingsStore::global(app_cx).user_arc();
                        crate::windows::prompt_and_open_folder_with_policy(config, app_cx);
                    },
                ),
                ContextMenuItem::new(
                    surface_strings::SECTION_ADD_MENU_GROUP,
                    move |_, window, app_cx| {
                        if let Some(ws) = ws_for_group.upgrade() {
                            ws.update(app_cx, |ws, cx| {
                                ws.on_new_group(&NewGroup, window, cx);
                            });
                        }
                    },
                ),
            ];
            if let Some(ws) = workspace.upgrade() {
                ws.update(cx, |ws, cx| ws.open_context_menu(position, items, cx));
            }
        }));

    SectionHeader::new(surface_strings::WORKTREES_SECTION_HEADER)
        .padding(theme::WORKTREE_ROW_PAD_X, theme::WORKTREE_SECTION_PAD_Y)
        .actions(add_button)
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
    let drop_target_rejected_bg = t.worktree_drop_target_rejected_bg;

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
    let wt_ref = daruda_store::project::WorktreeRef {
        project: project_id,
        worktree: wt_id,
    };
    let drag_payload = DragPayload::Worktree {
        target: wt_ref,
        label: wt_label_shared.clone(),
    };

    let ws_for_click = workspace.clone();
    let ws_for_drop = workspace.clone();
    let ws_for_rclick = workspace.clone();
    // Worktree IDs reset to 0 per project (`Worktree::bootstrap_from_project`
    // numbers each project's worktrees from 0), so `("worktree-row", wt_id)`
    // collides across projects — GPUI sees the same ElementId for the first
    // worktree of every project and routes the click to only one of them
    // (the first project's row), which is why clicking a 2nd project's
    // worktree never reaches `activate_worktree`. Encode both ids into the
    // id string so each row is uniquely addressable.
    let row_group = SharedString::from(format!("worktree-row-{project_id}-{wt_id}"));
    let mut row = div()
        .id(SharedString::from(format!(
            "worktree-row-{project_id}-{wt_id}"
        )))
        .group(row_group.clone())
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
            cx.new(|_| DragGhost {
                label: dragged.label(),
            })
        })
        // Drop target — highlight only when the in-flight payload is a
        // worktree from the same project. Cross-project worktrees, plus
        // any project/group payload, are rejected — drawn with the
        // rejected tint so the user sees the row noticed the drag but
        // won't accept it (the workspace op would silently discard
        // these without the tint).
        .drag_over::<DragPayload>(move |style, dragged, _window, _cx| match dragged {
            DragPayload::Worktree { target, .. } if target.project == project_id => {
                style.bg(drop_target_bg)
            }
            _ => style.bg(drop_target_rejected_bg),
        })
        .on_drop(
            cx.listener(move |_dock, dragged: &DragPayload, _window, cx| {
                if let DragPayload::Worktree { target, .. } = dragged
                    && let Some(ws) = ws_for_drop.upgrade()
                {
                    let from = *target;
                    ws.update(cx, |ws, cx| ws.reorder_worktree(from, wt_ref, cx));
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
        let row_group_for_close = row_group.clone();
        let close = button_bare(SharedString::from(format!(
            "wt-remove-{project_id}-{wt_id}"
        )))
        .ghost()
        .icon(IconName::Close)
        .invisible()
        .group_hover(row_group_for_close, |this| this.visible())
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
