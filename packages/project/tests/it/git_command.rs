//! Public API tests for git command.

use daruda_project::lane::git::git_command;
use std::path::Path;
use std::process::Command;

fn run(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(dir)
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?} failed in {dir:?}");
}

/// The reason this module exists: a status read must leave `.git/index`
/// byte-identical. The plain run at the end is what proves the fixture
/// was actually stale — without it the first assert could pass on a repo
/// git had nothing to refresh in.
#[test]
fn a_status_read_does_not_rewrite_the_index() {
    if !daruda_project::lane::git::has_git() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    run(root, &["init", "-q"]);
    run(root, &["config", "user.email", "daruda@test"]);
    run(root, &["config", "user.name", "daruda"]);
    std::fs::write(root.join("f.txt"), b"one\n").unwrap();
    run(root, &["add", "f.txt"]);
    run(root, &["commit", "-qm", "initial"]);

    // Re-write the same bytes under a later mtime so the index's stat
    // cache goes stale. The sleep buys a distinct timestamp: within one
    // filesystem tick git sees nothing to refresh and the test would pass
    // for the wrong reason.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    std::fs::write(root.join("f.txt"), b"one\n").unwrap();

    let index = root.join(".git").join("index");
    let before = std::fs::read(&index).unwrap();

    git_command(root)
        .args(["status", "--porcelain=v1"])
        .output()
        .unwrap();
    assert_eq!(
        std::fs::read(&index).unwrap(),
        before,
        "a status read through the builder must not touch the index"
    );

    Command::new("git")
        .current_dir(root)
        .args(["status", "--porcelain=v1"])
        .output()
        .unwrap();
    assert_ne!(
        std::fs::read(&index).unwrap(),
        before,
        "the fixture was not stale — the assert above proved nothing"
    );
}
