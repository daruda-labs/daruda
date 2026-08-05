//! Lanes view — list of lanes plus a section header. Rendered
//! into the left dock when `left_dock_view == Lanes`.
//!
//! Top-level rows interleave groups and ungrouped projects by their
//! shared `tab_order` pool. A group's member projects render only
//! when the group is expanded.

use crate::ui::theme;
use gpui::{AnyElement, Context, IntoElement, div, prelude::*, px};

use crate::surface::strings as surface_strings;
use crate::workspace::layout::{Dock, GroupSnapshot, LeftDockSnapshot, ProjectSnapshot};

use super::banner::agent_install_banner;
use super::rows::{
    git_badge_for, group_header_row, non_git_placeholder, project_header_row, section_header,
    worktree_row,
};

/// Top-level entry rendered in the lanes tree. Groups carry their
/// member projects so the renderer can expand or hide them together.
enum TopRow<'a> {
    Group(&'a GroupSnapshot, Vec<&'a ProjectSnapshot>),
    UngroupedProject(&'a ProjectSnapshot),
}

impl<'a> TopRow<'a> {
    fn tab_order(&self) -> u32 {
        match self {
            TopRow::Group(g, _) => g.tab_order,
            TopRow::UngroupedProject(p) => p.tab_order,
        }
    }
}

/// Render the Lanes view body.
pub(in crate::workspace) fn render(snap: &LeftDockSnapshot, cx: &mut Context<Dock>) -> AnyElement {
    // `_any_git` is unused inside `section_header` — pass `false` unconditionally.
    let header = section_header(false, snap, cx);

    // Banner + header stay outside the scroll region so the section
    // actions remain reachable no matter where the card list is scrolled.
    let mut body = crate::workspace::left_dock::left_panel_body().gap(px(theme::LANE_CARD_GAP));
    if snap.agent_install_banner_visible {
        body = body.child(agent_install_banner(snap, cx));
    }
    body = body.child(header);

    if snap.projects.is_empty() {
        body = body.child(empty_state(cx));
        return body.into_any_element();
    }

    let any_git = snap
        .projects
        .iter()
        .flat_map(|p| &p.lanes)
        .any(|w| w.is_git());

    let active_project = snap.active.project;
    let active_lane = snap.active.lane;

    // Build the interleaved top-level row list. Member projects keep
    // their own `tab_order` so the order inside an expanded group is
    // stable.
    let mut top_rows: Vec<TopRow<'_>> = Vec::with_capacity(snap.groups.len() + snap.projects.len());
    for g in &snap.groups {
        let mut members: Vec<&ProjectSnapshot> = snap
            .projects
            .iter()
            .filter(|p| p.group_id == Some(g.id))
            .collect();
        members.sort_by_key(|p| p.tab_order);
        top_rows.push(TopRow::Group(g, members));
    }
    for p in &snap.projects {
        if p.group_id.is_none() {
            top_rows.push(TopRow::UngroupedProject(p));
        }
    }
    top_rows.sort_by_key(|r| r.tab_order());

    let scroll_handle = snap.lanes_scroll_handle.clone();
    let mut cards = div()
        .id("lanes-scroll")
        .flex()
        .flex_col()
        .size_full()
        .gap(px(theme::LANE_CARD_GAP))
        .overflow_y_scroll()
        .track_scroll(&scroll_handle);

    for row in &top_rows {
        match row {
            TopRow::Group(group, members) => {
                // Card-level active fill: lit when the focused project
                // is a member of this group. Computed against the full
                // member list (not collapsed-filtered) so a collapsed
                // group whose active member is hidden still reads as
                // selected.
                let card_is_active = members.iter().any(|p| p.id == active_project);
                let header = group_header_row(group, snap, cx).into_any_element();
                let members_block = {
                    let mut inner = div().flex().flex_col().w_full();
                    if !group.is_collapsed {
                        for project in members {
                            inner = inner.child(grouped_project_block(
                                project,
                                active_project,
                                active_lane,
                                snap,
                                cx,
                            ));
                        }
                    }
                    inner.into_any_element()
                };
                cards = cards.child(super::card::group_card(
                    header,
                    members_block,
                    card_is_active,
                    cx,
                ));
            }
            TopRow::UngroupedProject(project) => {
                let inner = ungrouped_project_block(project, active_project, active_lane, snap, cx)
                    .into_any_element();
                cards = cards.child(super::card::ungrouped_shell(inner, cx));
            }
        }
    }

    // Non-git project hint — surfaces the `git init` path.
    if !any_git {
        cards = cards.child(non_git_placeholder(cx));
    }

    // Thumb overlay needs a `.relative()` parent; the cards' own
    // `LANE_CARD_MARGIN_X` gutter already clears the thumb (see
    // `lanes_scrollbar`).
    body = body.child(
        div()
            .flex_1()
            .relative()
            .overflow_hidden()
            .child(cards)
            .children(lanes_scrollbar(&scroll_handle, cx)),
    );

    body.into_any_element()
}

/// Scrollbar thumb for the card list. Display-only (no `on_drag`), matching
/// the Git Changes / Files dock views.
///
/// Sits in the gutter the cards leave free: the thumb spans `[2, 6]` px from
/// the right edge (`SCROLLBAR_MARGIN_R` + `SCROLLBAR_W`) while a card's
/// surface stops at `LANE_CARD_MARGIN_X` (8 px), so it never overlaps the
/// card — no extra padding needed.
fn lanes_scrollbar(
    handle: &gpui::ScrollHandle,
    cx: &gpui::App,
) -> Option<crate::ui::scrollbar::Thumb> {
    let viewport_h = handle.bounds().size.height;
    let max_offset = handle.max_offset().y;
    let t = theme::current(cx);
    crate::ui::scrollbar::vertical_thumb(
        "lanes-scrollbar-thumb",
        viewport_h,
        viewport_h + max_offset,
        handle.offset().y,
        px(0.),
        t.scrollbar_thumb,
        t.dock_scrollbar_thumb_hover,
    )
}

/// Project header + lane rows rendered flush against the dock edge.
fn ungrouped_project_block(
    project: &ProjectSnapshot,
    active_project: daruda_store::project::ProjectId,
    active_lane: daruda_store::project::LaneId,
    snap: &LeftDockSnapshot,
    cx: &mut Context<Dock>,
) -> impl IntoElement + use<> {
    // Same vertical gap as between adjacent lane rows — keeps the
    // project header from sitting flush against its first lane, so
    // header / list reads as the same rhythm the lane list uses
    // internally.
    let mut block = div()
        .flex()
        .flex_col()
        .w_full()
        .gap(px(theme::LANE_LIST_GAP_Y));
    let is_active_project = project.id == active_project;
    let project_is_git = project
        .lanes
        .iter()
        .any(|w| matches!(&w.kind, daruda_store::project::LaneKind::Git { .. }));
    block = block.child(project_header_row(
        super::rows::ProjectHeaderArgs {
            project_id: project.id,
            name: project.name.clone(),
            is_ungrouped: project.group_id.is_none(),
            is_active: is_active_project,
            is_git: project_is_git,
            is_collapsed: project.is_collapsed,
            last_active_lane_id: project.last_active_lane_id,
            availability: project.availability,
            default_branch: project.default_branch.clone(),
        },
        snap,
        cx,
    ));
    if !project.is_collapsed {
        let mut list = div()
            .flex()
            .flex_col()
            .w_full()
            .pl(px(theme::LANE_INDENT_STEP))
            .gap(px(theme::LANE_LIST_GAP_Y));
        for wt in &project.lanes {
            let is_active = project.id == active_project && wt.id == active_lane;
            let git_badge = git_badge_for(snap, project.id, wt.id);
            list = list.child(worktree_row(wt, project.id, is_active, git_badge, snap, cx));
        }
        block = block.child(list);
    }
    block
}

/// Member project block inside an expanded group card. Delegates to
/// `ungrouped_project_block`, which applies the stepped lane indent
/// internally; the group card wraps both the header and lane rows.
fn grouped_project_block(
    project: &ProjectSnapshot,
    active_project: daruda_store::project::ProjectId,
    active_lane: daruda_store::project::LaneId,
    snap: &LeftDockSnapshot,
    cx: &mut Context<Dock>,
) -> impl IntoElement + use<> {
    div()
        .flex()
        .flex_col()
        .w_full()
        .child(ungrouped_project_block(
            project,
            active_project,
            active_lane,
            snap,
            cx,
        ))
}

fn empty_state(cx: &mut Context<Dock>) -> impl IntoElement {
    let cta = crate::ui::button_primary(
        "projects-empty-add",
        surface_strings::projects_empty_state_cta(),
    )
    .on_click(cx.listener(|_dock, _: &gpui::ClickEvent, _window, app_cx| {
        // prompt_and_open_folder_with_policy takes only &mut App — _window unused.
        let config = crate::settings_store::SettingsStore::global(app_cx).user_arc();
        crate::windows::prompt_and_open_folder_with_policy(config, app_cx);
    }));

    div()
        .flex_1()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(theme::MAIN_EMPTY_STATE_GAP))
        .text_size(px(theme::DOCK_PLACEHOLDER_FONT_SIZE))
        .text_color(theme::current(cx).text_subtle)
        .child(crate::ui::placeholder_text(
            surface_strings::projects_empty_state(),
        ))
        .child(cta)
}
