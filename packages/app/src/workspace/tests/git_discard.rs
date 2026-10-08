//! The Git panel's discard: one file, or everything, back to HEAD — staged
//! changes included, which the per-file action once refused.

use super::*;

fn git(dir: &std::path::Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?} failed in {dir:?}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// `staged.txt` modified and staged only, `edited.txt` modified in the
/// worktree, `junk.txt` untracked.
fn changed_repo(temp: &std::path::Path) -> std::path::PathBuf {
    let root = temp.join("work");
    std::fs::create_dir_all(&root).unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    git(&root, &["config", "user.email", "daruda@test"]);
    git(&root, &["config", "user.name", "daruda"]);
    git(&root, &["config", "core.autocrlf", "false"]);
    for name in ["staged.txt", "edited.txt"] {
        std::fs::write(root.join(name), b"base\n").unwrap();
    }
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-qm", "base"]);
    std::fs::write(root.join("staged.txt"), b"staged\n").unwrap();
    git(&root, &["add", "staged.txt"]);
    std::fs::write(root.join("edited.txt"), b"edited\n").unwrap();
    std::fs::write(root.join("junk.txt"), b"junk\n").unwrap();
    root
}

fn open(
    cx: &mut TestAppContext,
    root: &std::path::Path,
) -> (
    gpui::WindowHandle<gpui_component::Root>,
    gpui::Entity<Workspace>,
    daruda_store::project::LaneRef,
) {
    let project = daruda_store::project::Project::from_path(root);
    let (wh, ws) = build_workspace_with(cx, &daruda_config::Config::default(), Some(project));
    ws.update(cx, |ws, cx| ws.reconcile_bootstrapped_lanes(cx));
    cx.run_until_parked();
    let id = ws.read_with(cx, |ws, _| ws.active_ref());
    ws.update(cx, |ws, cx| ws.refresh_git_status(id, cx));
    cx.run_until_parked();
    (wh, ws, id)
}

fn porcelain(root: &std::path::Path) -> String {
    git(root, &["status", "--porcelain"])
}

#[gpui::test]
fn discarding_a_staged_only_file_puts_it_back_at_head(cx: &mut TestAppContext) {
    if !crate::lane::git::has_git() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = changed_repo(temp.path());
    let (_wh, ws, lane) = open(cx, &root);
    ws.update(cx, |ws, cx| {
        ws.discard_changes(lane, vec!["staged.txt".into()], cx)
    });
    cx.run_until_parked();
    assert_eq!(
        std::fs::read_to_string(root.join("staged.txt")).unwrap(),
        "base\n"
    );
    let left = porcelain(&root);
    assert!(!left.contains("staged.txt"), "{left}");
    assert!(
        left.contains("edited.txt"),
        "only the one file is touched: {left}"
    );
}

#[gpui::test]
fn discarding_everything_leaves_nothing_to_commit(cx: &mut TestAppContext) {
    if !crate::lane::git::has_git() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = changed_repo(temp.path());
    let (_wh, ws, lane) = open(cx, &root);
    let every = ws.read_with(cx, |ws, _| {
        let id = ws.active_ref();
        let status = ws.lane_git_worktree(id).unwrap();
        status
            .staged
            .iter()
            .chain(&status.unstaged)
            .map(|e| e.path.clone())
            .collect::<Vec<_>>()
    });
    ws.update(cx, |ws, cx| ws.discard_changes(lane, every, cx));
    cx.run_until_parked();
    assert_eq!(porcelain(&root), "");
    assert!(!root.join("junk.txt").exists());
}

#[gpui::test]
fn discard_all_asks_before_touching_anything(cx: &mut TestAppContext) {
    if !crate::lane::git::has_git() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = changed_repo(temp.path());
    let (wh, ws, lane) = open(cx, &root);
    let before = porcelain(&root);
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.on_discard_all(lane, window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        porcelain(&root),
        before,
        "nothing happens before the confirm"
    );
}
