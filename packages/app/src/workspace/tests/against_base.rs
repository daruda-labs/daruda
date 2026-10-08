//! The against-base axis: what a lane committed since its base, read off the
//! tracking refresh.

use super::*;
use crate::lane::git::base::BaseProblem;
use crate::workspace::main_area::file_view_pane::DiffSource;

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
            .as_deref()
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
            ws.lane_scoped[&target].git.against_base.as_deref(),
            Some(&Err(BaseProblem::OnBaseBranch))
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
            ws.lane_scoped[&target].git.against_base.as_deref(),
            Some(&Err(BaseProblem::NoBaseConfigured))
        );
    });
}

#[gpui::test]
fn an_against_base_row_opens_a_diff_pinned_to_its_commits(cx: &mut TestAppContext) {
    if !crate::lane::git::has_git() {
        return;
    }
    use crate::workspace::main_area::file_view_pane::PaneFileContent;
    use crate::workspace::main_area::tab_ops::OpenIntent;

    let temp = tempfile::tempdir().unwrap();
    let root = feature_repo(temp.path());
    let (wh, ws) = build_workspace_with(
        cx,
        &daruda_config::Config::default(),
        Some(daruda_store::project::Project::from_path(&root)),
    );
    ws.update(cx, |ws, cx| ws.reconcile_bootstrapped_lanes(cx));
    cx.run_until_parked();
    let target = ws.update(cx, |ws, cx| {
        ws.projects[0].default_branch = Some("main".to_owned());
        let target = ws.active;
        ws.refresh_git_status(target, cx);
        target
    });
    cx.run_until_parked();

    let open = |cx: &mut TestAppContext| {
        cx.update_window(wh.into(), |_, window, cx| {
            ws.update(cx, |ws, cx| {
                ws.open_against_base_file(
                    target,
                    std::path::PathBuf::from("feature.txt"),
                    OpenIntent::Commit,
                    window,
                    cx,
                );
            });
        })
        .unwrap();
        cx.run_until_parked();
    };
    open(cx);
    let tabs = ws.read_with(cx, |ws, _| {
        let found = ws.lane_scoped[&target]
            .git
            .against_base
            .as_deref()
            .and_then(|r| r.as_ref().ok())
            .cloned()
            .expect("the lane has a base");
        let fc = ws
            .focused_file_content()
            .expect("the range pane is focused");
        assert_eq!(
            fc.view.source,
            DiffSource::Range {
                from: found.merge_base.clone(),
                to: found.tips.head.clone(),
                old_path: None,
                status: 'A',
            }
        );
        assert_eq!(fc.view.status(), Some('A'), "status comes from the range");
        match &fc.view.content {
            PaneFileContent::LoadedDiff { added, removed, .. } => {
                assert_eq!((*added, *removed), (1, 0));
            }
            PaneFileContent::Error(e) => panic!("load errored: {e}"),
            _ => panic!("expected a diff"),
        }
        ws.active_runtime().tabs.len()
    });

    // Opening the same row again finds the pane rather than stacking a tab.
    open(cx);
    ws.read_with(cx, |ws, _| assert_eq!(ws.active_runtime().tabs.len(), tabs));

    // The lane moves on and the file leaves the range; the open pane still
    // shows its pinned diff, so it must keep offering it.
    run_git(&root, &["rm", "-q", "feature.txt"]);
    run_git(&root, &["commit", "-qm", "drop"]);
    ws.update(cx, |ws, cx| ws.refresh_git_status(target, cx));
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| {
        let fc = ws
            .focused_file_content()
            .expect("the range pane is still open");
        assert_eq!(fc.view.status(), Some('A'));
    });
}

#[test]
fn a_persisted_range_restores_ahead_of_the_staged_flag_and_round_trips() {
    use daruda_store::project::{
        SerializedDiffRange, SerializedFileContent, SerializedFileViewMode,
    };
    let mut fc = SerializedFileContent {
        lane_id: 0,
        path: std::path::PathBuf::from("/r/a.rs"),
        staged: true,
        range: None,
        view_mode: SerializedFileViewMode::Changes,
        reference: false,
    };
    assert_eq!(DiffSource::from_serialized(&fc), DiffSource::Index);
    fc.staged = false;
    assert_eq!(DiffSource::from_serialized(&fc), DiffSource::WorkingTree);
    fc.range = Some(SerializedDiffRange {
        from: "m".into(),
        to: "h".into(),
        old_path: None,
        status: 'D',
    });
    fc.staged = true;
    assert_eq!(
        DiffSource::from_serialized(&fc),
        DiffSource::Range {
            from: "m".into(),
            to: "h".into(),
            old_path: None,
            status: 'D',
        }
    );
    // And writing it back yields the record it came from.
    let (staged, range) = DiffSource::from_serialized(&fc).to_serialized();
    assert!(!staged, "a range pane is not staged");
    assert_eq!(range, fc.range);
}

/// A git read that outlives its lane — torn down while the read ran — must
/// drop its result, not bring the lane's state back into the map.
#[gpui::test]
fn a_git_read_that_outlives_its_lane_does_not_revive_its_state(cx: &mut TestAppContext) {
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
    ws.update(cx, |ws, cx| ws.reconcile_bootstrapped_lanes(cx));
    cx.run_until_parked();
    let target = ws.update(cx, |ws, cx| {
        let target = ws.active;
        ws.refresh_git_status(target, cx);
        // What lane teardown does, while both reads are still in flight.
        ws.lane_scoped.remove(&target);
        target
    });
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| {
        assert!(
            !ws.lane_scoped.contains_key(&target),
            "a late git read recreated the removed lane's state"
        );
    });
}
