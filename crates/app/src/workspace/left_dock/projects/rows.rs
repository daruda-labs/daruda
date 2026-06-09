//! Visible row primitives for the lanes view:
//! `section_header`, `worktree_row`, the small `LaneId → GitBadge`
//! derivation, and the non-git placeholder hint.

use crate::ui::theme;
use daruda_terminal::ux::strings;
use gpui::{
    ClickEvent, Context, IntoElement, MouseButton, MouseDownEvent, Pixels, Point, Rgba,
    SharedString, div, prelude::*, px,
};

use daruda_store::project::{LaneId, ProjectId};

use crate::lane::Lane;
use crate::surface::strings as surface_strings;
use crate::ui::{
    ButtonVariants as _, ContextMenuItem, Icon, IconName, SectionHeader, Sizable as _, button_bare,
};
use crate::workspace::NewGroup;
use crate::workspace::layout::{Dock, GroupSnapshot, LeftDockSnapshot, ProjectSnapshot};

use super::claude_badges::{claude_badges_row, claude_status_cell};
use super::context_menu::build_context_menu_items;
use super::drag::{DragGhost, DragPayload};
use crate::workspace::dnd_ops::TopRow;

/// Compact summary of a lane's git status for the row badge — rolled
/// up into a single change count plus ahead/behind divergence so the
/// dock row can render a GitHub-Desktop-style chip
/// (`↑N ↓N [total]`). `None` means "show nothing" (clean tree + no
/// divergence, or no status cached yet).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::workspace) struct GitBadgeData {
    /// Modified + staged + untracked, rolled up. The previous design
    /// split these into `M` / `?` letters; the GH-Desktop pill collapses
    /// them into a single count so the chip stays narrow.
    pub total: u32,
    /// Commits ahead of the configured upstream (zero when no upstream
    /// is set or HEAD is detached).
    pub ahead: u32,
    /// Commits behind the configured upstream.
    pub behind: u32,
}

/// Build the GH-Desktop-style change/divergence summary for lane
/// `id` from the cached git status. Returns `None` when no status is
/// cached yet or the lane is fully clean (no changes + no
/// divergence) so the row stays uncluttered.
pub(in crate::workspace) fn git_badge_for(
    snap: &LeftDockSnapshot,
    project_id: ProjectId,
    id: LaneId,
) -> Option<GitBadgeData> {
    let target = daruda_store::project::LaneRef {
        project: project_id,
        lane: id,
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
    let total = (modified + untracked) as u32;
    if total == 0 && status.ahead == 0 && status.behind == 0 {
        return None;
    }
    Some(GitBadgeData {
        total,
        ahead: status.ahead,
        behind: status.behind,
    })
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
    let label_color = theme::TEXT_PRIMARY;
    let row_hover_bg = t.lane_row_hover_bg;
    let drop_target_bg = t.lane_drop_target_bg;
    let drop_target_rejected_bg = t.lane_drop_target_rejected_bg;

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
                .w(px(theme::LANE_GROUP_COLOR_DOT_SIZE))
                .h(px(theme::LANE_GROUP_COLOR_DOT_SIZE))
                .rounded(px(theme::LANE_GROUP_COLOR_DOT_RADIUS))
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
    let row_group_key = SharedString::from(format!("group-row-{group_id}"));

    let label_row = div()
        .flex()
        .flex_row()
        .items_center()
        .w_full()
        .gap(px(theme::LANE_LABEL_GAP))
        .text_size(px(theme::LANE_GROUP_LABEL_FONT_SIZE))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(label_color)
        .when_some(color_dot, |d, dot| d.child(dot))
        .child(
            div()
                .flex_1()
                .overflow_hidden()
                .whitespace_nowrap()
                .child(SharedString::from(group.name.to_uppercase())),
        )
        .child(
            // Chevron uses `button_bare` so its chrome (ghost padding /
            // hover / hit area) matches every `[+]` button on the row.
            // Hidden until row hover; collapsed groups keep it visible
            // as an expand affordance.
            button_bare(("group-chevron", group_id as usize))
                .ghost()
                .icon(caret_icon)
                .invisible()
                .group_hover(row_group_key.clone(), |this| this.visible())
                .when(group_is_collapsed, |b| b.visible())
                .on_click(cx.listener(move |_dock, _: &ClickEvent, _window, cx| {
                    cx.stop_propagation();
                    if let Some(ws) = ws_for_chevron.upgrade() {
                        ws.update(cx, |ws, cx| ws.toggle_group_collapse(group_id, cx));
                    }
                })),
        );

    div()
        .id(("group-header", group_id as usize))
        .group(row_group_key.clone())
        .flex()
        .flex_col()
        .w_full()
        .px(px(theme::LANE_ROW_PAD_X))
        .py(px(theme::LANE_SECTION_PAD_Y))
        .rounded(px(theme::LANE_ROW_RADIUS))
        .cursor_pointer()
        // Active highlight is expressed by the wrapping `group_card`
        // (see `card::group_card`), so the header row only carries the
        // hover lift. Painting an active bg here too would double-up
        // with the card fill and read as a brighter inner chip.
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
        // Lane payloads are rejected — a lane never leaves its
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
        .child(label_row)
}

/// Per-row state passed into [`project_header_row`].
///
/// Keeps the seven row-shape inputs (project id, name, group/active/git
/// flags, collapsed flag, and the snap-target lane) grouped at the
/// call site so the header signature stays readable as additional
/// per-project flags accumulate.
pub(in crate::workspace) struct ProjectHeaderArgs {
    pub project_id: ProjectId,
    pub name: SharedString,
    pub is_ungrouped: bool,
    pub is_active: bool,
    pub is_git: bool,
    pub is_collapsed: bool,
    pub last_active_lane_id: LaneId,
}

/// Single-row project header above the lanes list for one
/// project. Drag source for `DragPayload::Project` and drop target for
/// project / group payloads. The `is_ungrouped` flag gates whether a
/// dragged group can land here — groups only sit at the top level, so
/// dropping a group on a project nested inside another group must
/// silently no-op.
///
/// `is_git` enables the trailing `[+]` button (per-project "create
/// lane" affordance — the previous `+ new lane` row at the
/// bottom of the lane list was folded into this header so the
/// project row carries every project-scoped action).
///
/// `is_collapsed` flips the chevron between `ChevronDown` (expanded)
/// and `ChevronRight` (collapsed). The chevron carries its own click
/// handler that toggles the flag; the rest of the row stays bound to
/// `activate_lane(last_active)` so a header click still snaps the
/// focus per §5.5.
pub(in crate::workspace) fn project_header_row(
    args: ProjectHeaderArgs,
    snap: &LeftDockSnapshot,
    cx: &mut Context<Dock>,
) -> impl IntoElement + use<> {
    let ProjectHeaderArgs {
        project_id,
        name,
        is_ungrouped,
        is_active,
        is_git,
        is_collapsed,
        last_active_lane_id,
    } = args;
    let t = theme::current(cx);
    let label_color = if is_active {
        theme::TEXT_PRIMARY
    } else {
        t.muted_text
    };
    let row_hover_bg = t.lane_row_hover_bg;
    let row_active_bg = t.lane_card_active_bg;
    let drop_target_bg = t.lane_drop_target_bg;
    let drop_target_rejected_bg = t.lane_drop_target_rejected_bg;
    // Active highlight lands on this header only when the project is
    // ungrouped — grouped projects rely on the wrapping `group_card`
    // active fill instead, so painting an inner row chip would double
    // up the highlight.
    let show_active_bg = is_active && is_ungrouped;

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
        .gap(px(theme::LANE_LABEL_GAP))
        .w_full()
        .px(px(theme::LANE_ROW_PAD_X))
        .py(px(theme::LANE_SECTION_PAD_Y))
        .rounded(px(theme::LANE_ROW_RADIUS))
        .text_size(px(theme::LANE_LABEL_FONT_SIZE))
        .text_color(label_color)
        .cursor_pointer()
        .when(!show_active_bg, move |d| {
            d.hover(move |d| d.bg(row_hover_bg))
        })
        .when(show_active_bg, move |d| d.bg(row_active_bg))
        // Header click snaps the workspace focus to this project's
        // last-active lane (§5.5). No-op when the click lands on
        // the already-active project — the snap target would equal
        // the current focus and `activate_lane` short-circuits.
        .on_click(cx.listener(move |_dock, _: &ClickEvent, window, cx| {
            if let Some(ws) = ws_for_click.upgrade() {
                let target = daruda_store::project::LaneRef {
                    project: project_id,
                    lane: last_active_lane_id,
                };
                ws.update(cx, |ws, cx| ws.activate_lane(target, window, cx));
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
                    last_active_lane_id,
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
        // Lane payloads, group-on-grouped-project, and self-project
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
            // button on this row. Hidden until row hover; collapsed
            // projects keep it visible as an expand affordance.
            let row_group_for_chev = row_group.clone();
            button_bare(("project-chevron", project_id as usize))
                .ghost()
                .icon(chevron_icon)
                .invisible()
                .group_hover(row_group_for_chev, |this| this.visible())
                .when(is_collapsed, |b| b.visible())
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
                button_bare(("project-add-lane", project_id as usize))
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
                                // `git_repo_root()` returns the right
                                // repo when the modal reads it at open
                                // time.
                                let target = daruda_store::project::LaneRef {
                                    project: project_id,
                                    lane: last_active_lane_id,
                                };
                                ws.activate_lane(target, window, cx);
                                let Some(repo_root) = ws.git_repo_root() else {
                                    return;
                                };
                                crate::workspace::dialog_helpers::open_form_modal(
                                    "Create Lane",
                                    None,
                                    move |window, cx| {
                                        super::create_modal::CreateWorktreeModal::new(
                                            workspace_for_modal.clone(),
                                            repo_root,
                                            project_id,
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
    // the Command Palette). Per-project lane creation lives on
    // each project's `[+ new lane]` row.
    let workspace = snap.workspace.clone();
    let add_button = button_bare("section-add-toggle")
        .ghost()
        .icon(IconName::Plus)
        .on_click(cx.listener(move |_dock, ev: &ClickEvent, _window, cx| {
            let position: Point<Pixels> = ev.position();
            let ws_for_group = workspace.clone();
            let items = vec![
                ContextMenuItem::new(
                    surface_strings::section_add_menu_project(),
                    move |_, _window, app_cx| {
                        // No workspace handle needed — the global open-folder
                        // flow reads its config from `SettingsStore` and the
                        // resulting picker runs async without a captured
                        // entity.
                        let config =
                            crate::settings_store::SettingsStore::global(app_cx).user_arc();
                        crate::windows::prompt_and_open_folder_with_policy(config, app_cx);
                    },
                ),
                ContextMenuItem::new(
                    surface_strings::section_add_menu_group(),
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

    SectionHeader::new(surface_strings::projects_section_header())
        .padding(theme::LANE_ROW_PAD_X, theme::LANE_SECTION_PAD_Y)
        .actions(add_button)
}

/// GitHub-Desktop-style badge cluster rendered on the right of a
/// lane row's label. Layout (left to right, each group omitted when
/// its count is zero):
///
/// - `▲N` — ahead arrow + count
/// - `▼N` — behind arrow + count
/// - `[N]` — rolled-up change count inside a neutral grey pill
///
/// Color tokens are passed in so the parent can snapshot the live
/// theme once per render and reuse the values across rows without
/// touching `cx` again here.
fn git_badge_view(
    badge: GitBadgeData,
    arrow_color: gpui::Hsla,
    pill_bg: gpui::Hsla,
    pill_text: gpui::Hsla,
) -> impl IntoElement {
    let arrow_size = px(theme::GIT_BADGE_ARROW_SIZE);
    let arrow_group = move |icon: IconName, count: u32| {
        div()
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(theme::GIT_BADGE_ARROW_NUM_GAP))
            .text_size(px(theme::GIT_BADGE_FONT_SIZE))
            .text_color(arrow_color)
            .child(
                Icon::new(icon)
                    .with_size(arrow_size)
                    .text_color(arrow_color),
            )
            .child(format!("{count}"))
    };

    div()
        .flex_none()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::GIT_BADGE_GAP))
        .when(badge.ahead > 0, |d| {
            d.child(arrow_group(IconName::ArrowUp, badge.ahead))
        })
        .when(badge.behind > 0, |d| {
            d.child(arrow_group(IconName::ArrowDown, badge.behind))
        })
        .when(badge.total > 0, |d| {
            d.child(
                div()
                    .flex_none()
                    .min_w(px(theme::GIT_BADGE_PILL_MIN_W))
                    .px(px(theme::GIT_BADGE_PILL_PAD_X))
                    .py(px(theme::GIT_BADGE_PILL_PAD_Y))
                    .rounded(px(theme::GIT_BADGE_PILL_RADIUS))
                    .bg(pill_bg)
                    .text_size(px(theme::GIT_BADGE_FONT_SIZE))
                    .text_color(pill_text)
                    .text_center()
                    .child(format!("{}", badge.total)),
            )
        })
}

pub(in crate::workspace) fn worktree_row(
    wt: &Lane,
    project_id: ProjectId,
    is_active: bool,
    git_badge: Option<GitBadgeData>,
    snap: &LeftDockSnapshot,
    cx: &mut Context<Dock>,
) -> impl IntoElement {
    // Snapshot every row colour from the live theme so the closures
    // below (hover, drag_over) capture stable values.
    let t = theme::current(cx);
    let unread_dot_color = theme::WARNING;
    let label_active = theme::TEXT_PRIMARY;
    let label_inactive = t.muted_text;
    let badge_pill_bg = theme::OVERLAY_PROMINENT;
    let badge_pill_text = theme::TEXT_PRIMARY;
    let badge_arrow_text = theme::TEXT_TERTIARY;
    let sublabel_color = t.faint_text;
    let row_hover_bg = t.lane_row_hover_bg;
    let row_active_bg = theme::OVERLAY_ACTIVE;
    let drop_target_bg = t.lane_drop_target_bg;
    let drop_target_rejected_bg = t.lane_drop_target_rejected_bg;

    let label = wt.display_name();
    // Sublabel priority: user-set description → path.
    let sublabel = wt
        .description
        .clone()
        .unwrap_or_else(|| wt.path.to_str().map(|s| s.to_string()).unwrap_or_default());

    // Unread marker sits to the right of the label.
    let unread_dot = div()
        .flex_none()
        .w(px(theme::LANE_UNREAD_DOT_SIZE))
        .h(px(theme::LANE_UNREAD_DOT_SIZE))
        .rounded(px(theme::LANE_UNREAD_DOT_RADIUS))
        .bg(unread_dot_color);

    // Body — label row + sublabel row + optional Claude multi-session sub-row.
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
                .gap(px(theme::LANE_LABEL_GAP))
                .text_size(px(theme::LANE_LABEL_FONT_SIZE))
                .text_color(if is_active {
                    label_active
                } else {
                    label_inactive
                })
                .child(
                    div()
                        .flex_1()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .child(label.clone()),
                )
                .when(wt.is_unread, |d| d.child(unread_dot))
                .when_some(git_badge, |d, badge| {
                    d.child(git_badge_view(
                        badge,
                        badge_arrow_text,
                        badge_pill_bg,
                        badge_pill_text,
                    ))
                }),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(theme::LANE_SUBLABEL_GAP))
                .text_size(px(theme::LANE_SUB_FONT_SIZE))
                .text_color(sublabel_color)
                .overflow_hidden()
                .whitespace_nowrap()
                .child(sublabel),
        )
        .when_some(
            snap.claude_per_session_per_lane
                .get(&daruda_store::project::LaneRef {
                    project: project_id,
                    lane: wt.id,
                }),
            |d, sessions| {
                d.child(claude_badges_row(
                    sessions,
                    snap.claude_active_session_id.as_deref(),
                    cx,
                ))
            },
        );

    let wt_id: LaneId = wt.id;
    let wt_path: std::path::PathBuf = wt.path.clone();
    let wt_label_shared = SharedString::from(label.clone());
    let wt_description_current: Option<String> = wt.description.clone();
    let wt_name_current: Option<String> = wt.name.clone();
    let removable = crate::workspace::Workspace::lane_removable(wt);

    // Merge context menu state — captured before closures so all
    // values are 'static (no borrow of `wt` or `snap` inside closures).
    let wt_is_git = matches!(&wt.kind, daruda_store::project::LaneKind::Git { .. });
    let wt_is_detached = matches!(
        &wt.kind,
        daruda_store::project::LaneKind::Git { branch: None, .. }
    );
    let wt_source_branch: Option<String> = match &wt.kind {
        daruda_store::project::LaneKind::Git {
            branch: Some(b), ..
        } => Some(b.clone()),
        _ => None,
    };
    let wt_base_ref: Option<String> = wt.base_ref.clone();
    let wt_is_dirty = snap
        .git_status_cache
        .get(&daruda_store::project::LaneRef {
            project: project_id,
            lane: wt.id,
        })
        .is_some_and(|s| !s.staged.is_empty() || !s.unstaged.is_empty());
    let wt_source_repo_root: Option<std::path::PathBuf> = match &wt.kind {
        daruda_store::project::LaneKind::Git { repo_root, .. } => Some(repo_root.clone()),
        _ => None,
    };
    let workspace = snap.workspace.clone();

    // Drag payload — captured once so on_drag + on_drop share the same Arc.
    let wt_ref = daruda_store::project::LaneRef {
        project: project_id,
        lane: wt_id,
    };
    let drag_payload = DragPayload::Lane {
        target: wt_ref,
        label: wt_label_shared.clone(),
    };

    let ws_for_click = workspace.clone();
    let ws_for_drop = workspace.clone();
    let ws_for_rclick = workspace.clone();
    // Lane IDs reset to 0 per project (`Lane::bootstrap_from_project`
    // numbers each project's lanes from 0), so `("lane-row", wt_id)`
    // collides across projects — GPUI sees the same ElementId for the first
    // lane of every project and routes the click to only one of them
    // (the first project's row), which is why clicking a 2nd project's
    // lane never reaches `activate_lane`. Encode both ids into the
    // id string so each row is uniquely addressable.
    let row_group = SharedString::from(format!("lane-row-{project_id}-{wt_id}"));
    let mut row = div()
        .id(SharedString::from(format!("lane-row-{project_id}-{wt_id}")))
        .group(row_group.clone())
        .flex()
        .flex_row()
        .items_center()
        // Vertical padding matches the project/group header rows
        // (`LANE_SECTION_PAD_Y`) so all three row kinds share the
        // same breathing room regardless of body height — a `min_h`
        // approach collapsed to zero padding once the Claude
        // multi-session sub-row grew the row past the floor.
        .px(px(theme::LANE_ROW_PAD_X))
        .py(px(theme::LANE_SECTION_PAD_Y))
        .gap(px(theme::LANE_ROW_GAP))
        .rounded(px(theme::LANE_ROW_RADIUS))
        .cursor_pointer()
        .when(!is_active, move |d| d.hover(move |d| d.bg(row_hover_bg)))
        .when(is_active, move |d| d.bg(row_active_bg))
        // on_click fires only when the mousedown + mouseup happen at the
        // same position (no drag movement), so it coexists safely with
        // on_drag without a hysteresis guard.
        .on_click(cx.listener(move |_dock, _ev: &ClickEvent, window, cx| {
            if let Some(ws) = ws_for_click.upgrade() {
                let target = daruda_store::project::LaneRef {
                    project: project_id,
                    lane: wt_id,
                };
                ws.update(cx, |ws, cx| ws.activate_lane(target, window, cx));
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
        // lane from the same project. Cross-project lanes, plus
        // any project/group payload, are rejected — drawn with the
        // rejected tint so the user sees the row noticed the drag but
        // won't accept it (the workspace op would silently discard
        // these without the tint).
        .drag_over::<DragPayload>(move |style, dragged, _window, _cx| match dragged {
            DragPayload::Lane { target, .. } if target.project == project_id => {
                style.bg(drop_target_bg)
            }
            _ => style.bg(drop_target_rejected_bg),
        })
        .on_drop(
            cx.listener(move |_dock, dragged: &DragPayload, _window, cx| {
                if let DragPayload::Lane { target, .. } = dragged
                    && let Some(ws) = ws_for_drop.upgrade()
                {
                    let from = *target;
                    ws.update(cx, |ws, cx| ws.reorder_lane(from, wt_ref, cx));
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
        .child(claude_status_cell(
            snap.claude_status_per_lane
                .get(&daruda_store::project::LaneRef {
                    project: project_id,
                    lane: wt.id,
                })
                .copied(),
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
            // lane we're about to remove.
            cx.stop_propagation();
            if let Some(ws) = ws_for_close.upgrade() {
                let ws_for_modal = ws_for_close.clone();
                ws.update(cx, |ws, cx| {
                    let target = daruda_store::project::LaneRef {
                        project: project_id,
                        lane: wt_id,
                    };
                    let Some(wt) = ws.lane_for(target) else {
                        return;
                    };
                    if !crate::workspace::Workspace::lane_removable(wt) {
                        return;
                    }
                    let label = gpui::SharedString::from(wt.display_name());
                    let path = gpui::SharedString::from(wt.path.to_string_lossy().into_owned());
                    let plan = match ws.validate_remove_lane(target) {
                        Ok(p) => p,
                        Err(_) => return,
                    };
                    // Pull the branch name so the modal can offer "Also
                    // delete branch X" — None for default / detached
                    // lanes (modal hides the checkbox).
                    let branch = ws.lane_for(target).and_then(|w| match &w.kind {
                        daruda_store::project::LaneKind::Git {
                            branch: Some(b), ..
                        } => Some(b.clone()),
                        _ => None,
                    });
                    crate::workspace::dialog_helpers::open_form_modal(
                        "Remove Lane",
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

/// Non-interactive anchor row shown under a git project's header,
/// above its worktree rows. Displays the project's default branch
/// as the repo's "base", so worktree-only / bare repos still show
/// a main representation. No click / delete / merge affordance.
pub(in crate::workspace) fn repo_base_row(project: &ProjectSnapshot) -> impl IntoElement {
    let label = surface_strings::projects_repo_base_label(
        project.name.as_ref(),
        project.default_branch.as_ref().map(|b| b.as_ref()),
    );
    div()
        .flex()
        .flex_row()
        .items_center()
        .w_full()
        .px(px(theme::LANE_ROW_PAD_X))
        .py(px(theme::LANE_SECTION_PAD_Y))
        .gap(px(theme::LANE_LABEL_GAP))
        .text_size(px(theme::LANE_SUB_FONT_SIZE))
        .text_color(theme::TEXT_SUBTLE)
        .child(
            Icon::new(IconName::GalleryVerticalEnd)
                .with_size(px(theme::LANE_SUB_FONT_SIZE))
                .text_color(theme::TEXT_SUBTLE),
        )
        .child(
            div()
                .flex_1()
                .overflow_hidden()
                .whitespace_nowrap()
                .child(SharedString::from(label)),
        )
}

pub(in crate::workspace) fn non_git_placeholder(cx: &gpui::App) -> impl IntoElement {
    let t = theme::current(cx);
    let hint_color = t.faint_text;
    let init_color = t.muted_text;
    div()
        .flex()
        .flex_col()
        .gap(px(theme::LANE_PLACEHOLDER_LINE_GAP))
        .p(px(theme::LANE_PLACEHOLDER_PAD))
        .text_size(px(theme::LANE_SUB_FONT_SIZE))
        .text_color(hint_color)
        .child(strings::LANE_NON_GIT_HINT)
        .child(
            div()
                .mt(px(theme::LANE_PLACEHOLDER_GIT_INIT_MT))
                .text_color(init_color)
                .child(strings::LANE_GIT_INIT_LABEL),
        )
}
