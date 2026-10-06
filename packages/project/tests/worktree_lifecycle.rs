//! A lane's git worktree through the package's public API alone: create it
//! on a new branch, find it again, remove it, and remove it a second time.

use std::path::Path;

use daruda_project::lane::git::{
    add_lane, current_branch, delete_branch, has_git, init, list_worktrees, remove_lane,
    repo_root,
};

fn commit_empty(repo: &Path) {
    let status = daruda_core::process::command("git")
        .current_dir(repo)
        .args(["-c", "user.email=daruda@test", "-c", "user.name=daruda"])
        .args(["commit", "--allow-empty", "--quiet", "-m", "initial"])
        .status()
        .expect("git commit spawns");
    assert!(status.success(), "initial commit");
}

#[test]
fn a_lane_worktree_is_created_listed_and_removed() {
    if !has_git() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    // `/tmp` is a symlink on macOS; git reports resolved paths.
    let base = daruda_core::path::canonicalize(temp.path()).unwrap();
    let repo = base.join("repo");
    let lane = base.join("repo-feature");

    init(&repo).unwrap();
    commit_empty(&repo);
    assert_eq!(repo_root(&repo).as_deref(), Some(repo.as_path()));

    add_lane(&repo, &lane, Some("feature"), None).unwrap();
    assert_eq!(current_branch(&lane).unwrap().as_deref(), Some("feature"));
    let listed = list_worktrees(&repo).unwrap();
    assert!(
        listed
            .iter()
            .any(|w| w.path == lane && w.branch.as_deref() == Some("feature")),
        "new lane is listed: {listed:?}"
    );

    remove_lane(&repo, &lane, false).unwrap();
    assert!(!lane.exists());
    assert!(list_worktrees(&repo).unwrap().iter().all(|w| w.path != lane));
    // A lane already gone is the outcome the caller asked for.
    remove_lane(&repo, &lane, false).unwrap();
    delete_branch(&repo, "feature").unwrap();
}
