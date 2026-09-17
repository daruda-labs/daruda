//! Remote-facing git ops in the Git Changes header — the ones whose only
//! visible product is the header's `↑N ↓M` tracking indicator.

use super::*;

fn run_git(dir: &std::path::Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .current_dir(dir)
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?} failed in {dir:?}");
}

/// A bare "remote" plus a clone of it that is one commit behind, with the
/// clone's remote-tracking ref still pointing at the old tip — exactly the
/// state a `git fetch` exists to resolve. Returns the clone's path.
fn build_behind_clone(temp: &std::path::Path) -> std::path::PathBuf {
    let origin = temp.join("origin.git");
    let seed = temp.join("seed");
    let clone = temp.join("work");
    std::fs::create_dir_all(&origin).unwrap();
    std::fs::create_dir_all(&seed).unwrap();
    run_git(temp, &["init", "--bare", "-b", "main", "origin.git"]);

    run_git(&seed, &["init", "-b", "main"]);
    run_git(&seed, &["config", "user.email", "daruda@test"]);
    run_git(&seed, &["config", "user.name", "daruda"]);
    std::fs::write(seed.join("f.txt"), b"one\n").unwrap();
    run_git(&seed, &["add", "f.txt"]);
    run_git(&seed, &["commit", "-m", "initial"]);
    run_git(
        &seed,
        &["remote", "add", "origin", origin.to_str().unwrap()],
    );
    run_git(&seed, &["push", "-u", "origin", "main"]);

    run_git(
        temp,
        &["clone", origin.to_str().unwrap(), clone.to_str().unwrap()],
    );

    // A commit the clone cannot know about until it fetches.
    std::fs::write(seed.join("f.txt"), b"two\n").unwrap();
    run_git(&seed, &["commit", "-am", "second"]);
    run_git(&seed, &["push", "origin", "main"]);

    clone
}

/// Fetch's entire visible product is the header's `↓N` — it changes no file
/// in the working tree, and the file watcher skips `.git` paths on purpose,
/// so nothing else re-reads `git status` on its behalf. If the op does not
/// refresh the cached status itself, a successful fetch leaves the panel
/// byte-identical and reads to the user as a dead button.
#[gpui::test]
fn fetch_refreshes_the_tracking_indicator(cx: &mut TestAppContext) {
    if !crate::lane::git::has_git() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = build_behind_clone(temp.path());

    init_gpui_component(cx);
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path(&root);
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project_for_test(
            &config,
            Some(project),
            fresh_test_data_dir(),
            window,
            cx,
        )
    });
    let ws = wh.root(cx).unwrap();
    ws.update(cx, |ws, cx| ws.reconcile_bootstrapped_lanes(cx));
    cx.run_until_parked();

    let id = ws.read_with(cx, |ws, _| ws.active_ref());
    ws.update(cx, |ws, cx| ws.refresh_git_status(id, cx));
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| {
        let status = ws.lane_git(id).expect("a git lane has a status");
        assert_eq!(status.upstream.as_deref(), Some("origin/main"));
        assert_eq!(
            status.behind, 0,
            "before the fetch the clone cannot see the new commit"
        );
    });

    ws.update(cx, |ws, cx| ws.on_fetch(cx));
    cx.run_until_parked();

    ws.read_with(cx, |ws, _| {
        assert_eq!(
            ws.lane_git(id).expect("a git lane has a status").behind,
            1,
            "a successful fetch must leave the header showing what it found"
        );
    });
}
