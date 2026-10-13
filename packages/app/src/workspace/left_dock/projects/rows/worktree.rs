//! Lane row for the lanes view — the worktree row itself and its
//! GitHub-Desktop-style git badge cluster.

use crate::ui::theme;
use gpui::{ClickEvent, Context, IntoElement, SharedString, div, prelude::*, px};

use daruda_store::project::{LaneId, ProjectId};

use crate::lane::Lane;
use crate::ui::{Icon, IconName, Sizable as _};
use crate::workspace::layout::{Dock, LeftDockSnapshot};
use crate::workspace::root_menu::RootContextMenuExt as _;

use super::agent_badges::{agent_badges_row, agent_status_cell};
use super::context_menu::build_context_menu_items;
use super::drag::{DragGhost, DragPayload};

use super::{GitBadgeData, availability_badge, availability_chip};

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

pub(in crate::workspace::left_dock::projects) fn worktree_row(
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

    let status_cell = agent_status_cell(
        snap.agent_status_per_lane
            .get(&daruda_store::project::LaneRef {
                project: project_id,
                lane: wt.id,
            })
            .copied(),
        cx,
    );

    // The status cell shares the label's line so it stays pinned to the
    // first line when sub-rows (sublabel, multi-session badges) grow the row.
    let label_line = div()
        .min_h(px(theme::DOCK_TREE_ROW_LINE_MIN_H))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::LANE_ROW_GAP))
        .child(status_cell)
        .child(
            div()
                .flex_1()
                .overflow_hidden()
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
        );

    // Sub-rows indent past the status cell to stay under the label.
    let sub_rows = div()
        .flex()
        .flex_col()
        .pl(px(theme::STATUS_INDICATOR_CELL_WIDTH + theme::LANE_ROW_GAP))
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

    let body = div()
        .flex_1()
        .flex()
        .flex_col()
        .overflow_hidden()
        .child(label_line)
        .child(sub_rows);

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
