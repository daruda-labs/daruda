//! Public API tests for project.

use daruda_project::project::Project;
use daruda_store::project::{ProjectOverride, ProjectState, ProjectUuid};

#[test]
fn a_chosen_base_branch_wins_over_the_detected_default() {
    let dir = std::env::temp_dir().join("daruda_project_effective_base");
    let mut p = Project::bootstrap_placeholder(0, dir);
    assert_eq!(p.effective_base_branch(), None);
    p.default_branch = Some("main".into());
    assert_eq!(p.effective_base_branch(), Some("main"));
    p.base_branch = Some("develop".into());
    assert_eq!(p.effective_base_branch(), Some("develop"));
}

#[test]
fn bootstrap_non_git_yields_default_worktree() {
    let dir = std::env::temp_dir().join("daruda_project_bootstrap_non_git");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let p = Project::bootstrap(0, dir.clone());
    assert_eq!(p.lanes.len(), 1);
    assert_eq!(p.last_active_lane_id, p.lanes[0].id);
    assert!(p.group_id.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bootstrap_placeholder_skips_git_discovery() {
    // current_dir is a git repo in CI/dev runs — the placeholder
    // must still yield the single pure `Default` lane, proving no
    // git probe ran during construction.
    let root = std::env::current_dir().unwrap();
    let p = Project::bootstrap_placeholder(3, root.clone());
    assert_eq!(p.lanes.len(), 1);
    assert!(!p.lanes[0].is_git());
    assert_eq!(p.lanes[0].path, root);
    assert_eq!(p.last_active_lane_id, 0);
    assert_eq!(p.id, 3);
}

#[test]
fn from_disk_empty_lanes_rebootstraps() {
    // A persisted project with an empty lane list (corrupt /
    // interrupted save) must self-heal on restore instead of
    // yielding a lane-less project the left dock cannot render.
    let dir = std::env::temp_dir().join("daruda_project_from_disk_empty");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let ps = ProjectState {
        schema_version: 0,
        uuid: ProjectUuid::new(),
        root: dir.clone(),
        name: Some("daruda".to_string()),
        lanes: Vec::new(),
        last_active_lane_id: 0,
        next_lane_id: 0,
        default_branch: None,
        base_branch: None,
    };
    let p = Project::from_disk(0, &ps, &ProjectOverride::default());
    assert!(
        !p.lanes.is_empty(),
        "empty persisted lanes must re-bootstrap"
    );
    // Non-git temp dir → exactly one `Default` lane at id 0.
    assert_eq!(p.lanes.len(), 1);
    assert_eq!(p.lanes[0].id, 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn from_disk_empty_lanes_resets_stale_last_active_hint() {
    // Re-bootstrapping assigns lane ids from 0, but the persisted
    // `last_active_lane_id` was captured against the (now-discarded)
    // old lane set and may point at an id the new lanes never
    // produced. Keeping that stale value would let `snap_target`'s
    // fallback mask the inconsistency while the field round-trips
    // back to disk on the next save — a later session whose ids
    // happen to grow back into that range would then activate an
    // unrelated lane on a hint that just coincidentally matched.
    // The re-bootstrap branch must reset the hint to the synthesized
    // first lane.
    let dir = std::env::temp_dir().join("daruda_project_from_disk_stale_hint");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let ps = ProjectState {
        schema_version: 0,
        uuid: ProjectUuid::new(),
        root: dir.clone(),
        name: Some("daruda".to_string()),
        lanes: Vec::new(),
        last_active_lane_id: 7,
        next_lane_id: 0,
        default_branch: None,
        base_branch: None,
    };
    let p = Project::from_disk(0, &ps, &ProjectOverride::default());
    assert_eq!(p.lanes.len(), 1);
    assert_eq!(
        p.last_active_lane_id, p.lanes[0].id,
        "stale hint must follow the synthesized lane set, not survive into the next snapshot"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn from_disk_nonempty_lanes_hydrated_verbatim() {
    // The healthy path must hydrate the stored lanes as-is and never
    // touch the filesystem — a re-bootstrap would renumber the lane
    // to id 0, so a surviving non-zero id proves no fallback fired.
    let dir = std::env::temp_dir().join("daruda_project_from_disk_nonempty");
    let stored = daruda_project::lane::Lane::default_for_project(3, dir.clone()).to_serialized();
    let ps = ProjectState {
        schema_version: 0,
        uuid: ProjectUuid::new(),
        root: dir.clone(),
        name: Some("daruda".to_string()),
        lanes: vec![stored],
        last_active_lane_id: 3,
        next_lane_id: 4,
        default_branch: None,
        base_branch: None,
    };
    let p = Project::from_disk(0, &ps, &ProjectOverride::default());
    assert_eq!(p.lanes.len(), 1);
    assert_eq!(
        p.lanes[0].id, 3,
        "stored lane hydrated verbatim, not re-bootstrapped"
    );
}

#[test]
fn from_disk_reads_persisted_branch_fields() {
    // `from_disk` reads the persisted `default_branch` / `base_branch`
    // verbatim — no git call. A separate reconcile pass owns refreshing them.
    let dir = std::env::temp_dir().join("daruda_project_from_disk_branches");
    let stored = daruda_project::lane::Lane::default_for_project(0, dir.clone()).to_serialized();
    let ps = ProjectState {
        schema_version: 0,
        uuid: ProjectUuid::new(),
        root: dir.clone(),
        name: Some("daruda".to_string()),
        lanes: vec![stored],
        last_active_lane_id: 0,
        next_lane_id: 1,
        default_branch: Some("main".to_string()),
        base_branch: Some("develop".to_string()),
    };
    let p = Project::from_disk(0, &ps, &ProjectOverride::default());
    assert_eq!(p.default_branch.as_deref(), Some("main"));
    assert_eq!(p.base_branch.as_deref(), Some("develop"));
}

#[test]
fn snap_target_falls_back_to_first_when_hint_stale() {
    let dir = std::env::temp_dir().join("daruda_project_snap_stale");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut p = Project::bootstrap(7, dir.clone());
    p.last_active_lane_id = 999; // not in p.lanes
    let snap = p.snap_target().unwrap();
    assert_eq!(snap.project, 7);
    assert_eq!(snap.lane, p.lanes[0].id);
    let _ = std::fs::remove_dir_all(&dir);
}
