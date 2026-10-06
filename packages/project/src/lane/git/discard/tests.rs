use super::*;

fn entry(x: char, y: char, path: &str, original: Option<&str>) -> GitFileEntry {
    GitFileEntry {
        x,
        y,
        path: PathBuf::from(path),
        original_path: original.map(PathBuf::from),
    }
}

fn head(paths: &[&str]) -> HashSet<PathBuf> {
    paths.iter().map(PathBuf::from).collect()
}

fn plan_of(entries: &[GitFileEntry], in_head: &[&str]) -> DiscardPlan {
    DiscardPlan::classify(entries.iter(), &head(in_head))
}

#[test]
fn a_file_head_has_is_restored_whatever_its_columns_say() {
    // `AA` / `AU` are merge conflicts over a file HEAD has: undoing them
    // restores it, never deletes it.
    for (x, y) in [
        (' ', 'M'),
        ('M', 'M'),
        ('D', ' '),
        ('A', 'A'),
        ('A', 'U'),
        ('U', 'U'),
    ] {
        let plan = plan_of(&[entry(x, y, "f", None)], &["f"]);
        assert_eq!(plan.restore, vec![PathBuf::from("f")], "{x}{y}");
        assert!(plan.remove.is_empty(), "{x}{y}");
    }
}

#[test]
fn a_file_head_lacks_is_removed() {
    for (x, y) in [('A', ' '), ('A', 'M'), ('U', 'A'), ('D', 'D')] {
        let plan = plan_of(&[entry(x, y, "f", None)], &[]);
        assert_eq!(plan.remove, vec![PathBuf::from("f")], "{x}{y}");
    }
}

#[test]
fn a_rename_on_either_side_brings_its_original_back() {
    for (x, y) in [('R', ' '), (' ', 'R')] {
        let plan = plan_of(&[entry(x, y, "to", Some("from"))], &["from"]);
        assert_eq!(plan.restore, vec![PathBuf::from("from")], "{x}{y}");
        assert_eq!(plan.remove, vec![PathBuf::from("to")], "{x}{y}");
    }
}

#[test]
fn an_untracked_file_is_deleted() {
    let plan = plan_of(&[entry('?', '?', "junk", None)], &[]);
    assert_eq!(plan.untracked, vec![PathBuf::from("junk")]);
}

#[test]
fn a_path_listed_staged_and_unstaged_is_planned_once() {
    let plan = plan_of(
        &[entry('M', 'M', "b", None), entry('M', 'M', "b", None)],
        &["b"],
    );
    assert_eq!(plan.restore, vec![PathBuf::from("b")]);
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn repo() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    git(root, &["init", "-q", "-b", "main"]);
    git(root, &["config", "user.email", "daruda@test"]);
    git(root, &["config", "user.name", "daruda"]);
    temp
}

fn all(root: &Path) -> Vec<PathBuf> {
    let status = crate::lane::git::git_worktree_status(root).unwrap();
    status
        .staged
        .iter()
        .chain(&status.unstaged)
        .map(|e| e.path.clone())
        .collect()
}

/// Every shape of change at once, then one discard of all of them.
#[test]
fn discarding_every_change_leaves_the_tree_at_head() {
    if !crate::lane::git::has_git() {
        return;
    }
    let temp = repo();
    let root = temp.path();
    for name in ["m", "s", "b", "d", "from"] {
        std::fs::write(root.join(name), format!("{name}\n")).unwrap();
    }
    git(root, &["add", "-A"]);
    git(root, &["commit", "-qm", "base"]);
    std::fs::write(root.join("m"), "worktree edit\n").unwrap();
    std::fs::write(root.join("s"), "staged edit\n").unwrap();
    git(root, &["add", "s"]);
    std::fs::write(root.join("b"), "staged\n").unwrap();
    git(root, &["add", "b"]);
    std::fs::write(root.join("b"), "and again\n").unwrap();
    git(root, &["rm", "-q", "d"]);
    std::fs::write(root.join("new"), "added\n").unwrap();
    git(root, &["add", "new"]);
    git(root, &["mv", "from", "to"]);
    std::fs::write(root.join("junk"), "untracked\n").unwrap();

    discard(root, &all(root)).unwrap();

    assert_eq!(
        git(root, &["status", "--porcelain"]),
        "",
        "nothing left to discard"
    );
    for name in ["m", "s", "b", "d", "from"] {
        assert_eq!(
            std::fs::read_to_string(root.join(name)).unwrap(),
            format!("{name}\n"),
            "{name} is back at HEAD"
        );
    }
    for gone in ["new", "to", "junk"] {
        assert!(!root.join(gone).exists(), "{gone} is removed");
    }
}

/// A name with glob characters is that one file, not a pattern.
#[test]
fn a_glob_named_file_discards_only_itself() {
    if !crate::lane::git::has_git() {
        return;
    }
    let temp = repo();
    let root = temp.path();
    for name in ["[id].tsx", "i.tsx", "star*", "starfoo"] {
        std::fs::write(root.join(name), "base\n").unwrap();
    }
    git(root, &["add", "-A"]);
    git(root, &["commit", "-qm", "base"]);
    for name in ["[id].tsx", "i.tsx", "star*", "starfoo"] {
        std::fs::write(root.join(name), "edited\n").unwrap();
    }
    discard(root, &[PathBuf::from("[id].tsx"), PathBuf::from("star*")]).unwrap();
    for (name, want) in [
        ("[id].tsx", "base\n"),
        ("star*", "base\n"),
        ("i.tsx", "edited\n"),
        ("starfoo", "edited\n"),
    ] {
        assert_eq!(
            std::fs::read_to_string(root.join(name)).unwrap(),
            want,
            "{name}"
        );
    }
}

/// Both sides added the file; undoing the conflict keeps HEAD's copy.
#[test]
fn discarding_an_add_add_conflict_keeps_heads_copy() {
    if !crate::lane::git::has_git() {
        return;
    }
    let temp = repo();
    let root = temp.path();
    std::fs::write(root.join("keep"), "base\n").unwrap();
    git(root, &["add", "keep"]);
    git(root, &["commit", "-qm", "base"]);
    git(root, &["checkout", "-qb", "side"]);
    std::fs::write(root.join("both"), "side\n").unwrap();
    git(root, &["add", "both"]);
    git(root, &["commit", "-qm", "side"]);
    git(root, &["checkout", "-q", "main"]);
    std::fs::write(root.join("both"), "main\n").unwrap();
    git(root, &["add", "both"]);
    git(root, &["commit", "-qm", "main"]);
    let _ = std::process::Command::new("git")
        .current_dir(root)
        .args(["merge", "-q", "side"])
        .output();
    assert!(git(root, &["status", "--porcelain"]).starts_with("AA both"));

    discard(root, &[PathBuf::from("both")]).unwrap();
    assert_eq!(
        std::fs::read_to_string(root.join("both")).unwrap(),
        "main\n"
    );
    assert!(!git(root, &["status", "--porcelain"]).contains("both"));
}

/// Only what the user was shown goes: a file that appears after the list
/// was read is left alone, and a listed file since committed is not undone.
#[test]
fn only_paths_still_changed_from_the_pinned_list_are_touched() {
    if !crate::lane::git::has_git() {
        return;
    }
    let temp = repo();
    let root = temp.path();
    std::fs::write(root.join("a"), "base\n").unwrap();
    git(root, &["add", "a"]);
    git(root, &["commit", "-qm", "base"]);
    std::fs::write(root.join("added"), "new\n").unwrap();
    git(root, &["add", "added"]);
    let pinned = all(root);
    // After the list was read: the addition is committed, and a new file
    // appears that nobody saw.
    git(root, &["commit", "-qm", "agent"]);
    std::fs::write(root.join("fresh"), "unseen\n").unwrap();

    discard(root, &pinned).unwrap();
    assert!(
        root.join("added").exists(),
        "a committed file is not undone"
    );
    assert!(root.join("fresh").exists(), "an unseen file is not deleted");
}

#[test]
fn an_empty_pin_runs_no_git() {
    discard(Path::new("/definitely/not/a/repo"), &[]).unwrap();
}
