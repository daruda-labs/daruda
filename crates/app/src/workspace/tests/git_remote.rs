//! Remote-facing git ops in the Git Changes header — the ones whose only
//! visible product is the header's `↑N ↓M` tracking indicator.

use super::*;

const WATCH_REFRESH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(8);

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

fn lane_refs(ws: &Workspace) -> Vec<daruda_store::project::LaneRef> {
    let project = &ws.projects[0];
    project
        .lanes
        .iter()
        .map(|lane| daruda_store::project::LaneRef {
            project: project.id,
            lane: lane.id,
        })
        .collect()
}

/// Drive gpui until `predicate` observes a refresh from the real notify
/// watcher. The debounce thread uses wall time, so the loop yields to both
/// the OS watcher and the async delivery task.
fn wait_for_watch_refresh(
    cx: &mut TestAppContext,
    ws: &gpui::Entity<Workspace>,
    predicate: impl Fn(&Workspace) -> bool,
) {
    let deadline = std::time::Instant::now() + WATCH_REFRESH_TIMEOUT;
    loop {
        ws.update(cx, |ws, cx| ws.drain_git_watchers(cx));
        cx.run_until_parked();
        if ws.read_with(cx, |ws, _| predicate(ws)) {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "git-dir watcher did not refresh the workspace before the deadline"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
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
        let status = ws.lane_scoped[&id]
            .git
            .tracking
            .as_ref()
            .expect("a git lane has tracking info");
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
            ws.lane_scoped[&id]
                .git
                .tracking
                .as_ref()
                .expect("a git lane has tracking info")
                .behind,
            1,
            "a successful fetch must leave the header showing what it found"
        );
    });
}

/// A commit made outside daruda changes only git metadata. The dedicated
/// per-worktree watcher must invalidate the linked lane's working-tree cache;
/// the ordinary file-tree watcher cannot see this transition.
#[gpui::test]
fn external_commit_refreshes_linked_lane_through_the_git_dir_watcher(cx: &mut TestAppContext) {
    if !crate::lane::git::has_git() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("work");
    std::fs::create_dir_all(&root).unwrap();
    run_git(&root, &["init", "-q", "-b", "main"]);
    run_git(&root, &["config", "user.email", "daruda@test"]);
    run_git(&root, &["config", "user.name", "daruda"]);
    std::fs::write(root.join("base.txt"), b"base\n").unwrap();
    run_git(&root, &["add", "base.txt"]);
    run_git(&root, &["commit", "-qm", "initial"]);

    let linked = temp.path().join("side-wt");
    run_git(
        &root,
        &[
            "worktree",
            "add",
            "-q",
            linked.to_str().unwrap(),
            "-b",
            "side",
        ],
    );
    std::fs::write(linked.join("side.txt"), b"staged\n").unwrap();
    run_git(&linked, &["add", "side.txt"]);

    let (_wh, ws) = build_workspace_with(
        cx,
        &daruda_config::Config::default(),
        Some(daruda_store::project::Project::from_path(&root)),
    );
    ws.update(cx, |ws, cx| ws.reconcile_bootstrapped_lanes(cx));
    cx.run_until_parked();

    let linked = std::fs::canonicalize(&linked).unwrap();
    let target = ws.read_with(cx, |ws, _| {
        let project = &ws.projects[0];
        let lane = project
            .lanes
            .iter()
            .find(|lane| lane.path == linked)
            .expect("linked worktree must be a workspace lane");
        daruda_store::project::LaneRef {
            project: project.id,
            lane: lane.id,
        }
    });
    ws.update(cx, |ws, cx| ws.refresh_git_status(target, cx));
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| {
        assert_eq!(
            ws.lane_git_worktree(target)
                .expect("linked lane status")
                .staged
                .len(),
            1
        );
        assert_eq!(
            ws.git_watchers.len(),
            2,
            "the shared dir and linked lane dir must both be watched"
        );
    });

    run_git(&linked, &["commit", "-qm", "outside daruda"]);
    wait_for_watch_refresh(cx, &ws, |ws| {
        ws.lane_git_worktree(target)
            .is_some_and(|status| status.staged.is_empty())
    });
}

/// Fetch writes one shared remote-tracking ref. One common-dir watcher must
/// fan that event out so every lane recomputes its own divergence.
#[gpui::test]
fn external_fetch_refreshes_tracking_for_every_lane(cx: &mut TestAppContext) {
    if !crate::lane::git::has_git() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = build_behind_clone(temp.path());
    let linked = temp.path().join("side-wt");
    run_git(
        &root,
        &[
            "worktree",
            "add",
            "-q",
            linked.to_str().unwrap(),
            "-b",
            "side",
        ],
    );
    run_git(&root, &["branch", "--set-upstream-to=origin/main", "side"]);

    let (_wh, ws) = build_workspace_with(
        cx,
        &daruda_config::Config::default(),
        Some(daruda_store::project::Project::from_path(&root)),
    );
    ws.update(cx, |ws, cx| ws.reconcile_bootstrapped_lanes(cx));
    cx.run_until_parked();
    let targets = ws.read_with(cx, |ws, _| lane_refs(ws));
    ws.update(cx, |ws, cx| {
        for target in &targets {
            ws.refresh_git_status(*target, cx);
        }
    });
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| {
        assert_eq!(targets.len(), 2, "fixture must expose both lanes");
        assert_eq!(
            ws.git_watchers.len(),
            2,
            "one common-dir watcher plus the linked lane's own watcher"
        );
        for target in &targets {
            assert_eq!(
                ws.lane_scoped[target]
                    .git
                    .tracking
                    .as_ref()
                    .expect("initial tracking")
                    .behind,
                0,
                "the clone has not fetched the remote commit yet"
            );
        }
    });

    run_git(&root, &["fetch", "origin"]);
    wait_for_watch_refresh(cx, &ws, |ws| {
        targets.iter().all(|target| {
            ws.lane_scoped[target]
                .git
                .tracking
                .as_ref()
                .is_some_and(|tracking| tracking.behind == 1)
        })
    });
}
