//! Worktrees view — list of worktrees plus a section header. Rendered
//! into the left dock when `left_dock_view == Worktrees`.
//!
//! Top-level rows interleave groups and ungrouped projects by their
//! shared `tab_order` pool. A group's member projects render only
//! when the group is expanded.

use crate::ui::theme;
use gpui::{AnyElement, Context, IntoElement, div, prelude::*, px};

use crate::surface::strings as surface_strings;
use crate::workspace::layout::{Dock, GroupSnapshot, LeftDockSnapshot, ProjectSnapshot};

use super::banner::claude_install_banner;
use super::rows::{
    git_badge_for, group_header_row, non_git_placeholder, project_header_row, section_header,
    worktree_row,
};

/// Top-level entry rendered in the worktrees tree. Groups carry their
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

/// Render the Worktrees view body.
pub(in crate::workspace) fn render(snap: &LeftDockSnapshot, cx: &mut Context<Dock>) -> AnyElement {
    if snap.projects.is_empty() {
        return empty_state(cx).into_any_element();
    }

    let any_git = snap
        .projects
        .iter()
        .flat_map(|p| &p.worktrees)
        .any(|w| w.is_git());
    let active_tab_count = snap.active_tab_count;
    let active_project = snap.active.project;
    let active_worktree = snap.active.worktree;

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

    let header = section_header(any_git, snap, cx);

    let mut body = div()
        .flex()
        .flex_col()
        .w_full()
        .gap(px(theme::WORKTREE_CARD_GAP))
        .overflow_hidden();
    if snap.claude_install_banner_visible {
        body = body.child(claude_install_banner(snap, cx));
    }
    body = body.child(header);

    for row in &top_rows {
        match row {
            TopRow::Group(group, members) => {
                let header = group_header_row(group, snap, cx).into_any_element();
                let members_block = {
                    let mut inner = div().flex().flex_col().w_full();
                    if !group.is_collapsed {
                        for project in members {
                            inner = inner.child(grouped_project_block(
                                project,
                                active_project,
                                active_worktree,
                                active_tab_count,
                                snap,
                                cx,
                            ));
                        }
                    }
                    inner.into_any_element()
                };
                body = body.child(super::card::group_card(header, members_block, cx));
            }
            TopRow::UngroupedProject(project) => {
                let inner = ungrouped_project_block(
                    project,
                    active_project,
                    active_worktree,
                    active_tab_count,
                    snap,
                    cx,
                )
                .into_any_element();
                body = body.child(super::card::ungrouped_shell(inner, cx));
            }
        }
    }

    // Non-git project hint — surfaces the `git init` path.
    if !any_git {
        body = body.child(non_git_placeholder(cx));
    }

    body.into_any_element()
}

/// Project header + worktree rows rendered flush against the dock edge.
fn ungrouped_project_block(
    project: &ProjectSnapshot,
    active_project: daruda_store::project::ProjectId,
    active_worktree: daruda_store::project::WorktreeId,
    active_tab_count: usize,
    snap: &LeftDockSnapshot,
    cx: &mut Context<Dock>,
) -> impl IntoElement + use<> {
    let mut block = div().flex().flex_col().w_full();
    let is_active_project = project.id == active_project;
    let project_is_git = project
        .worktrees
        .iter()
        .any(|w| matches!(&w.kind, daruda_store::project::WorktreeKind::Git { .. }));
    block = block.child(project_header_row(
        super::rows::ProjectHeaderArgs {
            project_id: project.id,
            name: project.name.clone(),
            is_ungrouped: project.group_id.is_none(),
            is_active: is_active_project,
            is_git: project_is_git,
            is_collapsed: project.is_collapsed,
            last_active_worktree_id: project.last_active_worktree_id,
        },
        snap,
        cx,
    ));
    if !project.is_collapsed {
        let mut list = div()
            .flex()
            .flex_col()
            .w_full()
            .gap(px(theme::WORKTREE_LIST_GAP_Y));
        for wt in &project.worktrees {
            let is_active = project.id == active_project && wt.id == active_worktree;
            let tab_count = if is_active { active_tab_count } else { 0 };
            let git_badge = git_badge_for(snap, project.id, wt.id);
            list = list.child(worktree_row(
                wt, project.id, is_active, tab_count, git_badge, snap, cx,
            ));
        }
        block = block.child(list);
    }
    block
}

/// Member project block inside an expanded group card. Layout is the
/// same as the ungrouped variant — the wrapping card supplies the
/// padding that used to come from a left indent.
fn grouped_project_block(
    project: &ProjectSnapshot,
    active_project: daruda_store::project::ProjectId,
    active_worktree: daruda_store::project::WorktreeId,
    active_tab_count: usize,
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
            active_worktree,
            active_tab_count,
            snap,
            cx,
        ))
}

fn empty_state(cx: &gpui::App) -> impl IntoElement {
    div()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(theme::DOCK_PLACEHOLDER_FONT_SIZE))
        .text_color(theme::current(cx).dock_placeholder_text)
        .child(surface_strings::PROJECTS_EMPTY_STATE)
}
