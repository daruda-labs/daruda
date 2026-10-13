use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

// Temp-dir helpers — each test gets a unique path so parallel
// runs don't collide. The directory is created on demand and
// torn down at the end of the test.
pub(super) fn unique_tmpdir(prefix: &str) -> PathBuf {
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

pub(super) fn teardown(path: &Path) {
    let _ = std::fs::remove_dir_all(path);
}

pub(super) fn commit_initial(repo: &Path) {
    // Ensure we have an initial commit so `lane add` can create
    // checkouts — with no commits `add` refuses.
    run_git(repo, ["config", "user.email", "daruda@test"]).unwrap();
    run_git(repo, ["config", "user.name", "daruda"]).unwrap();
    run_git(repo, ["commit", "--allow-empty", "-m", "initial"]).unwrap();
}

// Skip-if-no-git guard for the whole module.
pub(super) fn require_git() -> bool {
    has_git()
}

// ----------------------------------------------------------------
// parse_git_status_output unit tests
// ----------------------------------------------------------------

#[test]
fn status_output_parses_index_worktree_renames_and_conflicts() {
    let data = parse_git_status_output("M  file.rs\0");
    assert_eq!(data.staged.len(), 1, "should have one staged entry");
    assert_eq!(data.staged[0].x, 'M');
    assert_eq!(data.staged[0].path.to_str().unwrap(), "file.rs");
    assert!(data.staged[0].original_path.is_none());
    assert!(data.unstaged.is_empty());

    let data = parse_git_status_output(" M file.rs\0");
    assert!(data.staged.is_empty());
    assert_eq!(data.unstaged.len(), 1);
    assert_eq!(data.unstaged[0].y, 'M');
    assert_eq!(data.unstaged[0].path.to_str().unwrap(), "file.rs");

    let data = parse_git_status_output("?? new.rs\0");
    assert!(data.staged.is_empty());
    assert_eq!(data.unstaged.len(), 1);
    assert_eq!(data.unstaged[0].x, '?');
    assert_eq!(data.unstaged[0].path.to_str().unwrap(), "new.rs");

    // Renamed: the destination, then the source as a record of its own.
    let data = parse_git_status_output("R  new.rs\0old.rs\0");
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

    // MM = staged modified + unstaged modified.
    let data = parse_git_status_output("MM file.rs\0");
    assert_eq!(data.staged.len(), 1, "one staged entry for MM");
    assert_eq!(data.unstaged.len(), 1, "one unstaged entry for MM");

    // A rename reported in the worktree column carries its source record too;
    // missing it would read `old.rs` as the next entry.
    let data = parse_git_status_output(" R new.rs\0old.rs\0?? x.rs\0");
    assert_eq!(data.unstaged.len(), 2);
    assert_eq!(
        data.unstaged[0].original_path,
        Some(PathBuf::from("old.rs"))
    );
    assert_eq!(data.unstaged[1].path, PathBuf::from("x.rs"));

    let data = parse_git_status_output("UU file.rs\0");
    assert!(data.staged.is_empty(), "UU must not appear in staged");
    assert_eq!(data.unstaged.len(), 1, "UU must appear in unstaged");
}

// ----------------------------------------------------------------
// parse_numstat unit tests — `git diff HEAD --numstat` parsing
// ----------------------------------------------------------------

#[test]
fn numstat_parses_text_binary_and_empty_input() {
    let stats = parse_numstat("13\t3\tsrc/lib.rs\x002\t1\tCargo.toml\0");
    assert_eq!(stats.len(), 2);
    assert_eq!(stats[0], (13, 3, PathBuf::from("src/lib.rs")));
    assert_eq!(stats[1], (2, 1, PathBuf::from("Cargo.toml")));

    // git emits `-\t-\t<path>` for binary files. Caller decides
    // whether to skip rendering "+0 -0".
    let stats = parse_numstat("-\t-\tassets/icon.png\0");
    assert_eq!(stats, vec![(0, 0, PathBuf::from("assets/icon.png"))]);

    let stats = parse_numstat("42\t0\tnew.rs\0");
    assert_eq!(stats[0], (42, 0, PathBuf::from("new.rs")));

    let stats = parse_numstat("0\t8\told.rs\0");
    assert_eq!(stats[0], (0, 8, PathBuf::from("old.rs")));

    assert!(parse_numstat("").is_empty());

    // A rename leaves the path empty; source and destination follow.
    let stats = parse_numstat("0\t0\t\0old name.rs\0새 이름.rs\x001\t0\ta.rs\0");
    assert_eq!(
        stats,
        vec![
            (0, 0, PathBuf::from("새 이름.rs")),
            (1, 0, PathBuf::from("a.rs"))
        ]
    );
}

// ----------------------------------------------------------------
// Tracking parsing — `git for-each-ref %(upstream:track)` rows
// ----------------------------------------------------------------

/// One row per branch, NUL-separated fields, `*` on the checked-out one.
fn ref_row(head: bool, branch: &str, upstream: &str, track: &str) -> String {
    format!(
        "{}\0{branch}\0{upstream}\0{track}\n",
        if head { "*" } else { " " }
    )
}

#[test]
fn tracking_output_parses_upstream_divergence_and_special_states() {
    let data = parse_tracking_output(&ref_row(true, "main", "", ""));
    assert_eq!(data.branch.as_deref(), Some("main"));
    assert!(data.upstream.is_none());
    assert_eq!(data.ahead, 0);
    assert_eq!(data.behind, 0);

    let data = parse_tracking_output(&ref_row(true, "main", "origin/main", ""));
    assert_eq!(data.upstream.as_deref(), Some("origin/main"));
    assert_eq!((data.ahead, data.behind), (0, 0));

    let data = parse_tracking_output(&ref_row(true, "main", "origin/main", "[ahead 3]"));
    assert_eq!((data.ahead, data.behind), (3, 0));

    let data = parse_tracking_output(&ref_row(true, "main", "origin/main", "[behind 2]"));
    assert_eq!((data.ahead, data.behind), (0, 2));

    let data = parse_tracking_output(&ref_row(true, "main", "origin/main", "[ahead 1, behind 2]"));
    assert_eq!((data.ahead, data.behind), (1, 2));

    // Upstream was deleted on the remote; keep the upstream string visible.
    let data = parse_tracking_output(&ref_row(true, "main", "origin/main", "[gone]"));
    assert_eq!(data.upstream.as_deref(), Some("origin/main"));
    assert_eq!((data.ahead, data.behind), (0, 0));
}

#[test]
fn tracking_output_picks_the_checked_out_row_and_survives_having_none() {
    // A linked worktree reports its own branch as HEAD while the other
    // rows — including one with divergence — must not be read instead.
    let rows = ref_row(false, "main", "origin/main", "[behind 9]")
        + &ref_row(true, "side", "origin/side", "[ahead 1]");
    let data = parse_tracking_output(&rows);
    assert_eq!(data.branch.as_deref(), Some("side"));
    assert_eq!((data.ahead, data.behind), (1, 0));

    // Detached HEAD marks no row at all.
    let data = parse_tracking_output(&ref_row(false, "main", "origin/main", "[ahead 1]"));
    assert!(data.branch.is_none());
    assert!(data.upstream.is_none());
    assert_eq!((data.ahead, data.behind), (0, 0));

    // An unborn branch lists no refs whatsoever.
    assert_eq!(
        parse_tracking_output(""),
        crate::lane::git::GitTracking::default()
    );
}

// ----------------------------------------------------------------
// parse_worktree_list unit tests
// ----------------------------------------------------------------

#[test]
fn parse_worktree_list_covers_porcelain_shapes() {
    let wts =
        parse_worktree_list("worktree /repo\nHEAD abcd1234\nbranch refs/heads/main\n\n").unwrap();
    assert_eq!(wts.len(), 1);
    assert_eq!(wts[0].path, PathBuf::from("/repo"));
    assert_eq!(wts[0].branch.as_deref(), Some("main"));
    assert_eq!(wts[0].head.as_deref(), Some("abcd1234"));
    assert!(!wts[0].bare);

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

    let wts = parse_worktree_list("worktree /repo/wt-detached\nHEAD ccc\ndetached\n\n").unwrap();
    assert_eq!(wts.len(), 1);
    assert!(wts[0].branch.is_none());

    let wts = parse_worktree_list("worktree /repo-bare\nHEAD ddd\nbare\n\n").unwrap();
    assert_eq!(wts.len(), 1);
    assert!(wts[0].bare);

    let sample = "worktree /repo\nHEAD abcd\nbranch refs/heads/main\nlocked locked-reason\nprunable stuff\n\n";
    let wts = parse_worktree_list(sample).unwrap();
    assert_eq!(wts.len(), 1);
    assert_eq!(wts[0].path, PathBuf::from("/repo"));

    assert!(matches!(
        parse_worktree_list("HEAD abcd\n\n"),
        Err(GitError::Parse(_))
    ));
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
fn already_removed_true_for_exit_128_when_path_is_gone() {
    let dir = unique_tmpdir("already_removed_gone");
    teardown(&dir); // the directory itself must not exist for this case
    assert!(already_removed(Some(128), &dir));
}

#[test]
fn already_removed_false_when_path_still_exists() {
    // A genuinely dirty/still-present worktree must still surface as a
    // real error (e.g. "needs --force") — never silently swallowed just
    // because the exit code matches.
    let dir = unique_tmpdir("already_removed_present");
    assert!(!already_removed(Some(128), &dir));
    teardown(&dir);
}

#[test]
fn already_removed_false_for_a_different_exit_code_even_if_path_is_gone() {
    // Conservative: only the exit-128 "fatal" class is eligible at all —
    // an unrelated non-128 failure must not be swallowed just because the
    // path happens to be missing.
    let dir = unique_tmpdir("already_removed_wrong_code");
    teardown(&dir);
    assert!(!already_removed(Some(1), &dir));
}

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
            let status = git_worktree_status(&dir).unwrap();
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

/// The conflict list names each file whole — a space in the name used to cut
/// it at the first word, since the path was read out of git's prose.
#[test]
fn a_conflict_reports_a_spaced_file_name_whole() {
    if !require_git() {
        return;
    }
    let dir = unique_tmpdir("merge_spaced");
    init(&dir).unwrap();
    run_git(&dir, ["config", "user.email", "daruda@test"]).unwrap();
    run_git(&dir, ["config", "user.name", "daruda"]).unwrap();
    std::fs::write(dir.join("my notes.md"), "base\n").unwrap();
    run_git(&dir, ["add", "."]).unwrap();
    run_git(&dir, ["commit", "-m", "base"]).unwrap();
    run_git(&dir, ["checkout", "-q", "-b", "side"]).unwrap();
    std::fs::write(dir.join("my notes.md"), "side\n").unwrap();
    run_git(&dir, ["commit", "-qam", "side"]).unwrap();
    run_git(&dir, ["checkout", "-q", "-"]).unwrap();
    std::fs::write(dir.join("my notes.md"), "main\n").unwrap();
    run_git(&dir, ["commit", "-qam", "main"]).unwrap();

    let files = match git_merge(&dir, "side").unwrap() {
        MergeOutcome::Conflicts(files) => files,
        other => panic!("expected a conflict, got {other:?}"),
    };
    assert_eq!(files, vec!["my notes.md".to_string()]);
    git_merge_abort(&dir).unwrap();
    teardown(&dir);
}

#[test]
fn run_git_returns_timeout_error_when_child_runs_past_deadline() {
    // Drive a slow child through `git` itself without relying on
    // network routing. The shell alias sleeps well past the tight
    // deadline, so the only behaviour under test is our timeout path.
    if !has_git() {
        return;
    }
    let dir = std::env::temp_dir();
    let started = Instant::now();
    let result = run_git_with_timeout(
        &dir,
        ["-c", "alias.sleep=!sleep 1", "sleep"],
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

// ----------------------------------------------------------------
// git dirs — where a lane's state actually lives
// ----------------------------------------------------------------

#[test]
fn excessive_output_stops_the_running_git_tree_before_the_deadline() {
    if !has_git() {
        return;
    }
    let started = Instant::now();
    let result = run_git_with_timeout(
        &std::env::temp_dir(),
        [
            "-c",
            "alias.noisy=!while :; do printf '%01024d\\n' 0 >&2; done",
            "noisy",
        ],
        Duration::from_secs(15),
    );
    assert!(matches!(result, Err(GitError::Parse(message)) if message.contains("exceeded")));
    assert!(started.elapsed() < Duration::from_secs(8));
}

#[test]
fn a_tree_path_is_slash_separated() {
    let path: PathBuf = ["src", "a b", "c.rs"].iter().collect();
    assert_eq!(tree_path(&path), "src/a b/c.rs");
}
