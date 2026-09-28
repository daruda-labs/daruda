//! The against-base axis: what a lane committed since its base, read off the
//! tracking refresh.

use super::*;
use crate::lane::git::base::BaseProblem;

fn run_git(dir: &std::path::Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .current_dir(dir)
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?} failed in {dir:?}");
}

/// A repo whose `feature` branch committed one file on top of `main`.
fn feature_repo(temp: &std::path::Path) -> std::path::PathBuf {
    let root = temp.join("work");
    std::fs::create_dir_all(&root).unwrap();
    run_git(&root, &["init", "-q", "-b", "main"]);
    run_git(&root, &["config", "user.email", "daruda@test"]);
    run_git(&root, &["config", "user.name", "daruda"]);
    std::fs::write(root.join("base.txt"), b"base\n").unwrap();
    run_git(&root, &["add", "base.txt"]);
    run_git(&root, &["commit", "-qm", "initial"]);
    run_git(&root, &["checkout", "-qb", "feature"]);
    std::fs::write(root.join("feature.txt"), b"new\n").unwrap();
    run_git(&root, &["add", "feature.txt"]);
    run_git(&root, &["commit", "-qm", "feature"]);
    root
}

#[gpui::test]
fn a_tracking_refresh_reads_what_the_lane_committed_since_its_base(cx: &mut TestAppContext) {
    if !crate::lane::git::has_git() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = feature_repo(temp.path());
    let (_wh, ws) = build_workspace_with(
        cx,
        &daruda_config::Config::default(),
        Some(daruda_store::project::Project::from_path(&root)),
    );
    // The construction placeholder becomes a git lane here.
    ws.update(cx, |ws, cx| ws.reconcile_bootstrapped_lanes(cx));
    cx.run_until_parked();
    let target = ws.update(cx, |ws, cx| {
        ws.projects[0].default_branch = Some("main".to_owned());
        let target = ws.active;
        ws.refresh_git_status(target, cx);
        target
    });
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| {
        let state = &ws.lane_scoped[&target].git;
        let found = state
            .against_base
            .as_ref()
            .expect("the tracking refresh reads the axis")
            .as_ref()
            .expect("the lane has a base");
        assert_eq!(found.tips.base.label, "main");
        assert_eq!(found.commits, 1);
        let paths: Vec<_> = found.files.iter().map(|f| f.path.clone()).collect();
        assert_eq!(paths, vec![std::path::PathBuf::from("feature.txt")]);
    });

    // On its own base branch a lane has nothing to compare.
    run_git(&root, &["checkout", "-q", "main"]);
    ws.update(cx, |ws, cx| ws.refresh_git_status(target, cx));
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| {
        assert_eq!(
            ws.lane_scoped[&target].git.against_base,
            Some(Err(BaseProblem::OnBaseBranch))
        );
    });
}

#[gpui::test]
fn a_lane_with_no_base_anywhere_says_so(cx: &mut TestAppContext) {
    if !crate::lane::git::has_git() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = feature_repo(temp.path());
    let (_wh, ws) = build_workspace_with(
        cx,
        &daruda_config::Config::default(),
        Some(daruda_store::project::Project::from_path(&root)),
    );
    // The construction placeholder becomes a git lane here.
    ws.update(cx, |ws, cx| ws.reconcile_bootstrapped_lanes(cx));
    cx.run_until_parked();
    let target = ws.update(cx, |ws, cx| {
        // Reconcile detected `main`; a repo with no detectable default is
        // what this case is about.
        ws.projects[0].default_branch = None;
        let target = ws.active;
        ws.refresh_git_status(target, cx);
        target
    });
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| {
        assert_eq!(
            ws.lane_scoped[&target].git.against_base,
            Some(Err(BaseProblem::NoBaseConfigured))
        );
    });
}
