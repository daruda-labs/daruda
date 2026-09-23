//! Visible row primitives for the lanes view:
//! `section_header`, `worktree_row`, the small `LaneId → GitBadge`
//! derivation, and the non-git placeholder hint.

use crate::ui::theme;
use gpui::{ClickEvent, Context, IntoElement, Rgba, SharedString, div, prelude::*, px};

use daruda_store::project::{LaneId, ProjectId};

use crate::lane::Lane;
use crate::lane::availability::LaneAvailability;
use crate::surface::strings as surface_strings;
use crate::ui::{
    DropdownMenu as _, Icon, IconName, PopupMenuItem, SectionHeader, Sizable as _, menu_builder,
};
use crate::workspace::NewGroup;
use crate::workspace::layout::{Dock, GroupSnapshot, LeftDockSnapshot};
use crate::workspace::root_menu::RootContextMenuExt as _;

use super::agent_badges::{agent_badges_row, agent_status_cell};
use super::context_menu::build_context_menu_items;
use super::drag::{DragGhost, DragPayload};
use crate::workspace::dnd_ops::TopRow;

/// Compact summary of a lane's git status for the row badge — rolled
/// up into a single change count plus ahead/behind divergence so the
/// dock row can render a GitHub-Desktop-style chip
/// (`↑N ↓N [total]`). `None` means "show nothing" (clean tree + no
/// divergence, or no status cached yet).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct GitBadgeData {
    /// Modified + staged + untracked, rolled up into a single count so the
    /// GH-Desktop pill stays narrow.
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
pub(super) fn git_badge_for(
    snap: &LeftDockSnapshot,
    project_id: ProjectId,
    id: LaneId,
) -> Option<GitBadgeData> {
    let target = daruda_store::project::LaneRef {
        project: project_id,
        lane: id,
    };
    // The badge spans both axes: the counts come from the working tree,
    // the arrows from the refs. Either side alone can make a badge.
    let status = snap.git_worktree_cache.get(&target);
    let tracking = snap.git_tracking_cache.get(&target);
    // Unstaged entries are identified by the y column, not x.
    let modified = status.map_or(0, |s| {
        s.staged.len()
            + s.unstaged
                .iter()
                .filter(|e| e.y != ' ' && e.y != '?')
                .count()
    });
    let untracked = status.map_or(0, |s| s.unstaged.iter().filter(|e| e.x == '?').count());
    let total = (modified + untracked) as u32;
    let (ahead, behind) = tracking.map_or((0, 0), |t| (t.ahead, t.behind));
    if total == 0 && ahead == 0 && behind == 0 {
        return None;
    }
    Some(GitBadgeData {
        total,
        ahead,
        behind,
    })
}

/// Icon + short label for an inaccessible lane / project state. Only
/// the non-`Present` variants are meaningful here; `Present` returns
/// `None` so callers render the normal row. The icon doubles as the
/// color cue — both states paint it in `theme::WARNING`.
fn availability_badge(availability: LaneAvailability) -> Option<(IconName, String)> {
    match availability {
        LaneAvailability::Present => None,
        // No `Lock` glyph in the icon set — `TriangleAlert` reads as
        // "something is wrong with this directory" for the missing
        // case; `EyeOff` reads as "present but unreadable" for the
        // permission-denied case.
        LaneAvailability::Missing => Some((
            IconName::TriangleAlert,
            surface_strings::projects_directory_missing(),
        )),
        LaneAvailability::AccessDenied => Some((
            IconName::EyeOff,
            surface_strings::projects_permission_denied(),
        )),
    }
}

/// Inline `⚠ <label>` chip rendered in place of the normal sublabel /
/// git badge for an inaccessible lane or project. Icon carries the
/// warning color; the label is muted.
fn availability_chip(icon: IconName, label: String, label_color: gpui::Hsla) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::LANE_LABEL_GAP))
        .text_size(px(theme::LANE_SUB_FONT_SIZE))
        .text_color(label_color)
        .child(
            Icon::new(icon)
                .with_size(px(theme::LANE_SUB_FONT_SIZE))
                .text_color(theme::WARNING),
        )
        .child(SharedString::from(label))
}

/// Right-aligned chip on the project header showing the default branch name
/// (e.g. `⎇ main`). Uses `GalleryVerticalEnd` as the branch glyph — the
/// closest available icon to the standard branch symbol; no dedicated
/// `GitBranch` variant exists in the current icon set.
fn branch_chip(branch: SharedString, label_color: gpui::Hsla) -> impl IntoElement {
    div()
        .flex()
        .flex_none() // must not stretch: sits right-aligned after the flex_1 name and keeps its intrinsic width
        .flex_row()
        .items_center()
        .gap(px(theme::LANE_LABEL_GAP))
        .px(px(theme::LANE_BRANCH_CHIP_PAD_X))
        .text_size(px(theme::LANE_SUB_FONT_SIZE))
        .text_color(label_color)
        .child(
            Icon::new(IconName::GalleryVerticalEnd)
                .with_size(px(theme::LANE_SUB_FONT_SIZE))
                .text_color(label_color),
        )
        .child(branch)
}

/// Single-row group header that opens / closes the accordion for the
/// projects nested under `group`. Visual leading caret flips on
/// `is_collapsed`; an optional color dot renders when `group.color`
/// parses as a hex RGBA. Clicking the row toggles the group's collapse
/// flag on the workspace.
pub(super) fn group_header_row(
    group: &GroupSnapshot,
    snap: &LeftDockSnapshot,
    cx: &mut Context<Dock>,
) -> impl IntoElement + use<> {
    let t = theme::current(cx);
    let label_color = t.text_body;
    let row_hover_bg = t.lane_row_hover_bg;
    let drop_target_bg = t.lane_drop_target_bg;
    let drop_target_rejected_bg = t.lane_drop_target_rejected_bg;

    // A stored group color remains independent of the disclosure indicator.
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
        crate::ui::icons::CHEVRON_RIGHT
    } else {
        crate::ui::icons::EXPAND_MORE
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
                .text_ellipsis()
                .child(SharedString::from(group.name.to_uppercase())),
        )
        .child(
            // Same hit target as the add action; collapse state lives in the SVG.
            crate::ui::button_icon(("group-chevron", group_id as usize), caret_icon, cx).on_click(
                cx.listener(move |_dock, _: &ClickEvent, _window, cx| {
                    cx.stop_propagation();
                    if let Some(ws) = ws_for_chevron.upgrade() {
                        ws.update(cx, |ws, cx| ws.toggle_group_collapse(group_id, cx));
                    }
                }),
            ),
        );

    super::tree::row()
        .id(("group-header", group_id as usize))
        .group(row_group_key.clone())
        .flex()
        .flex_col()
        .w_full()
        .cursor_pointer()
        .hover(move |d| d.bg(row_hover_bg))
        .on_click(cx.listener(move |_dock, _: &ClickEvent, _window, cx| {
            if let Some(ws) = workspace.upgrade() {
                ws.update(cx, |ws, cx| ws.toggle_group_collapse(group_id, cx));
            }
        }))
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
        .root_context_menu(ws_for_menu.clone(), move |menu, _window, _cx| {
            let items = super::group_menu::build_group_menu_items(
                group_id,
                group_name_for_menu.clone(),
                group_is_collapsed,
                ws_for_menu.clone(),
            );
            items.into_iter().fold(menu, |m, item| m.item(item))
        })
        .child(label_row)
}

/// Per-row state passed into [`project_header_row`].
///
/// Keeps the seven row-shape inputs (project id, name, group/active/git
/// flags, collapsed flag, and the snap-target lane) grouped at the
/// call site so the header signature stays readable as additional
/// per-project flags accumulate.
pub(super) struct ProjectHeaderArgs {
    pub project_id: ProjectId,
    pub name: SharedString,
    pub is_ungrouped: bool,
    pub is_active: bool,
    pub is_git: bool,
    pub is_collapsed: bool,
    pub last_active_lane_id: LaneId,
    /// Read-availability of the project root. Non-`Present` greys the
    /// header label and appends a state chip; the row stays clickable.
    pub availability: LaneAvailability,
    /// Detected default branch (e.g. `"main"`). When `Some`, a small
    /// branch chip is rendered right-aligned on the header. `None` for
    /// non-git projects or before detection resolves.
    pub default_branch: Option<SharedString>,
}

/// Single-row project header above the lanes list for one
/// project. Drag source for `DragPayload::Project` and drop target for
/// project / group payloads. The `is_ungrouped` flag gates whether a
/// dragged group can land here — groups only sit at the top level, so
/// dropping a group on a project nested inside another group must
/// silently no-op.
///
/// `is_git` enables the trailing `[+]` button — the project row's
/// per-project "create lane" affordance, so the row carries every
/// project-scoped action.
///
/// `is_collapsed` flips the chevron between `ChevronDown` (expanded)
/// and `ChevronRight` (collapsed). The chevron carries its own click
/// handler that toggles the flag; the rest of the row stays bound to
/// `activate_lane(last_active)` so a header click still snaps the focus.
pub(super) fn project_header_row(
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
        availability,
        default_branch,
    } = args;
    let t = theme::current(cx);
    // Inaccessible project roots always render muted, even when active —
    // the header reads as "this project can't be opened right now".
    let avail_badge = availability_badge(availability);
    let is_unavailable = avail_badge.is_some();
    let label_color = if is_active && !is_unavailable {
        t.text_primary
    } else {
        t.text_muted
    };
    let chip_color = t.text_subtle;
    let row_hover_bg = t.lane_row_hover_bg;
    let drop_target_bg = t.lane_drop_target_bg;
    let drop_target_rejected_bg = t.lane_drop_target_rejected_bg;

    let workspace = snap.workspace.clone();
    let ws_for_click = workspace.clone();
    let ws_for_drop = workspace.clone();
    let ws_for_menu = workspace.clone();
    let drag_payload = DragPayload::Project {
        id: project_id,
        label: name.clone(),
    };

    let row_group = SharedString::from(format!("project-row-{project_id}"));
    super::tree::row()
        .id(("project-header", project_id as usize))
        .group(row_group.clone())
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::LANE_LABEL_GAP))
        .w_full()
        .text_size(px(theme::LANE_LABEL_FONT_SIZE))
        .text_color(label_color)
        .cursor_pointer()
        .hover(move |d| d.bg(row_hover_bg))
        // Header click snaps the workspace focus to this project's
        // last-active lane. No-op when the click lands on
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
                crate::ui::icons::CHEVRON_RIGHT
            } else {
                crate::ui::icons::EXPAND_MORE
            };
            // Keep the chevron and add action on the same chrome grid.
            crate::ui::button_icon(("project-chevron", project_id as usize), chevron_icon, cx)
                .on_click(cx.listener(move |_dock, _: &ClickEvent, _window, cx| {
                    cx.stop_propagation();
                    if let Some(ws) = ws_for_chevron.upgrade() {
                        ws.update(cx, |ws, cx| ws.toggle_project_collapse(project_id, cx));
                    }
                }))
        })
        // The folder stays a quiet outline whatever weight the name carries.
        .child(
            crate::ui::icons::icon(crate::ui::icons::FOLDER)
                .with_size(px(theme::LANE_PROJECT_ICON_SIZE))
                .text_color(t.text_muted),
        )
        .child(div().flex_1().text_ellipsis().child(name))
        // Keep branch metadata quiet beside the project name.
        .when_some(default_branch.filter(|_| !is_unavailable), |row, branch| {
            row.child(branch_chip(branch, chip_color))
        })
        .when_some(avail_badge, |row, (icon, state_label)| {
            row.child(availability_chip(icon, state_label, chip_color))
        })
        // The create-lane `[+]` is meaningless for an inaccessible
        // project root, so hide it alongside the state chip.
        .when(is_git && !is_unavailable, |row| {
            let ws_for_add = snap.workspace.clone();
            let row_group_for_btn = row_group.clone();
            row.child(
                crate::ui::button_icon(
                    ("project-add-lane", project_id as usize),
                    crate::ui::icons::ADD,
                    cx,
                )
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
                            let catalog = ws.session_hosts.clone();
                            crate::workspace::dialog_helpers::open_form_modal(
                                surface_strings::create_lane_button_title(),
                                None,
                                move |window, cx| {
                                    super::create_modal::CreateWorktreeModal::new(
                                        workspace_for_modal.clone(),
                                        repo_root,
                                        project_id,
                                        catalog,
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
        .root_context_menu(ws_for_menu.clone(), move |menu, _window, _cx| {
            let items = super::project_menu::build_project_menu_items(
                project_id,
                last_active_lane_id,
                ws_for_menu.clone(),
            );
            items.into_iter().fold(menu, |m, item| m.item(item))
        })
}

pub(super) fn section_header(
    _any_git: bool,
    snap: &LeftDockSnapshot,
    cx: &mut Context<Dock>,
) -> impl IntoElement + use<> {
    // Section-level `[+]` is a toggle: clicking it opens a flat dropdown
    // with "Add Project…" (folder picker routed through
    // `prompt_and_open_folder_with_policy`, policy-aware) and
    // "New Group…". Per-project lane creation lives on each
    // project's `[+ new lane]` row.
    let ws_for_group = snap.workspace.clone();
    let add_button = crate::ui::button_icon("section-add-toggle", crate::ui::icons::ADD, cx)
        .dropdown_menu(menu_builder(move |menu, _window, _cx| {
            let ws_for_group = ws_for_group.clone();
            menu.item(
                PopupMenuItem::new(surface_strings::section_add_menu_project()).on_click(
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
            )
            .item(
                PopupMenuItem::new(surface_strings::section_add_menu_group()).on_click(
                    move |_, window, app_cx| {
                        if let Some(ws) = ws_for_group.upgrade() {
                            ws.update(app_cx, |ws, cx| {
                                ws.on_new_group(&NewGroup, window, cx);
                            });
                        }
                    },
                ),
            )
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

pub(super) fn worktree_row(
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
    let label_active = t.text_primary;
    let label_inactive = t.text_muted;
    let badge_pill_bg = t.overlay_prominent;
    let badge_pill_text = t.text_primary;
    let badge_arrow_text = t.text_muted;
    let sublabel_color = t.text_subtle;
    let row_hover_bg = t.lane_row_hover_bg;
    let row_active_bg = t.lane_card_active_bg;
    let drop_target_bg = t.lane_drop_target_bg;
    let drop_target_rejected_bg = t.lane_drop_target_rejected_bg;

    let label = wt.display_name();
    // Paths stay available on hover without making every lane a two-line row.
    let path_tooltip = format!("{label}\n{}", wt.path.display());
    let sublabel = wt.description.as_deref().filter(|s| !s.trim().is_empty());

    // Inaccessible lanes (Missing / AccessDenied) render muted: the
    // label greys out, the unread dot + git badge are suppressed (a
    // dead root has no meaningful git state), and the path sublabel is
    // replaced by the state chip. The row stays clickable so the user
    // can select it and Remove.
    let avail_badge = availability_badge(wt.availability);
    let is_unavailable = avail_badge.is_some();

    // Unread marker sits to the right of the label.
    let unread_dot = div()
        .flex_none()
        .w(px(theme::LANE_UNREAD_DOT_SIZE))
        .h(px(theme::LANE_UNREAD_DOT_SIZE))
        .rounded(px(theme::LANE_UNREAD_DOT_RADIUS))
        .bg(unread_dot_color);

    // Body — label row + sublabel row + optional agent multi-session sub-row.
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
                .text_color(if is_active && !is_unavailable {
                    label_active
                } else {
                    label_inactive
                })
                .child(div().flex_1().text_ellipsis().child(label.clone()))
                .when(wt.is_unread && !is_unavailable, |d| d.child(unread_dot))
                .when_some(git_badge.filter(|_| !is_unavailable), |d, badge| {
                    d.child(git_badge_view(
                        badge,
                        badge_arrow_text,
                        badge_pill_bg,
                        badge_pill_text,
                    ))
                }),
        )
        .when_some(avail_badge, |d, (icon, state_label)| {
            d.child(availability_chip(icon, state_label, sublabel_color))
        })
        .when_some(sublabel.filter(|_| !is_unavailable), |d, sublabel| {
            d.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(theme::LANE_SUBLABEL_GAP))
                    .text_size(px(theme::LANE_SUB_FONT_SIZE))
                    .text_color(sublabel_color)
                    .text_ellipsis()
                    .child(sublabel.to_owned()),
            )
        })
        .when_some(
            snap.agent_per_session_per_lane
                .get(&daruda_store::project::LaneRef {
                    project: project_id,
                    lane: wt.id,
                }),
            |d, sessions| {
                d.child(agent_badges_row(
                    sessions,
                    snap.agent_active_session_id.as_deref(),
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
    let wt_availability = wt.availability;

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
        .git_worktree_cache
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
    // Lane IDs restart at 0 per project, so `("lane-row", wt_id)` collides
    // across projects and GPUI routes every first-lane click to a single row.
    // Encode both project + lane id so each row is uniquely addressable.
    let row_group = SharedString::from(format!("lane-row-{project_id}-{wt_id}"));
    let mut row = super::tree::row()
        .id(SharedString::from(format!("lane-row-{project_id}-{wt_id}")))
        // Expose to test debug_bounds so gpui::test can find this row.
        .debug_selector(|| format!("lane-row-{project_id}-{wt_id}"))
        .pl(px(theme::LANE_ROW_INSET_L))
        .group(row_group.clone())
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::LANE_ROW_GAP))
        .cursor_pointer()
        .tooltip(crate::ui::tooltip::text(path_tooltip))
        // Reserve a same-width transparent left border on inactive rows so
        // the label x-position stays stable when the active border appears.
        .border_l(px(theme::LANE_ACTIVE_BORDER_W))
        .border_color(theme::TRANSPARENT)
        .when(!is_active, move |d| d.hover(move |d| d.bg(row_hover_bg)))
        .when(is_active, move |d| {
            d.rounded_l_none()
                .bg(row_active_bg)
                .border_color(theme::PRIMARY)
        })
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
        .root_context_menu(ws_for_rclick.clone(), move |menu, _window, _cx| {
            let path_str = wt_path.to_str().map(|s| s.to_string()).unwrap_or_default();
            let items = build_context_menu_items(super::context_menu::CtxMenuArgs {
                project_id,
                wt_id,
                path_str,
                current_description: wt_description_current.clone(),
                current_name: wt_name_current.clone(),
                workspace: ws_for_rclick.clone(),
                is_git: wt_is_git,
                is_detached: wt_is_detached,
                is_dirty: wt_is_dirty,
                source_branch: wt_source_branch.clone(),
                base_ref: wt_base_ref.clone(),
                source_path: wt_path.clone(),
                source_repo_root: wt_source_repo_root.clone(),
                availability: wt_availability,
                removable,
            });
            items.into_iter().fold(menu, |m, item| m.item(item))
        })
        .child(agent_status_cell(
            snap.agent_status_per_lane
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
        let close = crate::ui::button_icon_danger(
            SharedString::from(format!("wt-remove-{project_id}-{wt_id}")),
            crate::ui::icons::DELETE,
            cx,
        )
        .invisible()
        .group_hover(row_group_for_close, |this| this.visible())
        .on_click(cx.listener(move |_dock, _: &ClickEvent, window, cx| {
            // Stop the row's activate handler from firing —
            // clicking × should never also switch to the
            // lane we're about to remove.
            cx.stop_propagation();
            if let Some(ws) = ws_for_close.upgrade() {
                let target = daruda_store::project::LaneRef {
                    project: project_id,
                    lane: wt_id,
                };
                ws.update(cx, |ws, cx| ws.open_remove_lane_modal(target, window, cx));
            }
        }));
        row = row.child(close);
    }

    row
}

pub(super) fn non_git_placeholder(cx: &gpui::App) -> impl IntoElement {
    let t = theme::current(cx);
    let hint_color = t.text_subtle;
    let init_color = t.text_muted;
    div()
        .flex()
        .flex_col()
        .gap(px(theme::LANE_PLACEHOLDER_LINE_GAP))
        .p(px(theme::LANE_PLACEHOLDER_PAD))
        .text_size(px(theme::LANE_SUB_FONT_SIZE))
        .text_color(hint_color)
        .child(surface_strings::lane_non_git_hint())
        .child(
            div()
                .mt(px(theme::LANE_PLACEHOLDER_GIT_INIT_MT))
                .text_color(init_color)
                .child(surface_strings::lane_git_init_label()),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn availability_badge_present_is_none() {
        assert!(availability_badge(LaneAvailability::Present).is_none());
    }

    #[test]
    fn availability_badge_missing_uses_triangle_alert() {
        let (icon, label) =
            availability_badge(LaneAvailability::Missing).expect("missing yields a badge");
        assert!(matches!(icon, IconName::TriangleAlert));
        assert_eq!(label, surface_strings::projects_directory_missing());
    }

    #[test]
    fn availability_badge_denied_uses_eye_off() {
        let (icon, label) =
            availability_badge(LaneAvailability::AccessDenied).expect("denied yields a badge");
        assert!(matches!(icon, IconName::EyeOff));
        assert_eq!(label, surface_strings::projects_permission_denied());
    }
}
