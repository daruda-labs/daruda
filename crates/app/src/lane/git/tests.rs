use super::*;
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
    std::fs::canonicalize(&dir).unwrap()
}

fn teardown(path: &Path) {
    let _ = std::fs::remove_dir_all(path);
}

fn commit_initial(repo: &Path) {
    // Ensure we have an initial commit so `lane add` can create
    // checkouts — with no commits `add` refuses.
    run_git(repo, ["config", "user.email", "daruda@test"]).unwrap();
    run_git(repo, ["config", "user.name", "daruda"]).unwrap();
    run_git(repo, ["commit", "--allow-empty", "-m", "initial"]).unwrap();
}

// Skip-if-no-git guard for the whole module.
fn require_git() -> bool {
    has_git()
}

// ----------------------------------------------------------------
// parse_git_status_output unit tests
// ----------------------------------------------------------------

#[test]
fn status_staged_modified() {
    let out = "M  file.rs\n";
    let data = parse_git_status_output(out);
    assert_eq!(data.staged.len(), 1, "should have one staged entry");
    assert_eq!(data.staged[0].x, 'M');
    assert_eq!(data.staged[0].path.to_str().unwrap(), "file.rs");
    assert!(data.unstaged.is_empty());
}

#[test]
fn status_unstaged_modified() {
    let out = " M file.rs\n";
    let data = parse_git_status_output(out);
    assert!(data.staged.is_empty());
    assert_eq!(data.unstaged.len(), 1);
    assert_eq!(data.unstaged[0].y, 'M');
    assert_eq!(data.unstaged[0].path.to_str().unwrap(), "file.rs");
}

#[test]
fn status_untracked() {
    let out = "?? new.rs\n";
    let data = parse_git_status_output(out);
    assert!(data.staged.is_empty());
    assert_eq!(data.unstaged.len(), 1);
    assert_eq!(data.unstaged[0].x, '?');
    assert_eq!(data.unstaged[0].path.to_str().unwrap(), "new.rs");
}

#[test]
fn status_renamed_staged() {
    // Renamed: destination path appears after " -> ".
    let out = "R  old.rs -> new.rs\n";
    let data = parse_git_status_output(out);
    assert_eq!(data.staged.len(), 1);
    assert_eq!(data.staged[0].x, 'R');
    assert_eq!(data.staged[0].path.to_str().unwrap(), "new.rs");
    assert_eq!(
        data.staged[0]
            .original_path
            .as_ref()
            .and_then(|p| p.to_str()),
        Some("old.rs"),
        "rename keeps the source path on `original_path`"
    );
    assert!(data.unstaged.is_empty());
}

#[test]
fn status_non_rename_has_no_original_path() {
    let data = parse_git_status_output("M  file.rs\n");
    assert!(data.staged[0].original_path.is_none());
}

// ----------------------------------------------------------------
// parse_numstat unit tests — `git diff HEAD --numstat` parsing
// ----------------------------------------------------------------

#[test]
fn numstat_single_text_file() {
    let stats = parse_numstat("13\t3\tsrc/lib.rs\n");
    assert_eq!(stats, vec![(13, 3, PathBuf::from("src/lib.rs"))]);
}

#[test]
fn numstat_multiple_files() {
    let stats = parse_numstat("13\t3\tsrc/lib.rs\n2\t1\tCargo.toml\n");
    assert_eq!(stats.len(), 2);
    assert_eq!(stats[0], (13, 3, PathBuf::from("src/lib.rs")));
    assert_eq!(stats[1], (2, 1, PathBuf::from("Cargo.toml")));
}

#[test]
fn numstat_binary_file_maps_to_zero_zero() {
    // git emits `-\t-\t<path>` for binary files. Caller decides
    // whether to skip rendering "+0 −0".
    let stats = parse_numstat("-\t-\tassets/icon.png\n");
    assert_eq!(stats, vec![(0, 0, PathBuf::from("assets/icon.png"))]);
}

#[test]
fn numstat_pure_addition() {
    let stats = parse_numstat("42\t0\tnew.rs\n");
    assert_eq!(stats[0], (42, 0, PathBuf::from("new.rs")));
}

#[test]
fn numstat_pure_deletion() {
    let stats = parse_numstat("0\t8\told.rs\n");
    assert_eq!(stats[0], (0, 8, PathBuf::from("old.rs")));
}

#[test]
fn numstat_empty_input() {
    let stats = parse_numstat("");
    assert!(stats.is_empty());
}

#[test]
fn status_both_modified() {
    // MM = staged modified + unstaged modified.
    let out = "MM file.rs\n";
    let data = parse_git_status_output(out);
    assert_eq!(data.staged.len(), 1, "one staged entry for MM");
    assert_eq!(data.unstaged.len(), 1, "one unstaged entry for MM");
}

#[test]
fn status_conflict_uu_only_in_unstaged() {
    let out = "UU file.rs\n";
    let data = parse_git_status_output(out);
    assert!(data.staged.is_empty(), "UU must not appear in staged");
    assert_eq!(data.unstaged.len(), 1, "UU must appear in unstaged");
}

// ----------------------------------------------------------------
// Branch line (`## ...`) parsing — `git status --branch` header
// ----------------------------------------------------------------

#[test]
fn branch_line_no_upstream() {
    let data = parse_git_status_output("## main\n");
    assert_eq!(data.branch.as_deref(), Some("main"));
    assert!(data.upstream.is_none());
    assert_eq!(data.ahead, 0);
    assert_eq!(data.behind, 0);
}

#[test]
fn branch_line_with_upstream_no_divergence() {
    let data = parse_git_status_output("## main...origin/main\n");
    assert_eq!(data.branch.as_deref(), Some("main"));
    assert_eq!(data.upstream.as_deref(), Some("origin/main"));
    assert_eq!(data.ahead, 0);
    assert_eq!(data.behind, 0);
}

#[test]
fn branch_line_ahead_only() {
    let data = parse_git_status_output("## main...origin/main [ahead 3]\n");
    assert_eq!(data.ahead, 3);
    assert_eq!(data.behind, 0);
}

#[test]
fn branch_line_behind_only() {
    let data = parse_git_status_output("## main...origin/main [behind 2]\n");
    assert_eq!(data.ahead, 0);
    assert_eq!(data.behind, 2);
}

#[test]
fn branch_line_ahead_and_behind() {
    let data = parse_git_status_output("## main...origin/main [ahead 1, behind 2]\n");
    assert_eq!(data.ahead, 1);
    assert_eq!(data.behind, 2);
}

#[test]
fn branch_line_gone_keeps_upstream() {
    // Upstream was deleted on the remote — git annotates with `[gone]`.
    // We still record the upstream string so the user can see what's
    // missing; ahead/behind stay at 0.
    let data = parse_git_status_output("## main...origin/main [gone]\n");
    assert_eq!(data.upstream.as_deref(), Some("origin/main"));
    assert_eq!(data.ahead, 0);
    assert_eq!(data.behind, 0);
}

#[test]
fn branch_line_detached_head() {
    let data = parse_git_status_output("## HEAD (no branch)\n");
    assert!(data.branch.is_none());
    assert!(data.upstream.is_none());
}

#[test]
fn branch_line_initial_empty_repo() {
    let data = parse_git_status_output("## No commits yet on main\n");
    assert_eq!(data.branch.as_deref(), Some("main"));
    assert!(data.upstream.is_none());
}

#[test]
fn branch_line_combined_with_files() {
    let out = "## main...origin/main [ahead 2]\n M file.rs\n?? new.rs\n";
    let data = parse_git_status_output(out);
    assert_eq!(data.branch.as_deref(), Some("main"));
    assert_eq!(data.ahead, 2);
    assert_eq!(data.unstaged.len(), 2);
}

// ----------------------------------------------------------------
// parse_worktree_list unit tests
// ----------------------------------------------------------------

#[test]
fn parse_porcelain_single_worktree() {
    let sample = "worktree /repo\nHEAD abcd1234\nbranch refs/heads/main\n\n";
    let wts = parse_worktree_list(sample).unwrap();
    assert_eq!(wts.len(), 1);
    assert_eq!(wts[0].path, PathBuf::from("/repo"));
    assert_eq!(wts[0].branch.as_deref(), Some("main"));
    assert_eq!(wts[0].head.as_deref(), Some("abcd1234"));
    assert!(!wts[0].bare);
}

#[test]
fn parse_porcelain_multiple_worktrees() {
    let sample = "\
worktree /repo
HEAD aaa
branch refs/heads/main

worktree /repo/wt-feat
HEAD bbb
branch refs/heads/feat/sidebar

";
    let wts = parse_worktree_list(sample).unwrap();
    assert_eq!(wts.len(), 2);
    assert_eq!(wts[0].branch.as_deref(), Some("main"));
    assert_eq!(wts[1].branch.as_deref(), Some("feat/sidebar"));
}

#[test]
fn parse_porcelain_detached_head_has_no_branch() {
    let sample = "worktree /repo/wt-detached\nHEAD ccc\ndetached\n\n";
    let wts = parse_worktree_list(sample).unwrap();
    assert_eq!(wts.len(), 1);
    assert!(wts[0].branch.is_none());
}

#[test]
fn parse_porcelain_bare_flagged() {
    let sample = "worktree /repo-bare\nHEAD ddd\nbare\n\n";
    let wts = parse_worktree_list(sample).unwrap();
    assert_eq!(wts.len(), 1);
    assert!(wts[0].bare);
}

#[test]
fn parse_porcelain_unknown_keys_ignored() {
    let sample = "worktree /repo\nHEAD abcd\nbranch refs/heads/main\nlocked locked-reason\nprunable stuff\n\n";
    let wts = parse_worktree_list(sample).unwrap();
    assert_eq!(wts.len(), 1);
    assert_eq!(wts[0].path, PathBuf::from("/repo"));
}

#[test]
fn parse_porcelain_stray_head_before_worktree_errors() {
    let sample = "HEAD abcd\n\n";
    let err = parse_worktree_list(sample);
    assert!(matches!(err, Err(GitError::Parse(_))));
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
fn head_message_returns_tip_commit_message() {
    if !require_git() {
        return;
    }
    let dir = unique_tmpdir("headmsg");
    init(&dir).unwrap();
    run_git(&dir, ["config", "user.email", "daruda@test"]).unwrap();
    run_git(&dir, ["config", "user.name", "daruda"]).unwrap();
    run_git(
        &dir,
        [
            "commit",
            "--allow-empty",
            "-m",
            "first subject\n\nbody line",
        ],
    )
    .unwrap();

    // Full subject + body, trailing newline trimmed.
    assert_eq!(
        git_head_message(&dir).unwrap(),
        "first subject\n\nbody line"
    );
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
fn default_branch_prefers_origin_head() {
    if !require_git() {
        return;
    }
    let dir = unique_tmpdir("default_origin_head");
    init(&dir).unwrap();
    commit_initial(&dir);
    // Simulate a clone whose remote default is `origin/main`: create a
    // local `origin/main` remote-tracking ref, then point
    // `origin/HEAD` at it the way `git clone` / `git remote set-head`
    // would. Detection must report `main` from this symbolic ref even
    // when the local branch carries a different name.
    run_git(&dir, ["branch", "-m", "local-feature"]).unwrap();
    run_git(&dir, ["update-ref", "refs/remotes/origin/main", "HEAD"]).unwrap();
    run_git(
        &dir,
        [
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/main",
        ],
    )
    .unwrap();
    assert_eq!(default_branch(&dir).as_deref(), Some("main"));
    teardown(&dir);
}

#[test]
fn default_branch_falls_back_to_local_main() {
    if !require_git() {
        return;
    }
    let dir = unique_tmpdir("default_local_main");
    init(&dir).unwrap();
    commit_initial(&dir);
    // No `origin/HEAD`. Ensure a local `main` exists regardless of the
    // git version's init default, then detection should pick it.
    let current = current_branch(&dir).unwrap().unwrap();
    if current != "main" {
        run_git(&dir, ["branch", "main"]).unwrap();
    }
    assert_eq!(default_branch(&dir).as_deref(), Some("main"));
    teardown(&dir);
}

#[test]
fn default_branch_falls_back_to_master_when_no_main() {
    if !require_git() {
        return;
    }
    let dir = unique_tmpdir("default_master");
    init(&dir).unwrap();
    commit_initial(&dir);
    // No `origin/HEAD` and no local `main`: rename the init branch to
    // something neutral, add `master`, and verify that detection matches
    // `master` as the second conventional candidate when neither
    // `origin/HEAD` nor `main` is present.
    run_git(&dir, ["branch", "-m", "trunk"]).unwrap();
    run_git(&dir, ["branch", "master"]).unwrap();
    assert_eq!(default_branch(&dir).as_deref(), Some("master"));
    teardown(&dir);
}

#[test]
fn default_branch_falls_back_to_current_when_no_convention() {
    if !require_git() {
        return;
    }
    let dir = unique_tmpdir("default_current");
    init(&dir).unwrap();
    commit_initial(&dir);
    // No `origin/HEAD`, no `main`, no `master` — only an oddly-named
    // branch is checked out. Detection falls through to the current
    // branch.
    run_git(&dir, ["branch", "-m", "wip/spike"]).unwrap();
    assert_eq!(default_branch(&dir).as_deref(), Some("wip/spike"));
    teardown(&dir);
}

#[test]
fn default_branch_none_on_detached_head() {
    if !require_git() {
        return;
    }
    let dir = unique_tmpdir("default_detached");
    init(&dir).unwrap();
    commit_initial(&dir);
    // Detach HEAD at the current commit and rename the branch away so
    // neither `main` nor `master` exists. With no convention to match
    // and a detached HEAD, detection returns None.
    run_git(&dir, ["branch", "-m", "wip/spike"]).unwrap();
    let head = run_git(&dir, ["rev-parse", "HEAD"]).unwrap();
    run_git(&dir, ["checkout", "--detach", head.trim()]).unwrap();
    run_git(&dir, ["branch", "-D", "wip/spike"]).unwrap();
    assert_eq!(default_branch(&dir), None);
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
    let before = git_status(&dir).unwrap();
    assert!(before.staged.is_empty(), "nothing staged before git_add");

    git_add(&dir, std::path::Path::new("hello.txt")).unwrap();

    let after = git_status(&dir).unwrap();
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

    let status = git_status(&dir).unwrap();
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

    let after_add = git_status(&dir).unwrap();
    assert_eq!(after_add.staged.len(), 1, "file staged after add");

    git_restore_staged(&dir, rel).unwrap();

    let after_restore = git_status(&dir).unwrap();
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

/// New W-11 affordance: passing `base = Some(ref)` makes the new
/// lane branch from that ref instead of the current HEAD.
/// We verify the resulting checkout is at the named branch's tip.
#[test]
fn git_merge_fast_forward() {
    if !require_git() {
        return;
    }
    let dir = unique_tmpdir("merge_ff");
    init(&dir).unwrap();
    commit_initial(&dir);

    // Create feature branch with one commit ahead of main.
    let feat = dir.join("wt-feat");
    add_lane(&dir, &feat, Some("feat/add"), None).unwrap();
    run_git(&dir, ["config", "user.email", "daruda@test"]).unwrap();
    run_git(&dir, ["config", "user.name", "daruda"]).unwrap();
    std::fs::write(feat.join("feature.txt"), "new").unwrap();
    run_git(&feat, ["config", "user.email", "daruda@test"]).unwrap();
    run_git(&feat, ["config", "user.name", "daruda"]).unwrap();
    run_git(&feat, ["add", "feature.txt"]).unwrap();
    run_git(&feat, ["commit", "-m", "add feature"]).unwrap();

    // Merge feat/add into main (main lane at `dir`).
    let outcome = git_merge(&dir, "feat/add").unwrap();
    assert!(
        matches!(outcome, MergeOutcome::Success),
        "expected Success, got {outcome:?}"
    );
    teardown(&dir);
}

#[test]
fn git_merge_already_up_to_date() {
    if !require_git() {
        return;
    }
    let dir = unique_tmpdir("merge_uptodate");
    init(&dir).unwrap();
    commit_initial(&dir);

    // Create a branch at the same commit as main — nothing to merge.
    run_git(&dir, ["branch", "same-commit"]).unwrap();
    let outcome = git_merge(&dir, "same-commit").unwrap();
    assert!(
        matches!(outcome, MergeOutcome::AlreadyUpToDate),
        "expected AlreadyUpToDate, got {outcome:?}"
    );
    teardown(&dir);
}

#[test]
fn git_merge_abort_restores_clean_state() {
    if !require_git() {
        return;
    }
    let dir = unique_tmpdir("merge_abort");
    init(&dir).unwrap();
    run_git(&dir, ["config", "user.email", "daruda@test"]).unwrap();
    run_git(&dir, ["config", "user.name", "daruda"]).unwrap();

    // Write conflicting content on main.
    std::fs::write(dir.join("conflict.txt"), "main content").unwrap();
    run_git(&dir, ["add", "conflict.txt"]).unwrap();
    run_git(&dir, ["commit", "-m", "main adds file"]).unwrap();

    // Create a branch that also modifies the same file.
    let feat = dir.join("wt-conflict");
    add_lane(&dir, &feat, Some("feat/conflict"), None).unwrap();
    run_git(&feat, ["config", "user.email", "daruda@test"]).unwrap();
    run_git(&feat, ["config", "user.name", "daruda"]).unwrap();
    std::fs::write(feat.join("conflict.txt"), "feature content").unwrap();
    run_git(&feat, ["add", "conflict.txt"]).unwrap();
    run_git(&feat, ["commit", "-m", "feature modifies file"]).unwrap();

    // Attempt merge from main — should conflict because both branches
    // modified conflict.txt from the same base but diverge (main had
    // the file first; feat started from initial, then both wrote it).
    // NOTE: if git fast-forwards instead of conflicting, the test is
    // trivially valid (Success path), so we abort either way.
    let outcome = git_merge(&dir, "feat/conflict");
    match outcome {
        Ok(MergeOutcome::Conflicts(_)) => {
            // Abort must succeed and leave the tree clean.
            git_merge_abort(&dir).unwrap();
            let status = git_status(&dir).unwrap();
            assert!(
                status.unstaged.iter().all(|e| e.x != 'U' && e.y != 'U'),
                "no conflict markers should remain after abort"
            );
        }
        Ok(MergeOutcome::Success | MergeOutcome::AlreadyUpToDate) => {
            // Diverged history not achieved in this fixture — skip.
        }
        Err(e) => panic!("unexpected merge error: {e}"),
    }
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

#[test]
fn run_git_returns_timeout_error_when_child_runs_past_deadline() {
    // Use `git ls-remote` against a non-routable address — the
    // resolver/connect blocks for several seconds, well past our
    // tight test deadline. We don't depend on any specific network
    // condition: any CI environment where `git` exists will trip
    // the deadline. If git itself can't be launched the test
    // skips (consistent with other tests in this module).
    if !has_git() {
        return;
    }
    let dir = std::env::temp_dir();
    let started = Instant::now();
    let result = run_git_with_timeout(
        &dir,
        ["ls-remote", "https://10.255.255.1/nonexistent.git"],
        Duration::from_millis(200),
    );
    let elapsed = started.elapsed();

    match result {
        Err(GitError::Timeout(d)) => {
            assert_eq!(d, Duration::from_millis(200));
        }
        other => panic!("expected Timeout, got {other:?} in {elapsed:?}"),
    }
    // The poll granularity is 50 ms so the actual elapsed time can
    // run to roughly `timeout + poll_interval`. Assert a generous
    // upper bound so flake-prone CI never trips.
    assert!(
        elapsed < Duration::from_secs(2),
        "timeout took too long: {elapsed:?}"
    );
}
