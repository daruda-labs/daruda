//! Public API tests for lane.

use daruda_project::lane::availability::LaneAvailability;
use daruda_project::lane::{Lane, git};
use daruda_store::project::{LaneKind, LaneStatus};
use std::path::PathBuf;

#[test]
fn default_for_project_is_default_kind() {
    let w = Lane::default_for_project(0, PathBuf::from("/tmp/scratch"));
    assert_eq!(w.kind, LaneKind::Default);
    assert!(!w.is_git());
    assert_eq!(w.status, LaneStatus::Idle);
    assert!(w.last_activity > 0);
}

#[test]
fn display_name_uses_path_basename_for_default() {
    let w = Lane::default_for_project(0, PathBuf::from("/Users/alice/scratch"));
    assert_eq!(w.display_name(), "scratch");
}

#[test]
fn display_name_uses_branch_for_git() {
    let mut w = Lane::default_for_project(0, PathBuf::from("/repo"));
    w.kind = LaneKind::Git {
        branch: Some("feat/sidebar".into()),
        repo_root: PathBuf::from("/repo"),
        worktree_root: PathBuf::from("/repo"),
    };
    assert_eq!(w.display_name(), "feat/sidebar");
}

#[test]
fn display_name_prefers_user_name() {
    let mut w = Lane::default_for_project(0, PathBuf::from("/tmp"));
    w.name = Some("My Work".into());
    assert_eq!(w.display_name(), "My Work");
}

#[test]
fn bootstrap_non_git_returns_single_default() {
    let dir = std::env::temp_dir().join("daruda_boot_non_git");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let wts = Lane::bootstrap_from_project(&dir);
    assert_eq!(wts.len(), 1);
    assert_eq!(wts[0].id, 0);
    assert_eq!(wts[0].kind, LaneKind::Default);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bootstrap_git_repo_single_worktree() {
    if !git::has_git() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("daruda_boot_git_single_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let dir = daruda_core::path::canonicalize(&dir).unwrap();
    git::init(&dir).unwrap();
    std::process::Command::new("git")
        .current_dir(&dir)
        .args(["config", "user.email", "daruda@test"])
        .output()
        .unwrap();
    std::process::Command::new("git")
        .current_dir(&dir)
        .args(["config", "user.name", "daruda"])
        .output()
        .unwrap();
    std::process::Command::new("git")
        .current_dir(&dir)
        .args(["commit", "--allow-empty", "-m", "initial"])
        .output()
        .unwrap();

    let wts = Lane::bootstrap_from_project(&dir);
    assert_eq!(wts.len(), 1);
    assert!(wts[0].is_git());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bootstrap_git_repo_multi_worktree_sorts_main_first() {
    if !git::has_git() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("daruda_boot_git_multi_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let dir = daruda_core::path::canonicalize(&dir).unwrap();
    git::init(&dir).unwrap();
    let _ = std::process::Command::new("git")
        .current_dir(&dir)
        .args(["config", "user.email", "daruda@test"])
        .output();
    let _ = std::process::Command::new("git")
        .current_dir(&dir)
        .args(["config", "user.name", "daruda"])
        .output();
    let _ = std::process::Command::new("git")
        .current_dir(&dir)
        .args(["commit", "--allow-empty", "-m", "initial"])
        .output();

    let extra = dir.join("wt-side");
    git::add_lane(&dir, &extra, Some("side"), None).unwrap();

    let wts = Lane::bootstrap_from_project(&dir);
    assert_eq!(wts.len(), 2);
    // The project_root's lane must sort to id 0.
    assert_eq!(wts[0].path, dir);
    assert_eq!(wts[0].id, 0);
    assert_eq!(wts[1].id, 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn round_trip_through_serialized_drops_tabs_only() {
    let w = Lane {
        id: 7,
        kind: LaneKind::Git {
            branch: Some("main".into()),
            repo_root: PathBuf::from("/repo"),
            worktree_root: PathBuf::from("/repo/wt-feat"),
        },
        path: PathBuf::from("/repo/wt-feat"),
        name: Some("Feature branch".into()),
        tab_order: 2,
        is_unread: true,
        last_activity: 12345,
        status: LaneStatus::Running,
        is_main: false,
        base_ref: Some("origin/main".into()),
        description: Some("PR #123".into()),
        remote_cwd: Some("/remote/path".into()),
        session_host: None,
        // Runtime-only — must not survive the round-trip; the
        // assertion below proves it resets to the default.
        availability: LaneAvailability::Missing,
    };
    let s = w.to_serialized();
    let back = Lane::from_serialized(&s);
    assert_eq!(back.id, 7);
    assert_eq!(back.kind, w.kind);
    assert_eq!(back.path, w.path);
    assert_eq!(back.name, w.name);
    assert_eq!(back.tab_order, 2);
    assert!(back.is_unread);
    assert_eq!(back.last_activity, 12345);
    // Status is runtime-only and always resets to Idle.
    assert_eq!(back.status, LaneStatus::Idle);
    // Availability is runtime-only and never serialized — it must
    // load as the default `Present` regardless of the source value.
    assert_eq!(back.availability, LaneAvailability::Present);
    // Serialized form carries no tabs.
    assert!(s.tabs.is_empty());
    assert_eq!(s.active_tab_index, 0);
    // base_ref + description + remote_cwd survive the round-trip.
    assert_eq!(back.base_ref, Some("origin/main".to_string()));
    assert_eq!(back.description, Some("PR #123".to_string()));
    assert_eq!(back.remote_cwd, Some("/remote/path".to_string()));
}

#[test]
fn remote_cwd_defaults_to_none_when_absent() {
    let w = Lane::default_for_project(0, PathBuf::from("/tmp"));
    assert_eq!(w.remote_cwd, None);
    let s = w.to_serialized();
    assert_eq!(s.remote_cwd, None);
    let back = Lane::from_serialized(&s);
    assert_eq!(back.remote_cwd, None);
}

/// All three variants survive the runtime↔serialized round-trip, and
/// the distinction the precedence rule depends on — never answered
/// (`None`) vs. explicitly local (`Some(Local)`) — survives with them.
#[test]
fn session_host_round_trips_every_variant() {
    use daruda_store::project::LaneSessionHost;
    let cases = [
        None,
        Some(LaneSessionHost::Local),
        Some(LaneSessionHost::Ssh {
            target: "buildbox".into(),
            session_path: "/home/user/project".into(),
            registry_id: None,
        }),
        Some(LaneSessionHost::Docker {
            container: "dev-1".into(),
            session_path: "/workspace".into(),
            registry_id: None,
        }),
    ];
    for case in cases {
        let mut w = Lane::default_for_project(0, PathBuf::from("/tmp"));
        w.set_session_host(case.clone());
        let back = Lane::from_serialized(&w.to_serialized());
        assert_eq!(back.session_host, case);
    }
}

/// A lane saved before this field existed must load as "never
/// answered", which is what keeps its legacy `remote_cwd` in play.
#[test]
fn a_lane_saved_without_a_session_host_loads_as_unanswered() {
    let json = r#"{
        "id": 3,
        "kind": { "type": "default" },
        "path": "/tmp/lane",
        "remote_cwd": "/remote/path"
    }"#;
    let s: daruda_store::project::SerializedLane =
        serde_json::from_str(json).expect("legacy lane JSON must still load");
    assert_eq!(s.session_host, None);
    assert_eq!(s.remote_cwd, Some("/remote/path".to_string()));
}

#[test]
fn set_remote_cwd_overwrites_and_clears() {
    let mut w = Lane::default_for_project(0, PathBuf::from("/tmp"));
    w.set_remote_cwd(Some("/remote/vm/path".into()));
    assert_eq!(w.remote_cwd, Some("/remote/vm/path".to_string()));
    w.set_remote_cwd(None);
    assert_eq!(w.remote_cwd, None);
}

#[test]
fn from_serialized_recomputes_is_main_true_when_roots_match() {
    // Verifies that `from_serialized` actually fires the is_main recomputation:
    // a fixture where worktree_root == repo_root must round-trip to is_main=true.
    let w = Lane {
        id: 3,
        kind: LaneKind::Git {
            branch: Some("main".into()),
            repo_root: PathBuf::from("/repo"),
            worktree_root: PathBuf::from("/repo"),
        },
        path: PathBuf::from("/repo"),
        name: None,
        tab_order: 0,
        is_unread: false,
        last_activity: 99999,
        status: LaneStatus::Running,
        is_main: false, // intentionally wrong — from_serialized must correct it
        base_ref: None,
        description: None,
        remote_cwd: None,
        session_host: None,
        availability: LaneAvailability::Present,
    };
    let s = w.to_serialized();
    let back = Lane::from_serialized(&s);
    assert!(
        back.is_main,
        "from_serialized must recompute is_main=true when worktree_root == repo_root"
    );
}

// -- is_main derivation tests -----------------------------------------

#[test]
fn is_main_true_when_worktree_root_equals_repo_root() {
    let lane = Lane::git(
        0,
        PathBuf::from("/repo"),
        Some("main".into()),
        PathBuf::from("/repo"),
        PathBuf::from("/repo"),
        0,
    );
    assert!(lane.is_main);
}

#[test]
fn is_main_false_for_linked_worktree() {
    let lane = Lane::git(
        1,
        PathBuf::from("/repo-feat"),
        Some("feat/x".into()),
        PathBuf::from("/repo"),
        PathBuf::from("/repo-feat"),
        1,
    );
    assert!(!lane.is_main);
}

#[test]
fn is_main_false_for_default_kind() {
    let lane = Lane::default_for_project(0, PathBuf::from("/non-git"));
    assert!(!lane.is_main);
}

#[test]
fn is_main_true_when_path_is_subdir_but_worktree_root_equals_repo_root() {
    // User opened a subdirectory; path is anchored to the subdir, but
    // worktree_root stays at the true git toplevel which equals repo_root.
    let lane = Lane::git(
        0,
        PathBuf::from("/repo/sub"),
        Some("main".into()),
        PathBuf::from("/repo"),
        PathBuf::from("/repo"),
        0,
    );
    assert!(
        lane.is_main,
        "subdir-anchored main lane must still be is_main"
    );
}
