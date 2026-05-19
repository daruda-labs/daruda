//! Tests for the left-dock DnD ops (worktree, project, group
//! reordering). Each test sets up a workspace with the minimum shape
//! the op under test needs — extra projects, groups, or worktrees are
//! injected directly instead of going through filesystem-bound paths.

use super::*;
use crate::workspace::dnd_ops::TopRow;
use daruda_store::project::{GroupId, ProjectId, WorktreeId, WorktreeKind, WorktreeRef};

/// Push a synthetic worktree into the project so reorder tests have
/// more than the bootstrapped row to work with. `tab_order` matches the
/// caller-supplied value because the dock sorts by it and we want
/// deterministic before-state.
fn push_worktree(project: &mut crate::project::Project, id: WorktreeId, tab_order: u32) {
    project.worktrees.push(crate::worktree::Worktree {
        id,
        kind: WorktreeKind::Default,
        path: std::path::PathBuf::from(format!("/tmp/dnd_wt_{id}")),
        name: None,
        tab_order,
        is_unread: false,
        last_activity: 0,
        status: daruda_store::project::WorktreeStatus::Idle,
        base_ref: None,
        description: None,
    });
}

fn make_workspace_with_dirs(
    cx: &mut TestAppContext,
    primary: &str,
) -> gpui::WindowHandle<Workspace> {
    let config = daruda_config::Config::default();
    std::fs::create_dir_all(primary).unwrap();
    let project = daruda_store::project::Project::from_path(primary);
    cx.add_window(|window, cx| {
        Workspace::new_with_project(&config, Some(project), fresh_test_data_dir(), window, cx)
    })
}

// ---- reorder_worktree ----

#[gpui::test]
fn reorder_worktree_moves_before_target_and_renumbers(cx: &mut TestAppContext) {
    let wh = make_workspace_with_dirs(cx, "/tmp/daruda_dnd_reorder_wt");
    let ws = wh.root(cx).unwrap();
    let (project_id, ref_a, ref_b) = ws.update(cx, |ws, _| {
        let p = &mut ws.projects[0];
        let pid = p.id;
        // Bootstrapped worktree already sits at id 0 / tab_order 0.
        push_worktree(p, 100, 1);
        push_worktree(p, 200, 2);
        (
            pid,
            WorktreeRef {
                project: pid,
                worktree: 200,
            },
            WorktreeRef {
                project: pid,
                worktree: 100,
            },
        )
    });
    ws.update(cx, |ws, cx| ws.reorder_worktree(ref_a, ref_b, cx));
    ws.read_with(cx, |ws, _| {
        let p = ws.projects.iter().find(|p| p.id == project_id).unwrap();
        // Expect: [0, 200, 100] in tab_order 0..2.
        let order: Vec<(WorktreeId, u32)> =
            p.worktrees.iter().map(|w| (w.id, w.tab_order)).collect();
        assert_eq!(order, vec![(0, 0), (200, 1), (100, 2)]);
    });
}

#[gpui::test]
fn reorder_worktree_rejects_cross_project(cx: &mut TestAppContext) {
    let wh = make_workspace_with_dirs(cx, "/tmp/daruda_dnd_wt_cross_a");
    let ws = wh.root(cx).unwrap();
    std::fs::create_dir_all("/tmp/daruda_dnd_wt_cross_b").unwrap();
    let _ = cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.add_project(
                std::path::PathBuf::from("/tmp/daruda_dnd_wt_cross_b"),
                window,
                cx,
            )
        })
    });
    // Snapshot tab_order before the cross-project move attempt.
    let before: Vec<(ProjectId, WorktreeId, u32)> = ws.read_with(cx, |ws, _| {
        ws.projects
            .iter()
            .flat_map(|p| {
                p.worktrees
                    .iter()
                    .map(move |w| (p.id, w.id, w.tab_order))
                    .collect::<Vec<_>>()
            })
            .collect()
    });
    let (from_project, to_project) =
        ws.read_with(cx, |ws, _| (ws.projects[0].id, ws.projects[1].id));
    let from = WorktreeRef {
        project: from_project,
        worktree: 0,
    };
    let to = WorktreeRef {
        project: to_project,
        worktree: 0,
    };
    ws.update(cx, |ws, cx| ws.reorder_worktree(from, to, cx));
    // Order is unchanged — cross-project move is rejected silently.
    ws.read_with(cx, |ws, _| {
        let after: Vec<(ProjectId, WorktreeId, u32)> = ws
            .projects
            .iter()
            .flat_map(|p| {
                p.worktrees
                    .iter()
                    .map(move |w| (p.id, w.id, w.tab_order))
                    .collect::<Vec<_>>()
            })
            .collect();
        assert_eq!(before, after);
    });
}

// ---- reorder_project_before ----

#[gpui::test]
fn reorder_project_before_in_top_level_swaps_positions(cx: &mut TestAppContext) {
    let wh = make_workspace_with_dirs(cx, "/tmp/daruda_dnd_proj_top_a");
    let ws = wh.root(cx).unwrap();
    std::fs::create_dir_all("/tmp/daruda_dnd_proj_top_b").unwrap();
    let _ = cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.add_project(
                std::path::PathBuf::from("/tmp/daruda_dnd_proj_top_b"),
                window,
                cx,
            )
        })
    });
    let (pa, pb) = ws.read_with(cx, |ws, _| (ws.projects[0].id, ws.projects[1].id));
    // Before: [a=0, b=1]. Move b before a → [b=0, a=1].
    ws.update(cx, |ws, cx| ws.reorder_project_before(pb, pa, cx));
    ws.read_with(cx, |ws, _| {
        let pa_order = ws.projects.iter().find(|p| p.id == pa).unwrap().tab_order;
        let pb_order = ws.projects.iter().find(|p| p.id == pb).unwrap().tab_order;
        assert!(pb_order < pa_order, "b must sit before a after the move");
        let mut orders: Vec<u32> = vec![pa_order, pb_order];
        orders.sort();
        assert_eq!(orders, vec![0, 1]);
    });
}

#[gpui::test]
fn reorder_project_before_inherits_target_group(cx: &mut TestAppContext) {
    let wh = make_workspace_with_dirs(cx, "/tmp/daruda_dnd_proj_inherit_a");
    let ws = wh.root(cx).unwrap();
    std::fs::create_dir_all("/tmp/daruda_dnd_proj_inherit_b").unwrap();
    let _ = cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.add_project(
                std::path::PathBuf::from("/tmp/daruda_dnd_proj_inherit_b"),
                window,
                cx,
            )
        })
    });
    let (pa, pb, gid) = ws.update(cx, |ws, cx| {
        let gid: GroupId = ws.add_group("g".to_string(), None, cx);
        let pa = ws.projects[0].id;
        let pb = ws.projects[1].id;
        ws.move_project_to_group(pa, Some(gid), cx);
        (pa, pb, gid)
    });
    // Before the DnD op: pa is in `gid`, pb is ungrouped. Drop pb on
    // pa → pb re-parents into `gid` at pa's position.
    ws.update(cx, |ws, cx| ws.reorder_project_before(pb, pa, cx));
    ws.read_with(cx, |ws, _| {
        let pb_after = ws.projects.iter().find(|p| p.id == pb).unwrap();
        assert_eq!(pb_after.group_id, Some(gid));
        // Both projects in the group must occupy 0..1 contiguously.
        let mut orders: Vec<u32> = ws
            .projects
            .iter()
            .filter(|p| p.group_id == Some(gid))
            .map(|p| p.tab_order)
            .collect();
        orders.sort();
        assert_eq!(orders, vec![0, 1]);
    });
}

#[gpui::test]
fn reorder_project_before_inside_same_group_swaps_order(cx: &mut TestAppContext) {
    let wh = make_workspace_with_dirs(cx, "/tmp/daruda_dnd_proj_same_grp_a");
    let ws = wh.root(cx).unwrap();
    std::fs::create_dir_all("/tmp/daruda_dnd_proj_same_grp_b").unwrap();
    let _ = cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.add_project(
                std::path::PathBuf::from("/tmp/daruda_dnd_proj_same_grp_b"),
                window,
                cx,
            )
        })
    });
    let (pa, pb, gid) = ws.update(cx, |ws, cx| {
        let gid = ws.add_group("g".to_string(), None, cx);
        let pa = ws.projects[0].id;
        let pb = ws.projects[1].id;
        ws.move_project_to_group(pa, Some(gid), cx);
        ws.move_project_to_group(pb, Some(gid), cx);
        (pa, pb, gid)
    });
    // After moves both projects share the same per-group pool. pa was
    // pushed in first (tab_order < pb). Drop pb before pa to verify the
    // intra-group code path actually swaps them and persists 0..N.
    ws.update(cx, |ws, cx| ws.reorder_project_before(pb, pa, cx));
    ws.read_with(cx, |ws, _| {
        let pa_t = ws.projects.iter().find(|p| p.id == pa).unwrap().tab_order;
        let pb_t = ws.projects.iter().find(|p| p.id == pb).unwrap().tab_order;
        assert!(pb_t < pa_t, "pb must land before pa within the group");
        let mut orders: Vec<u32> = ws
            .projects
            .iter()
            .filter(|p| p.group_id == Some(gid))
            .map(|p| p.tab_order)
            .collect();
        orders.sort();
        assert_eq!(orders, vec![0, 1]);
    });
}

#[gpui::test]
fn reorder_project_before_adjacent_target_is_identity_noop(cx: &mut TestAppContext) {
    let wh = make_workspace_with_dirs(cx, "/tmp/daruda_dnd_proj_adj_a");
    let ws = wh.root(cx).unwrap();
    std::fs::create_dir_all("/tmp/daruda_dnd_proj_adj_b").unwrap();
    let _ = cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.add_project(
                std::path::PathBuf::from("/tmp/daruda_dnd_proj_adj_b"),
                window,
                cx,
            )
        })
    });
    // pa already sits immediately before pb in the top-level pool.
    // Dropping pa before pb is a visual no-op; tab_orders and group
    // membership must be left untouched.
    let (pa, pb, before_state) = ws.read_with(cx, |ws, _| {
        let pa = ws.projects[0].id;
        let pb = ws.projects[1].id;
        let state: Vec<(ProjectId, u32, Option<GroupId>)> = ws
            .projects
            .iter()
            .map(|p| (p.id, p.tab_order, p.group_id))
            .collect();
        (pa, pb, state)
    });
    ws.update(cx, |ws, cx| ws.reorder_project_before(pa, pb, cx));
    let after_state: Vec<(ProjectId, u32, Option<GroupId>)> = ws.read_with(cx, |ws, _| {
        ws.projects
            .iter()
            .map(|p| (p.id, p.tab_order, p.group_id))
            .collect()
    });
    assert_eq!(before_state, after_state);
}

#[gpui::test]
fn reorder_project_before_unknown_id_is_noop(cx: &mut TestAppContext) {
    let wh = make_workspace_with_dirs(cx, "/tmp/daruda_dnd_proj_unknown");
    let ws = wh.root(cx).unwrap();
    let (pa, before) = ws.read_with(cx, |ws, _| (ws.projects[0].id, ws.projects[0].tab_order));
    // Unknown `to` must leave state untouched — no group_id mutation,
    // no tab_order shift.
    ws.update(cx, |ws, cx| ws.reorder_project_before(pa, 9_999, cx));
    let after = ws.read_with(cx, |ws, _| ws.projects[0].tab_order);
    assert_eq!(before, after);
}

#[gpui::test]
fn reorder_project_before_self_is_noop(cx: &mut TestAppContext) {
    let wh = make_workspace_with_dirs(cx, "/tmp/daruda_dnd_proj_self");
    let ws = wh.root(cx).unwrap();
    let pa = ws.read_with(cx, |ws, _| ws.projects[0].id);
    let before = ws.read_with(cx, |ws, _| ws.projects[0].tab_order);
    ws.update(cx, |ws, cx| ws.reorder_project_before(pa, pa, cx));
    let after = ws.read_with(cx, |ws, _| ws.projects[0].tab_order);
    assert_eq!(before, after);
}

// ---- move_project_to_group_end ----

#[gpui::test]
fn move_project_to_group_end_appends_and_renumbers_top_pool(cx: &mut TestAppContext) {
    let wh = make_workspace_with_dirs(cx, "/tmp/daruda_dnd_move_end_a");
    let ws = wh.root(cx).unwrap();
    std::fs::create_dir_all("/tmp/daruda_dnd_move_end_b").unwrap();
    let _ = cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.add_project(
                std::path::PathBuf::from("/tmp/daruda_dnd_move_end_b"),
                window,
                cx,
            )
        })
    });
    let (pa, pb, gid) = ws.update(cx, |ws, cx| {
        let gid = ws.add_group("g".to_string(), None, cx);
        // Seed: group has one existing member so we can verify pb lands after it.
        let pa = ws.projects[0].id;
        ws.move_project_to_group(pa, Some(gid), cx);
        let pb = ws.projects[1].id;
        (pa, pb, gid)
    });
    // pb is ungrouped before the call. After: pb appended at end of
    // group's member list, and the top-level pool renumbers without pb.
    ws.update(cx, |ws, cx| ws.move_project_to_group_end(pb, gid, cx));
    ws.read_with(cx, |ws, _| {
        let pa_t = ws.projects.iter().find(|p| p.id == pa).unwrap().tab_order;
        let pb_p = ws.projects.iter().find(|p| p.id == pb).unwrap();
        assert_eq!(pb_p.group_id, Some(gid));
        assert!(pb_p.tab_order > pa_t, "pb must follow pa in the group");
        // Top-level pool now contains only the group itself; the lone
        // entry must sit at tab_order 0.
        let group_order = ws.groups.iter().find(|g| g.id == gid).unwrap().tab_order;
        assert_eq!(group_order, 0);
    });
}

#[gpui::test]
fn move_project_to_group_end_same_group_is_noop(cx: &mut TestAppContext) {
    let wh = make_workspace_with_dirs(cx, "/tmp/daruda_dnd_move_same");
    let ws = wh.root(cx).unwrap();
    let (pa, gid) = ws.update(cx, |ws, cx| {
        let gid = ws.add_group("g".to_string(), None, cx);
        let pa = ws.projects[0].id;
        ws.move_project_to_group(pa, Some(gid), cx);
        (pa, gid)
    });
    let before = ws.read_with(cx, |ws, _| {
        ws.projects.iter().find(|p| p.id == pa).unwrap().tab_order
    });
    ws.update(cx, |ws, cx| ws.move_project_to_group_end(pa, gid, cx));
    let after = ws.read_with(cx, |ws, _| {
        ws.projects.iter().find(|p| p.id == pa).unwrap().tab_order
    });
    assert_eq!(before, after);
}

// ---- reorder_group_before_top_row ----

#[gpui::test]
fn reorder_group_before_group_target_swaps_at_top_level(cx: &mut TestAppContext) {
    let wh = make_workspace_with_dirs(cx, "/tmp/daruda_dnd_group_g2g");
    let ws = wh.root(cx).unwrap();
    let (ga, gb) = ws.update(cx, |ws, cx| {
        let ga = ws.add_group("a".to_string(), None, cx);
        let gb = ws.add_group("b".to_string(), None, cx);
        (ga, gb)
    });
    let (ga_t_before, gb_t_before) = ws.read_with(cx, |ws, _| {
        (
            ws.groups.iter().find(|g| g.id == ga).unwrap().tab_order,
            ws.groups.iter().find(|g| g.id == gb).unwrap().tab_order,
        )
    });
    assert!(ga_t_before < gb_t_before);
    // Move gb before ga → gb lands at ga's old slot, ga shifts right.
    ws.update(cx, |ws, cx| {
        ws.reorder_group_before_top_row(gb, TopRow::Group(ga), cx)
    });
    ws.read_with(cx, |ws, _| {
        let ga_t = ws.groups.iter().find(|g| g.id == ga).unwrap().tab_order;
        let gb_t = ws.groups.iter().find(|g| g.id == gb).unwrap().tab_order;
        assert!(gb_t < ga_t);
    });
}

#[gpui::test]
fn reorder_group_before_ungrouped_project_target(cx: &mut TestAppContext) {
    let wh = make_workspace_with_dirs(cx, "/tmp/daruda_dnd_group_p");
    let ws = wh.root(cx).unwrap();
    let (gid, pa) = ws.update(cx, |ws, cx| {
        // Group is added AFTER the bootstrapped project, so it sits
        // after the ungrouped project in the shared pool.
        let gid = ws.add_group("g".to_string(), None, cx);
        let pa = ws.projects[0].id;
        (gid, pa)
    });
    let (group_t_before, proj_t_before) = ws.read_with(cx, |ws, _| {
        (
            ws.groups.iter().find(|g| g.id == gid).unwrap().tab_order,
            ws.projects.iter().find(|p| p.id == pa).unwrap().tab_order,
        )
    });
    assert!(group_t_before > proj_t_before);
    // Move group before the ungrouped project → group's tab_order
    // drops below the project's.
    ws.update(cx, |ws, cx| {
        ws.reorder_group_before_top_row(gid, TopRow::Project(pa), cx)
    });
    ws.read_with(cx, |ws, _| {
        let group_t = ws.groups.iter().find(|g| g.id == gid).unwrap().tab_order;
        let proj_t = ws.projects.iter().find(|p| p.id == pa).unwrap().tab_order;
        assert!(group_t < proj_t);
    });
}

#[gpui::test]
fn reorder_group_before_grouped_project_is_noop(cx: &mut TestAppContext) {
    let wh = make_workspace_with_dirs(cx, "/tmp/daruda_dnd_group_grouped");
    let ws = wh.root(cx).unwrap();
    let (gid_anchor, gid_drag, pa) = ws.update(cx, |ws, cx| {
        let anchor = ws.add_group("anchor".to_string(), None, cx);
        let drag = ws.add_group("drag".to_string(), None, cx);
        let pa = ws.projects[0].id;
        ws.move_project_to_group(pa, Some(anchor), cx);
        (anchor, drag, pa)
    });
    let before = ws.read_with(cx, |ws, _| {
        ws.groups
            .iter()
            .find(|g| g.id == gid_drag)
            .unwrap()
            .tab_order
    });
    // Attempt to drop the drag group on the grouped project — must be
    // rejected because groups never sit inside groups.
    ws.update(cx, |ws, cx| {
        ws.reorder_group_before_top_row(gid_drag, TopRow::Project(pa), cx)
    });
    let after = ws.read_with(cx, |ws, _| {
        ws.groups
            .iter()
            .find(|g| g.id == gid_drag)
            .unwrap()
            .tab_order
    });
    assert_eq!(before, after);
    let _ = gid_anchor;
}

#[gpui::test]
fn reorder_group_self_target_is_noop(cx: &mut TestAppContext) {
    let wh = make_workspace_with_dirs(cx, "/tmp/daruda_dnd_group_self");
    let ws = wh.root(cx).unwrap();
    let gid = ws.update(cx, |ws, cx| ws.add_group("g".to_string(), None, cx));
    let before = ws.read_with(cx, |ws, _| {
        ws.groups.iter().find(|g| g.id == gid).unwrap().tab_order
    });
    ws.update(cx, |ws, cx| {
        ws.reorder_group_before_top_row(gid, TopRow::Group(gid), cx)
    });
    let after = ws.read_with(cx, |ws, _| {
        ws.groups.iter().find(|g| g.id == gid).unwrap().tab_order
    });
    assert_eq!(before, after);
}
