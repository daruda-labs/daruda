//! The Git panel's commit and push buttons: whether they ask first, and what
//! an empty commit message commits as. Driven against a real repository.

use super::*;
use crate::workspace::{CommitChanges, PushChanges};

fn run_git(dir: &std::path::Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?} failed in {dir:?}");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A repository with one commit, whose `origin` is a bare repository it has
/// pushed to, and `staged` files modified and staged on top.
fn repo_with_staged(temp: &std::path::Path, staged: &[&str]) -> std::path::PathBuf {
    let origin = temp.join("origin.git");
    let root = temp.join("work");
    std::fs::create_dir_all(&root).unwrap();
    run_git(temp, &["init", "-q", "--bare", "-b", "main", "origin.git"]);
    run_git(&root, &["init", "-q", "-b", "main"]);
    run_git(&root, &["config", "user.email", "daruda@test"]);
    run_git(&root, &["config", "user.name", "daruda"]);
    for name in staged {
        std::fs::write(root.join(name), b"one\n").unwrap();
    }
    run_git(&root, &["add", "-A"]);
    run_git(&root, &["commit", "-qm", "initial"]);
    run_git(
        &root,
        &["remote", "add", "origin", origin.to_str().unwrap()],
    );
    run_git(&root, &["push", "-q", "-u", "origin", "main"]);
    for name in staged {
        std::fs::write(root.join(name), b"two\n").unwrap();
    }
    run_git(&root, &["add", "-A"]);
    root
}

fn workspace_on(
    cx: &mut TestAppContext,
    root: &std::path::Path,
    config: daruda_config::Config,
) -> (
    gpui::WindowHandle<gpui_component::Root>,
    gpui::Entity<Workspace>,
) {
    // Root-hosted like the app: the commit confirm is a Root dialog.
    let project = daruda_store::project::Project::from_path(root);
    let (wh, ws) = build_workspace_with(cx, &config, Some(project));
    ws.update(cx, |ws, cx| ws.reconcile_bootstrapped_lanes(cx));
    cx.run_until_parked();
    let id = ws.read_with(cx, |ws, _| ws.active_ref());
    ws.update(cx, |ws, cx| ws.refresh_git_status(id, cx));
    cx.run_until_parked();
    (wh, ws)
}

fn commit(
    wh: gpui::WindowHandle<gpui_component::Root>,
    ws: &gpui::Entity<Workspace>,
    cx: &mut TestAppContext,
    message: &str,
) {
    let message = message.to_string();
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.git
                .commit_input
                .update(cx, |panel, cx| panel.set_text(message.as_str(), window, cx));
            ws.on_commit_changes(&CommitChanges, window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
}

fn head_subject(root: &std::path::Path) -> String {
    run_git(root, &["log", "-1", "--format=%s"])
}

fn config_with(edit: impl FnOnce(&mut daruda_config::GitConfig)) -> daruda_config::Config {
    let mut config = daruda_config::Config::default();
    edit(&mut config.git);
    config
}

#[gpui::test]
fn a_commit_asks_first_by_default(cx: &mut TestAppContext) {
    if !crate::lane::git::has_git() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = repo_with_staged(temp.path(), &["a.txt"]);
    let (wh, ws) = workspace_on(cx, &root, daruda_config::Config::default());
    commit(wh, &ws, cx, "mine");
    assert_eq!(
        head_subject(&root),
        "initial",
        "nothing lands before the confirm"
    );
}

#[gpui::test]
fn a_commit_with_the_confirm_off_lands_at_once(cx: &mut TestAppContext) {
    if !crate::lane::git::has_git() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = repo_with_staged(temp.path(), &["a.txt"]);
    let (wh, ws) = workspace_on(cx, &root, config_with(|g| g.confirm_commit = false));
    commit(wh, &ws, cx, "mine");
    assert_eq!(head_subject(&root), "mine");
}

#[gpui::test]
fn an_empty_message_commits_as_an_update_of_the_one_file(cx: &mut TestAppContext) {
    if !crate::lane::git::has_git() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = repo_with_staged(temp.path(), &["a.txt"]);
    let (wh, ws) = workspace_on(cx, &root, config_with(|g| g.confirm_commit = false));
    commit(wh, &ws, cx, "  ");
    assert_eq!(head_subject(&root), "Update a.txt");
}

#[gpui::test]
fn an_empty_message_names_the_count_of_several_files(cx: &mut TestAppContext) {
    if !crate::lane::git::has_git() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = repo_with_staged(temp.path(), &["a.txt", "b.txt"]);
    let (wh, ws) = workspace_on(cx, &root, config_with(|g| g.confirm_commit = false));
    commit(wh, &ws, cx, "");
    assert_eq!(head_subject(&root), "Update 2 files");
}

#[gpui::test]
fn an_empty_message_is_refused_with_the_default_off(cx: &mut TestAppContext) {
    if !crate::lane::git::has_git() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = repo_with_staged(temp.path(), &["a.txt"]);
    let config = config_with(|g| {
        g.confirm_commit = false;
        g.default_commit_message = false;
    });
    let (wh, ws) = workspace_on(cx, &root, config);
    commit(wh, &ws, cx, "");
    assert_eq!(head_subject(&root), "initial");
}

#[gpui::test]
fn a_push_with_the_confirm_off_lands_at_once(cx: &mut TestAppContext) {
    if !crate::lane::git::has_git() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = repo_with_staged(temp.path(), &["a.txt"]);
    run_git(&root, &["commit", "-qm", "local"]);
    let config = config_with(|g| g.confirm_push = false);
    let (wh, ws) = workspace_on(cx, &root, config);
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.on_push_changes(&PushChanges, window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    let origin = temp.path().join("origin.git");
    assert_eq!(
        run_git(&origin, &["log", "-1", "--format=%s", "main"]),
        "local",
        "the push reached the remote"
    );
}
