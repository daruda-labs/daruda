//! Public API tests for git.

use daruda_project::lane::git::*;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

// Temp-dir helpers — each test gets a unique path so parallel
// runs don't collide. The directory is created on demand and
// torn down at the end of the test.
fn unique_tmpdir(prefix: &str) -> PathBuf {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let id = SEQ.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("daruda_git_{prefix}_{pid}_{nonce}_{id}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // Resolve symlinks (on macOS `/tmp` → `/private/tmp`) so
    // direct PathBuf comparisons against git's resolved output
    // don't break.
    daruda_core::path::canonicalize(&dir).unwrap()
}

fn teardown(path: &Path) {
    let _ = std::fs::remove_dir_all(path);
}

// Skip-if-no-git guard for the whole module.
fn require_git() -> bool {
    has_git()
}

fn commit_initial(repo: &Path) {
    for args in [
        vec!["config", "user.email", "daruda@test"],
        vec!["config", "user.name", "daruda"],
        vec!["commit", "--allow-empty", "-m", "initial"],
    ] {
        let output = git_command(repo).args(&args).output().unwrap();
        assert!(output.status.success(), "git {args:?}: {output:?}");
    }
}

/// What `-z` is for: without it git C-quotes a path holding a space or a
/// non-ASCII byte (`"\355\225\234…"`), and the quoted text was taken as the
/// path — so staging, diffs and opening that file all failed.
#[test]
fn a_real_status_reports_spaced_and_non_ascii_paths_verbatim() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let git = |args: &[&str]| {
        let out = git_command(root).args(args).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
    };
    git(&["init", "-q"]);
    git(&["config", "user.email", "t@example.com"]);
    git(&["config", "user.name", "t"]);
    std::fs::write(root.join("old name.rs"), "a\nb\n").unwrap();
    std::fs::write(root.join("한글.md"), "x\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "init"]);
    git(&["mv", "old name.rs", "새 이름.rs"]);
    std::fs::write(root.join("한글.md"), "x\ny\n").unwrap();
    std::fs::write(root.join("untracked 파일.txt"), "").unwrap();

    let status = git_worktree_status(root).unwrap();
    let staged: Vec<_> = status.staged.iter().map(|e| e.path.clone()).collect();
    let unstaged: Vec<_> = status.unstaged.iter().map(|e| e.path.clone()).collect();
    assert_eq!(staged, vec![PathBuf::from("새 이름.rs")]);
    assert_eq!(
        status.staged[0].original_path,
        Some(PathBuf::from("old name.rs"))
    );
    assert_eq!(
        unstaged,
        vec![
            PathBuf::from("한글.md"),
            PathBuf::from("untracked 파일.txt")
        ]
    );
    assert_eq!(status.diffstat.get(Path::new("한글.md")), Some(&(1, 0)));
    assert!(status.diffstat.contains_key(Path::new("새 이름.rs")));
}

#[test]
fn init_creates_repo() {
    if !require_git() {
        return;
    }
    let dir = unique_tmpdir("init");
    init(&dir).unwrap();
    assert!(is_git_repo(&dir));
    assert_eq!(repo_root(&dir).as_deref(), Some(dir.as_path()));
    teardown(&dir);
}

#[test]
fn is_git_repo_false_for_plain_dir() {
    if !require_git() {
        return;
    }
    let dir = unique_tmpdir("plain");
    assert!(!is_git_repo(&dir));
    assert!(repo_root(&dir).is_none());
    teardown(&dir);
}

#[test]
fn current_branch_after_init_is_main_or_master() {
    if !require_git() {
        return;
    }
    let dir = unique_tmpdir("branch");
    init(&dir).unwrap();
    commit_initial(&dir);
    let b = current_branch(&dir).unwrap();
    // Git defaults to `main` on newer versions, `master` on older.
    assert!(b.as_deref() == Some("main") || b.as_deref() == Some("master"));
    teardown(&dir);
}

#[test]
fn head_message_errors_when_no_commits() {
    if !require_git() {
        return;
    }
    let dir = unique_tmpdir("headmsg_empty");
    init(&dir).unwrap();
    // No commit yet → `git log -1` fails; the empty-box amend path turns this
    // into the "nothing to amend" toast.
    assert!(git_head_message(&dir).is_err());
    teardown(&dir);
}

#[test]
fn default_branch_none_on_non_repo() {
    if !require_git() {
        return;
    }
    let dir = unique_tmpdir("default_nonrepo");
    // Plain directory, never `git init`-ed.
    assert_eq!(default_branch(&dir), None);
    teardown(&dir);
}

#[test]
fn add_and_list_worktrees_round_trip() {
    if !require_git() {
        return;
    }
    let dir = unique_tmpdir("addlist");
    init(&dir).unwrap();
    commit_initial(&dir);

    let wt_path = dir.join("wt-feature");
    add_lane(&dir, &wt_path, Some("feat/xyz"), None).unwrap();

    let listed = list_worktrees(&dir).unwrap();
    assert_eq!(listed.len(), 2);
    let feat = listed.iter().find(|w| w.path == wt_path).unwrap();
    assert_eq!(feat.branch.as_deref(), Some("feat/xyz"));
    teardown(&dir);
}

#[test]
fn remove_worktree_deletes_entry() {
    if !require_git() {
        return;
    }
    let dir = unique_tmpdir("remove");
    init(&dir).unwrap();
    commit_initial(&dir);

    let wt_path = dir.join("wt-remove");
    add_lane(&dir, &wt_path, Some("scratch"), None).unwrap();
    assert_eq!(list_worktrees(&dir).unwrap().len(), 2);

    remove_lane(&dir, &wt_path, false).unwrap();
    let listed = list_worktrees(&dir).unwrap();
    assert_eq!(listed.len(), 1);
    assert!(listed.iter().all(|w| w.path != wt_path));
    teardown(&dir);
}

#[test]
fn remove_lane_succeeds_when_the_worktree_was_already_removed_elsewhere() {
    // Mirrors the reported bug: the worktree was already fully removed
    // (directory + git registration) from a *different* environment —
    // e.g. another terminal running `git worktree remove` directly, or a
    // prior daruda removal that raced. A second removal attempt from
    // daruda's UI must not get stuck: git reports "is not a working
    // tree" (exit 128) because there is genuinely nothing left to
    // remove, and that outcome — no worktree at this path — is already
    // what the user wanted.
    if !require_git() {
        return;
    }
    let dir = unique_tmpdir("extfullremove");
    init(&dir).unwrap();
    commit_initial(&dir);

    let wt_path = dir.join("wt-extfull");
    add_lane(&dir, &wt_path, Some("scratch/extfull"), None).unwrap();
    remove_lane(&dir, &wt_path, false).unwrap(); // the "external" removal

    // daruda's Lane entry is still sitting in the list at this point —
    // the user now clicks Remove in the app for the same target.
    remove_lane(&dir, &wt_path, false).unwrap();
    teardown(&dir);
}

#[test]
fn add_worktree_with_existing_branch_fails_cleanly() {
    if !require_git() {
        return;
    }
    let dir = unique_tmpdir("dup");
    init(&dir).unwrap();
    commit_initial(&dir);

    let first = dir.join("wt-first");
    add_lane(&dir, &first, Some("dup-branch"), None).unwrap();

    // Re-using `-b dup-branch` must fail — ensures GitError::Exit
    // carries a useful message.
    let second = dir.join("wt-second");
    let err = add_lane(&dir, &second, Some("dup-branch"), None).unwrap_err();
    match err {
        GitError::Exit { stderr, .. } => {
            assert!(!stderr.is_empty());
        }
        other => panic!("expected Exit error, got {other:?}"),
    }
    teardown(&dir);
}

#[test]
fn delete_branch_after_worktree_removal() {
    if !require_git() {
        return;
    }
    let dir = unique_tmpdir("delbranch");
    init(&dir).unwrap();
    commit_initial(&dir);

    // Create a lane (creates the branch as a side effect),
    // then remove the lane so the branch is no longer checked
    // out anywhere — the modal flow does this same sequence.
    let wt = dir.join("wt-temp");
    add_lane(&dir, &wt, Some("temp/work"), None).unwrap();
    remove_lane(&dir, &wt, false).unwrap();

    // Branch still exists post-remove (git keeps it).
    let pre = std::process::Command::new("git")
        .current_dir(&dir)
        .args(["rev-parse", "--verify", "temp/work"])
        .output()
        .unwrap();
    assert!(pre.status.success(), "branch should exist before delete");

    delete_branch(&dir, "temp/work").unwrap();

    let post = std::process::Command::new("git")
        .current_dir(&dir)
        .args(["rev-parse", "--verify", "temp/work"])
        .output()
        .unwrap();
    assert!(!post.status.success(), "branch must be gone after delete");
    teardown(&dir);
}

#[test]
fn delete_branch_rejects_currently_checked_out() {
    if !require_git() {
        return;
    }
    let dir = unique_tmpdir("delbranch_curr");
    init(&dir).unwrap();
    commit_initial(&dir);
    // The default branch (main/master, depending on git version)
    // is currently checked out at `dir`. Attempting to delete it
    // must fail cleanly.
    let current = current_branch(&dir).unwrap().unwrap();
    let err = delete_branch(&dir, &current).unwrap_err();
    match err {
        GitError::Exit { stderr, .. } => assert!(!stderr.is_empty()),
        other => panic!("expected Exit error, got {other:?}"),
    }
    teardown(&dir);
}

#[test]
fn git_add_stages_file() {
    if !require_git() {
        return;
    }
    let dir = unique_tmpdir("git_add");
    init(&dir).unwrap();
    commit_initial(&dir);

    let file = dir.join("hello.txt");
    std::fs::write(&file, "hello").unwrap();

    // File is untracked — not in staged before add.
    let before = git_worktree_status(&dir).unwrap();
    assert!(before.staged.is_empty(), "nothing staged before git_add");

    git_add(&dir, std::path::Path::new("hello.txt")).unwrap();

    let after = git_worktree_status(&dir).unwrap();
    assert_eq!(after.staged.len(), 1, "one staged entry after git_add");
    assert_eq!(after.staged[0].x, 'A');
    teardown(&dir);
}

#[test]
fn git_add_all_stages_all_files() {
    if !require_git() {
        return;
    }
    let dir = unique_tmpdir("git_add_all");
    init(&dir).unwrap();
    commit_initial(&dir);

    std::fs::write(dir.join("a.txt"), "a").unwrap();
    std::fs::write(dir.join("b.txt"), "b").unwrap();

    git_add_all(&dir).unwrap();

    let status = git_worktree_status(&dir).unwrap();
    assert_eq!(status.staged.len(), 2, "both files staged after add --all");
    teardown(&dir);
}

#[test]
fn git_restore_staged_unstages_file() {
    if !require_git() {
        return;
    }
    let dir = unique_tmpdir("git_restore");
    init(&dir).unwrap();
    commit_initial(&dir);

    let rel = std::path::Path::new("restore_me.txt");
    std::fs::write(dir.join(rel), "data").unwrap();
    git_add(&dir, rel).unwrap();

    let after_add = git_worktree_status(&dir).unwrap();
    assert_eq!(after_add.staged.len(), 1, "file staged after add");

    git_restore_staged(&dir, rel).unwrap();

    let after_restore = git_worktree_status(&dir).unwrap();
    assert!(
        after_restore.staged.is_empty(),
        "nothing staged after restore --staged"
    );
    assert_eq!(
        after_restore.unstaged.len(),
        1,
        "file appears as untracked after unstage"
    );
    teardown(&dir);
}

#[test]
fn add_worktree_with_explicit_base_ref() {
    if !require_git() {
        return;
    }
    let dir = unique_tmpdir("base_ref");
    init(&dir).unwrap();
    commit_initial(&dir);
    // Branch "side" at the initial commit, then advance "main"
    // by another empty commit so HEAD diverges from "side".
    let _ = std::process::Command::new("git")
        .current_dir(&dir)
        .args(["branch", "side"])
        .output();
    let _ = std::process::Command::new("git")
        .current_dir(&dir)
        .args(["commit", "--allow-empty", "-m", "advance-main"])
        .output();

    // Build the lane with explicit base = "side". The new
    // branch must point at "side"'s commit, not the current HEAD.
    let wt_path = dir.join("wt-from-side");
    add_lane(&dir, &wt_path, Some("feat/from-side"), Some("side")).unwrap();

    let side_sha = std::process::Command::new("git")
        .current_dir(&dir)
        .args(["rev-parse", "side"])
        .output()
        .unwrap()
        .stdout;
    let wt_sha = std::process::Command::new("git")
        .current_dir(&wt_path)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap()
        .stdout;
    assert_eq!(side_sha, wt_sha, "lane must inherit base ref's commit");
    teardown(&dir);
}

/// The split this probe exists for. A linked worktree keeps `index` and
/// `HEAD` to itself while sharing `refs/`, so watching only one of the two
/// directories misses either the staging or the fetch.
#[test]
fn git_dirs_separate_a_linked_worktree_from_the_shared_repository() {
    if !require_git() {
        return;
    }
    let dir = unique_tmpdir("git_dirs");
    init(&dir).unwrap();
    commit_initial(&dir);
    let linked = dir.join("side-wt");
    add_lane(&dir, &linked, Some("side"), None).unwrap();

    let main = git_dirs(&dir).unwrap();
    assert!(
        main.git_dir.is_absolute(),
        "relative paths must be resolved"
    );
    assert_eq!(
        main.git_dir, main.common_dir,
        "the main worktree's own dir is the shared one"
    );

    let side = git_dirs(&linked).unwrap();
    assert_ne!(
        side.git_dir, side.common_dir,
        "a linked worktree keeps its own index and HEAD"
    );
    assert_eq!(
        side.common_dir, main.common_dir,
        "both lanes share the refs a fetch moves"
    );
    assert!(side.git_dir.starts_with(&side.common_dir));

    teardown(&dir);
}
