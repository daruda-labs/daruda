//! Tests for the left-dock DnD ops (lane, project, group
//! reordering). Each test sets up a workspace with the minimum shape
//! the op under test needs — extra projects, groups, or lanes are
//! injected directly instead of going through filesystem-bound paths.

use super::*;
use crate::workspace::dnd_ops::TopRow;
use daruda_store::project::{GroupId, LaneId, LaneKind, LaneRef, ProjectId};

/// Push a synthetic lane into the project so reorder tests have
/// more than the bootstrapped row to work with. `tab_order` matches the
/// caller-supplied value because the dock sorts by it and we want
/// deterministic before-state.
fn push_worktree(project: &mut crate::project::Project, id: LaneId, tab_order: u32) {
    project.lanes.push(crate::lane::Lane {
        id,
        kind: LaneKind::Default,
        path: std::path::PathBuf::from(format!("/tmp/dnd_wt_{id}")),
        name: None,
        tab_order,
        is_unread: false,
        last_activity: 0,
        status: daruda_store::project::LaneStatus::Idle,
        is_main: false,
        base_ref: None,
        description: None,
        remote_cwd: None,
        session_host: None,
        availability: crate::lane::availability::LaneAvailability::Present,
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
        Workspace::new_with_project_for_test(
            &config,
            Some(project),
            fresh_test_data_dir(),
            window,
            cx,
        )
    })
}

/// Add a second project directly into the workspace without going
/// through `add_project` (which triggers `activate_lane`, file-tree
/// setup, and terminal pane spawn — none of which the DnD reorder ops
/// care about). The returned id matches what production would mint.
fn push_project_in_memory(ws: &mut Workspace, root: &str) -> ProjectId {
    let id = ws.next_project_id;
    ws.next_project_id = ws.next_project_id.checked_add(1).unwrap();
    let mut p = crate::project::Project::bootstrap(id, std::path::PathBuf::from(root));
    p.tab_order = ws.projects.len() as u32;
    ws.projects.push(p);
    id
}

// ---- reorder_lane ----

#[gpui::test]
fn reorder_lane_moves_within_project_and_rejects_cross_project(cx: &mut TestAppContext) {
    let wh = make_workspace_with_dirs(cx, "/tmp/daruda_dnd_reorder_wt");
    let ws = wh.root(cx).unwrap();
    let (project_id, ref_a, ref_b, other_project_id) = ws.update(cx, |ws, _cx| {
        let other_project_id = push_project_in_memory(ws, "/tmp/daruda_dnd_wt_cross_b");
        let p = &mut ws.projects[0];
        let pid = p.id;
        // Bootstrapped lane already sits at id 0 / tab_order 0.
        push_worktree(p, 100, 1);
        push_worktree(p, 200, 2);
        (
            pid,
            LaneRef {
                project: pid,
                lane: 200,
            },
            LaneRef {
                project: pid,
                lane: 100,
            },
            other_project_id,
        )
    });
    ws.update(cx, |ws, cx| ws.reorder_lane(ref_a, ref_b, cx));
    ws.read_with(cx, |ws, _| {
        let p = ws.projects.iter().find(|p| p.id == project_id).unwrap();
        // Expect: [0, 200, 100] in tab_order 0..2.
        let order: Vec<(LaneId, u32)> = p.lanes.iter().map(|w| (w.id, w.tab_order)).collect();
        assert_eq!(order, vec![(0, 0), (200, 1), (100, 2)]);
    });

    let before: Vec<(ProjectId, LaneId, u32)> = ws.read_with(cx, |ws, _| {
        ws.projects
            .iter()
            .flat_map(|p| {
                p.lanes
                    .iter()
                    .map(move |w| (p.id, w.id, w.tab_order))
                    .collect::<Vec<_>>()
            })
            .collect()
    });
    let from = LaneRef {
        project: project_id,
        lane: 0,
    };
    let to = LaneRef {
        project: other_project_id,
        lane: 0,
    };
    ws.update(cx, |ws, cx| ws.reorder_lane(from, to, cx));
    // Order is unchanged — cross-project move is rejected silently.
    ws.read_with(cx, |ws, _| {
        let after: Vec<(ProjectId, LaneId, u32)> = ws
            .projects
            .iter()
            .flat_map(|p| {
                p.lanes
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
fn reorder_project_before_top_level_handles_upward_and_downward_adjacent_moves(
    cx: &mut TestAppContext,
) {
    let wh = make_workspace_with_dirs(cx, "/tmp/daruda_dnd_proj_top_a");
    let ws = wh.root(cx).unwrap();
    ws.update(cx, |ws, _cx| {
        push_project_in_memory(ws, "/tmp/daruda_dnd_proj_top_b");
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

    // Move b downward onto a's slot. Downward drops land after the target.
    ws.update(cx, |ws, cx| ws.reorder_project_before(pb, pa, cx));
    ws.read_with(cx, |ws, _| {
        let pa_order = ws.projects.iter().find(|p| p.id == pa).unwrap().tab_order;
        let pb_order = ws.projects.iter().find(|p| p.id == pb).unwrap().tab_order;
        assert!(pa_order < pb_order, "b must sit after a after the move");
        let mut orders: Vec<u32> = vec![pa_order, pb_order];
        orders.sort();
        assert_eq!(orders, vec![0, 1]);
    });

    let before = ws.read_with(cx, |ws, _| {
        ws.projects
            .iter()
            .map(|p| (p.id, p.group_id, p.tab_order))
            .collect::<Vec<_>>()
    });
    ws.update(cx, |ws, cx| ws.reorder_project_before(pa, 9_999, cx));
    let after = ws.read_with(cx, |ws, _| {
        ws.projects
            .iter()
            .map(|p| (p.id, p.group_id, p.tab_order))
            .collect::<Vec<_>>()
    });
    assert_eq!(before, after, "unknown targets are no-ops");

    let before = ws.read_with(cx, |ws, _| {
        ws.projects
            .iter()
            .map(|p| (p.id, p.group_id, p.tab_order))
            .collect::<Vec<_>>()
    });
    ws.update(cx, |ws, cx| ws.reorder_project_before(pa, pa, cx));
    let after = ws.read_with(cx, |ws, _| {
        ws.projects
            .iter()
            .map(|p| (p.id, p.group_id, p.tab_order))
            .collect::<Vec<_>>()
    });
    assert_eq!(before, after, "self-target drops are no-ops");
}

#[gpui::test]
fn reorder_project_before_inherits_target_group_and_swaps_inside_it(cx: &mut TestAppContext) {
    let wh = make_workspace_with_dirs(cx, "/tmp/daruda_dnd_proj_inherit_a");
    let ws = wh.root(cx).unwrap();
    ws.update(cx, |ws, _cx| {
        push_project_in_memory(ws, "/tmp/daruda_dnd_proj_inherit_b");
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
        let pa_t = ws.projects.iter().find(|p| p.id == pa).unwrap().tab_order;
        let pb_after = ws.projects.iter().find(|p| p.id == pb).unwrap();
        assert_eq!(pb_after.group_id, Some(gid));
        assert!(
            pb_after.tab_order < pa_t,
            "pb must land before pa after inheriting the group"
        );
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

    ws.update(cx, |ws, cx| ws.reorder_project_before(pa, pb, cx));
    ws.read_with(cx, |ws, _| {
        let pa_t = ws.projects.iter().find(|p| p.id == pa).unwrap().tab_order;
        let pb_t = ws.projects.iter().find(|p| p.id == pb).unwrap().tab_order;
        assert!(pa_t < pb_t, "pa must land before pb within the group");
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
fn reorder_project_before_downward_across_multiple_lands_after_target(cx: &mut TestAppContext) {
    let wh = make_workspace_with_dirs(cx, "/tmp/daruda_dnd_proj_down_multi_a");
    let ws = wh.root(cx).unwrap();
    ws.update(cx, |ws, _cx| {
        for sub in ["b", "c"] {
            push_project_in_memory(ws, &format!("/tmp/daruda_dnd_proj_down_multi_{sub}"));
        }
    });
    // Before: [pa=0, pb=1, pc=2]. Drag pa onto pc (down past pb).
    // Expected: pa lands AFTER pc → [pb=0, pc=1, pa=2]. Regression guard
    // for the "1 → 3 lands one slot short at 2" downward-drop bug.
    let (pa, pb, pc) = ws.read_with(cx, |ws, _| {
        (ws.projects[0].id, ws.projects[1].id, ws.projects[2].id)
    });
    ws.update(cx, |ws, cx| ws.reorder_project_before(pa, pc, cx));
    ws.read_with(cx, |ws, _| {
        let pa_t = ws.projects.iter().find(|p| p.id == pa).unwrap().tab_order;
        let pb_t = ws.projects.iter().find(|p| p.id == pb).unwrap().tab_order;
        let pc_t = ws.projects.iter().find(|p| p.id == pc).unwrap().tab_order;
        assert_eq!(pb_t, 0);
        assert_eq!(pc_t, 1);
        assert_eq!(pa_t, 2);
    });
}

// ---- move_project_to_group_end ----

#[gpui::test]
fn move_project_to_group_end_appends_and_renumbers_top_pool(cx: &mut TestAppContext) {
    let wh = make_workspace_with_dirs(cx, "/tmp/daruda_dnd_move_end_a");
    let ws = wh.root(cx).unwrap();
    ws.update(cx, |ws, _cx| {
        push_project_in_memory(ws, "/tmp/daruda_dnd_move_end_b");
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
    let pb_order_after_append = ws.read_with(cx, |ws, _| {
        let pa_t = ws.projects.iter().find(|p| p.id == pa).unwrap().tab_order;
        let pb_p = ws.projects.iter().find(|p| p.id == pb).unwrap();
        assert_eq!(pb_p.group_id, Some(gid));
        assert!(pb_p.tab_order > pa_t, "pb must follow pa in the group");
        // Top-level pool now contains only the group itself; the lone
        // entry must sit at tab_order 0.
        let group_order = ws.groups.iter().find(|g| g.id == gid).unwrap().tab_order;
        assert_eq!(group_order, 0);
        pb_p.tab_order
    });

    ws.update(cx, |ws, cx| ws.move_project_to_group_end(pb, gid, cx));
    let after = ws.read_with(cx, |ws, _| {
        ws.projects.iter().find(|p| p.id == pb).unwrap().tab_order
    });
    assert_eq!(pb_order_after_append, after);
}

// ---- reorder_group_before_top_row ----

#[gpui::test]
fn reorder_group_before_top_level_group_or_project_target(cx: &mut TestAppContext) {
    let wh = make_workspace_with_dirs(cx, "/tmp/daruda_dnd_group_g2g");
    let ws = wh.root(cx).unwrap();
    let (pa, ga, gb) = ws.update(cx, |ws, cx| {
        let ga = ws.add_group("a".to_string(), None, cx);
        let gb = ws.add_group("b".to_string(), None, cx);
        (ws.projects[0].id, ga, gb)
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

    let (group_t_before, proj_t_before) = ws.read_with(cx, |ws, _| {
        (
            ws.groups.iter().find(|g| g.id == ga).unwrap().tab_order,
            ws.projects.iter().find(|p| p.id == pa).unwrap().tab_order,
        )
    });
    assert!(group_t_before > proj_t_before);
    // Move group before the ungrouped project → group's tab_order
    // drops below the project's.
    ws.update(cx, |ws, cx| {
        ws.reorder_group_before_top_row(ga, TopRow::Project(pa), cx)
    });
    ws.read_with(cx, |ws, _| {
        let group_t = ws.groups.iter().find(|g| g.id == ga).unwrap().tab_order;
        let proj_t = ws.projects.iter().find(|p| p.id == pa).unwrap().tab_order;
        assert!(group_t < proj_t);
    });

    ws.update(cx, |ws, cx| ws.move_project_to_group(pa, Some(ga), cx));
    let group_order = |ws: &Workspace| {
        ws.groups
            .iter()
            .map(|g| (g.id, g.tab_order))
            .collect::<Vec<_>>()
    };
    let before = ws.read_with(cx, |ws, _| group_order(ws));
    // Attempt to drop the drag group on the grouped project — must be
    // rejected because groups never sit inside groups.
    ws.update(cx, |ws, cx| {
        ws.reorder_group_before_top_row(gb, TopRow::Project(pa), cx)
    });
    let after = ws.read_with(cx, |ws, _| group_order(ws));
    assert_eq!(before, after);

    let before = ws.read_with(cx, |ws, _| group_order(ws));
    ws.update(cx, |ws, cx| {
        ws.reorder_group_before_top_row(gb, TopRow::Group(gb), cx)
    });
    let after = ws.read_with(cx, |ws, _| group_order(ws));
    assert_eq!(before, after);
}
