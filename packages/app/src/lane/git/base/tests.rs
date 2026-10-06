use super::*;
use crate::lane::git::init;
use crate::lane::git::tests::{commit_initial, require_git, teardown, unique_tmpdir};

fn commit_file(repo: &Path, name: &str, body: &str, message: &str) {
    std::fs::write(repo.join(name), body).unwrap();
    run_git(repo, ["add", name]).unwrap();
    run_git(repo, ["commit", "-m", message]).unwrap();
}

/// A repo on `main` with one commit, plus a `feature` branch checked out.
fn repo_with_feature(prefix: &str) -> PathBuf {
    let dir = unique_tmpdir(prefix);
    init(&dir).unwrap();
    commit_initial(&dir);
    run_git(&dir, ["branch", "-M", "main"]).unwrap();
    commit_file(&dir, "base.txt", "one\n", "base");
    run_git(&dir, ["checkout", "-b", "feature"]).unwrap();
    dir
}

#[test]
fn name_status_parses_plain_rename_and_copy_records() {
    let raw = "M\0src/a.rs\0R087\0old.rs\0new.rs\0A\0b.rs\0C100\0x.rs\0y.rs\0";
    let files = parse_name_status(raw);
    let got: Vec<_> = files
        .iter()
        .map(|f| (f.status, f.path.clone(), f.old_path.clone()))
        .collect();
    assert_eq!(
        got,
        vec![
            ('M', PathBuf::from("src/a.rs"), None),
            ('R', PathBuf::from("new.rs"), Some(PathBuf::from("old.rs"))),
            ('A', PathBuf::from("b.rs"), None),
            ('C', PathBuf::from("y.rs"), Some(PathBuf::from("x.rs"))),
        ]
    );
    assert!(parse_name_status("").is_empty());
    // A rename cut off before its destination is dropped, not misread.
    assert!(parse_name_status("R100\0old.rs\0").is_empty());
}

#[test]
fn a_local_base_without_upstream_resolves_to_itself() {
    if !require_git() {
        return;
    }
    let dir = repo_with_feature("base_local");
    let base = resolve_base_ref(&dir, "main").unwrap();
    assert_eq!(base.label, "main");
    assert_eq!(base.sha, verify(&dir, "main").unwrap());
    teardown(&dir);
}

#[test]
fn a_local_base_with_upstream_reads_as_the_upstream() {
    if !require_git() {
        return;
    }
    let dir = repo_with_feature("base_upstream");
    // `origin/main` one commit behind local `main`: the upstream is what the
    // lane is compared against, not the local copy. The remote has to exist,
    // since `%(upstream)` maps the merge ref through its fetch refspec.
    run_git(
        &dir,
        ["remote", "add", "origin", "https://example.invalid/r.git"],
    )
    .unwrap();
    run_git(&dir, ["update-ref", "refs/remotes/origin/main", "main~1"]).unwrap();
    run_git(&dir, ["config", "branch.main.remote", "origin"]).unwrap();
    run_git(&dir, ["config", "branch.main.merge", "refs/heads/main"]).unwrap();
    let base = resolve_base_ref(&dir, "main").unwrap();
    assert_eq!(base.label, "origin/main");
    assert_eq!(base.sha, verify(&dir, "main~1").unwrap());
    teardown(&dir);
}

#[test]
fn a_name_no_local_branch_carries_falls_back_to_origin() {
    if !require_git() {
        return;
    }
    let dir = repo_with_feature("base_origin_fallback");
    run_git(&dir, ["update-ref", "refs/remotes/origin/trunk", "main"]).unwrap();
    assert_eq!(
        resolve_base_ref(&dir, "trunk").unwrap().label,
        "origin/trunk"
    );
    assert_eq!(
        resolve_base_ref(&dir, "nowhere"),
        Err(BaseProblem::NotFound("nowhere".to_owned()))
    );
    teardown(&dir);
}

#[test]
fn tips_refuse_a_missing_base_and_the_base_branch_itself() {
    if !require_git() {
        return;
    }
    let dir = repo_with_feature("base_tips");
    assert_eq!(
        base_tips(&dir, None, Some("feature")),
        Err(BaseProblem::NoBaseConfigured)
    );
    assert_eq!(
        base_tips(&dir, Some("main"), Some("main")),
        Err(BaseProblem::OnBaseBranch)
    );
    let tips = base_tips(&dir, Some("main"), Some("feature")).unwrap();
    assert_eq!(tips.head, verify(&dir, "HEAD").unwrap());
    teardown(&dir);
}

#[test]
fn changes_since_lists_only_what_the_lane_committed() {
    if !require_git() {
        return;
    }
    let dir = repo_with_feature("base_changes");
    commit_file(&dir, "base.txt", "one\ntwo\n", "edit");
    commit_file(&dir, "new.txt", "fresh\n", "add");
    run_git(&dir, ["mv", "new.txt", "moved.txt"]).unwrap();
    run_git(&dir, ["commit", "-m", "move"]).unwrap();
    // The base moves on after the lane left it; its commit must not show up.
    run_git(&dir, ["checkout", "main"]).unwrap();
    commit_file(&dir, "upstream.txt", "later\n", "upstream");
    run_git(&dir, ["checkout", "feature"]).unwrap();
    // Uncommitted work is the status list's, not this view's.
    std::fs::write(dir.join("base.txt"), "dirty\n").unwrap();

    let tips = base_tips(&dir, Some("main"), Some("feature")).unwrap();
    let changes = changes_since(&dir, tips).unwrap();
    assert_eq!(changes.commits, 3);
    let mut got: Vec<_> = changes
        .files
        .iter()
        .map(|f| {
            (
                f.path.to_string_lossy().into_owned(),
                f.status,
                f.added,
                f.removed,
            )
        })
        .collect();
    got.sort();
    assert_eq!(
        got,
        vec![
            ("base.txt".to_owned(), 'M', 1, 0),
            ("moved.txt".to_owned(), 'A', 1, 0),
        ]
    );

    let diff = git_diff_range(
        &dir,
        &changes.merge_base,
        &changes.tips.head,
        &[&dir.join("base.txt")],
    )
    .unwrap();
    assert!(diff.contains("+two"), "{diff}");
    assert!(
        !diff.contains("dirty"),
        "the working tree leaked in: {diff}"
    );
    let at_head = git_show_at(&dir, &changes.tips.head, Path::new("base.txt")).unwrap();
    assert_eq!(at_head, b"one\ntwo\n");
    teardown(&dir);
}

#[test]
fn unrelated_histories_have_no_merge_base() {
    if !require_git() {
        return;
    }
    let dir = repo_with_feature("base_unrelated");
    run_git(&dir, ["checkout", "--orphan", "island"]).unwrap();
    run_git(&dir, ["commit", "--allow-empty", "-m", "island"]).unwrap();
    let tips = base_tips(&dir, Some("main"), Some("island")).unwrap();
    assert_eq!(changes_since(&dir, tips), Err(BaseProblem::NoMergeBase));
    teardown(&dir);
}
