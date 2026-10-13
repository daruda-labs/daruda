//! Header rows for the lanes view — group accordion header, project
//! header, and the section header with its add menu.

use crate::ui::theme;
use gpui::{ClickEvent, Context, IntoElement, Rgba, SharedString, div, prelude::*, px};

use daruda_store::project::{LaneId, ProjectId};

use crate::lane::availability::LaneAvailability;
use crate::surface::strings as surface_strings;
use crate::ui::{DropdownMenu as _, PopupMenuItem, SectionHeader, menu_builder};
use crate::workspace::NewGroup;
use crate::workspace::layout::{Dock, GroupSnapshot, LeftDockSnapshot};
use crate::workspace::root_menu::RootContextMenuExt as _;

use super::drag::{DragGhost, DragPayload};
use crate::workspace::dnd_ops::TopRow;

use super::{availability_badge, availability_chip, branch_chip};

/// Single-row group header that opens / closes the accordion for the
/// projects nested under `group`. Visual leading caret flips on
/// `is_collapsed`; an optional color dot renders when `group.color`
/// parses as a hex RGBA. Clicking the row toggles the group's collapse
/// flag on the workspace.
pub(in crate::workspace::left_dock::projects) fn group_header_row(
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
pub(in crate::workspace::left_dock::projects) struct ProjectHeaderArgs {
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
/// `is_collapsed` flips the leading glyph between an open folder
/// (expanded) and a closed one (collapsed). The glyph carries its own click
/// handler that toggles the flag; the rest of the row stays bound to
/// `activate_lane(last_active)` so a header click still snaps the focus.
pub(in crate::workspace::left_dock::projects) fn project_header_row(
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
        // One glyph shows the fold state and toggles it, so no chevron
        // column sits in front of the name. It stays a quiet outline
        // whatever weight the name carries.
        .child({
            let ws_for_toggle = snap.workspace.clone();
            let folder_icon = if is_collapsed {
                crate::ui::icons::FOLDER
            } else {
                crate::ui::icons::FOLDER_OPEN
            };
            crate::ui::button_icon_glyph(
                ("project-fold", project_id as usize),
                folder_icon,
                theme::LANE_PROJECT_ICON_SIZE,
                cx,
            )
            .on_click(cx.listener(move |_dock, _: &ClickEvent, _window, cx| {
                cx.stop_propagation();
                if let Some(ws) = ws_for_toggle.upgrade() {
                    ws.update(cx, |ws, cx| ws.toggle_project_collapse(project_id, cx));
                }
            }))
        })
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
                            let catalog = ws.mirrors.session_hosts.clone();
                            crate::workspace::dialog_helpers::open_form_modal(
                                surface_strings::create_lane::button_title(),
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

pub(in crate::workspace::left_dock::projects) fn section_header(
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
                PopupMenuItem::new(surface_strings::projects::add_menu_project()).on_click(
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
                PopupMenuItem::new(surface_strings::projects::add_menu_group()).on_click(
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

    SectionHeader::new(surface_strings::projects::section_header())
        .padding(theme::LANE_ROW_PAD_X, theme::LANE_SECTION_PAD_Y)
        .actions(add_button)
}
