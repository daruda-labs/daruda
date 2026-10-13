//! Public API tests for projects.

use std::path::PathBuf;

use daruda_store::project::{ProjectOverride, ProjectState};

use daruda_project::lane::Lane;
use daruda_project::project::{Project, Projects};
use daruda_store::project::{LaneRef, ProjectId, ProjectUuid};

/// A project rooted at a fresh non-git directory, so it bootstraps
/// exactly one lane with id 0.
fn project(id: ProjectId, dir_name: &str) -> (Project, PathBuf) {
    let dir = std::env::temp_dir().join(dir_name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let state = ProjectState {
        schema_version: 0,
        uuid: ProjectUuid::new(),
        root: dir.clone(),
        name: None,
        lanes: Vec::new(),
        last_active_lane_id: 0,
        next_lane_id: 0,
        default_branch: None,
        base_branch: None,
    };
    (
        Project::from_disk(id, &state, &ProjectOverride::default()),
        dir,
    )
}

#[test]
fn a_lane_ref_reaches_its_own_projects_lane_not_a_same_numbered_one() {
    let (first, first_dir) = project(0, "daruda_projects_lookup_a");
    let (mut second, second_dir) = project(1, "daruda_projects_lookup_b");
    second.group_id = Some(7);
    let second_uuid = second.uuid;
    let projects = Projects::from(vec![first, second]);

    // Both projects number their only lane 0; the ref tells them apart.
    let target = LaneRef {
        project: 1,
        lane: 0,
    };
    let lane = projects.lane(target).expect("project 1's lane 0");
    assert!(std::ptr::eq(lane, &projects[1].lanes[0]));
    assert_eq!(projects.by_uuid(second_uuid).map(|p| p.id), Some(1));
    assert!(projects.get(2).is_none());

    let refs: Vec<LaneRef> = projects.lanes().map(|(r, _, _)| r).collect();
    assert_eq!(
        refs,
        vec![
            LaneRef {
                project: 0,
                lane: 0
            },
            target
        ]
    );
    assert_eq!(
        projects.ungrouped().map(|p| p.id).collect::<Vec<_>>(),
        vec![0]
    );
    assert_eq!(
        projects.in_group(7).map(|p| p.id).collect::<Vec<_>>(),
        vec![1]
    );

    let _ = std::fs::remove_dir_all(first_dir);
    let _ = std::fs::remove_dir_all(second_dir);
}

#[test]
fn a_path_is_owned_by_the_deepest_lane_holding_it_in_any_project() {
    let (outer, outer_dir) = project(0, "daruda_projects_owning_outer");
    let (mut other, other_dir) = project(1, "daruda_projects_owning_other");
    // A worktree checked out inside the first project's root, filed under
    // the second project: the deeper lane wins even across projects.
    let nested_dir = outer_dir.join("nested");
    std::fs::create_dir_all(&nested_dir).unwrap();
    let nested = Lane::default_for_project(5, nested_dir.clone());
    other.lanes.push(nested);
    let projects = Projects::from(vec![outer, other]);

    let in_nested = nested_dir.join("a.txt");
    let in_outer = outer_dir.join("b.txt");
    let in_other = other_dir.join("c.txt");
    assert_eq!(
        projects.lane_owning(&in_nested),
        Some(LaneRef {
            project: 1,
            lane: 5
        })
    );
    assert_eq!(
        projects.lane_owning(&in_outer),
        Some(LaneRef {
            project: 0,
            lane: 0
        })
    );
    assert_eq!(
        projects.lane_owning(&in_other),
        Some(LaneRef {
            project: 1,
            lane: 0
        })
    );
    assert_eq!(
        projects.lane_owning(std::path::Path::new("/nowhere/at/all")),
        None
    );

    let _ = std::fs::remove_dir_all(outer_dir);
    let _ = std::fs::remove_dir_all(other_dir);
}
