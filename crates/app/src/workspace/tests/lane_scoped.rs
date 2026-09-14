use daruda_store::project::LaneRef;
use gpui::{AppContext, TestAppContext};

use crate::files::tree::FileTree;
use crate::lane::Lane;
use crate::lane::git::GitStatusData;
use crate::workspace::Workspace;
use crate::workspace::tests::build_workspace_with;

fn seed_git_state(ws: &mut Workspace, target: LaneRef) {
    let git = &mut ws.lane_scoped_mut(target).git;
    git.status = Some(GitStatusData {
        branch: Some("retained-branch".into()),
        ..Default::default()
    });
    git.fetch_in_flight = true;
    git.fetch_pending_repeat = true;
    git.collapsed_dirs.insert("src".into());
    git.cursor = Some("src/main.rs".into());
}

fn seed_files_state(ws: &mut Workspace, target: LaneRef) {
    let root = ws.lane_for(target).unwrap().path.clone();
    std::fs::create_dir_all(&root).unwrap();
    let files = &mut ws.lane_scoped_mut(target).files;
    files.tree = Some(FileTree::new(root.clone()));
    files.watcher = Some(crate::files::watcher::FileTreeWatcher::new(root.clone()).unwrap());
    files.reload_queue = Some(Default::default());
    files.visible_cache = Some(std::sync::Arc::new(Vec::new()));
    files.gitignore = Some(crate::files::gitignore::GitignoreSet::build(&root));
}

fn assert_files_state_retained(ws: &Workspace, target: LaneRef) {
    let files = &ws.lane_scoped[&target].files;
    assert_eq!(
        files.tree.as_ref().unwrap().root,
        ws.lane_for(target).unwrap().path
    );
    assert!(files.watcher.is_some());
    assert!(files.reload_queue.is_some());
    assert!(files.gitignore.is_some());
}

fn seed_history_state(ws: &mut Workspace, target: LaneRef) {
    let history = &mut ws.lane_scoped_mut(target).input_history;
    history.push("older command");
    history.push("newer command");
    assert_eq!(history.prev("unfinished draft"), Some("newer command"));
}

fn assert_history_state_retained(ws: &mut Workspace, target: LaneRef) {
    let history = &mut ws.lane_scoped.get_mut(&target).unwrap().input_history;
    assert!(history.has_entries());
    assert!(history.is_navigating());
    assert_eq!(history.forward(), Some("unfinished draft"));
    assert_eq!(history.prev("another draft"), Some("newer command"));
    assert_eq!(history.prev("newer command"), Some("older command"));
}

fn assert_git_state_retained(ws: &Workspace, target: LaneRef) {
    let git = &ws.lane_scoped[&target].git;
    assert_eq!(
        ws.lane_git(target).unwrap().branch.as_deref(),
        Some("retained-branch")
    );
    assert!(git.fetch_in_flight);
    assert!(git.fetch_pending_repeat);
    assert!(git.collapsed_dirs.contains("src"));
    assert_eq!(
        git.cursor.as_deref(),
        Some(std::path::Path::new("src/main.rs"))
    );
}

fn add_removable_lane(ws: &mut Workspace) -> LaneRef {
    let project = ws.active_project_mut().unwrap();
    let repo_root = project.lanes[0].path.clone();
    let checkout = repo_root.join("feature");
    let lane = project.lanes.iter().map(|lane| lane.id).max().unwrap() + 1;
    project.lanes.push(Lane::git(
        lane,
        checkout.clone(),
        Some("feature".into()),
        repo_root,
        checkout,
        1,
    ));
    LaneRef {
        project: project.id,
        lane,
    }
}

#[gpui::test]
fn removing_a_lane_drops_its_scoped_state_and_preserves_its_sibling(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let (wh, ws) = build_workspace_with(
        cx,
        &daruda_config::Config::default(),
        Some(daruda_store::project::Project::from_path(root.path())),
    );
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            let survivor = ws.active;
            let removed = add_removable_lane(ws);
            assert!(ws.validate_remove_lane(removed).is_ok());
            seed_git_state(ws, survivor);
            seed_git_state(ws, removed);
            seed_files_state(ws, survivor);
            seed_files_state(ws, removed);
            seed_history_state(ws, survivor);
            seed_history_state(ws, removed);
            assert!(ws.lane_scoped.contains_key(&removed));

            ws.finalize_remove_lane(removed, window, cx);

            assert!(ws.lane_for(removed).is_none());
            assert!(
                !ws.lane_scoped.contains_key(&removed),
                "removed lane must drop its scoped state"
            );
            assert_git_state_retained(ws, survivor);
            assert_files_state_retained(ws, survivor);
            assert_history_state_retained(ws, survivor);
        });
    })
    .unwrap();
}

#[gpui::test]
fn closing_a_project_drops_all_its_scoped_state_and_preserves_other_projects(
    cx: &mut TestAppContext,
) {
    let root = tempfile::tempdir().unwrap();
    let other_root = tempfile::tempdir().unwrap();
    let (wh, ws) = build_workspace_with(
        cx,
        &daruda_config::Config::default(),
        Some(daruda_store::project::Project::from_path(root.path())),
    );
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            let survivor = ws.active;
            let removed = ws
                .add_project(other_root.path().to_path_buf(), window, cx)
                .unwrap();
            let sibling = add_removable_lane(ws);
            assert_eq!(survivor.lane, removed.lane);
            assert_ne!(survivor.project, removed.project);
            for target in [survivor, removed, sibling] {
                seed_git_state(ws, target);
                seed_files_state(ws, target);
                seed_history_state(ws, target);
                assert!(ws.lane_scoped.contains_key(&target));
            }

            assert!(ws.close_active_project(window, cx));

            assert_eq!(ws.active, survivor);
            assert!(
                !ws.lane_scoped
                    .keys()
                    .any(|target| target.project == removed.project),
                "closed project must drop every lane's scoped state"
            );
            assert_git_state_retained(ws, survivor);
            assert_files_state_retained(ws, survivor);
            assert_history_state_retained(ws, survivor);
        });
    })
    .unwrap();
}

#[gpui::test]
fn files_filter_invalidation_only_visits_lanes_with_trees(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let (_wh, ws) = build_workspace_with(
        cx,
        &daruda_config::Config::default(),
        Some(daruda_store::project::Project::from_path(root.path())),
    );
    ws.update(cx, |ws, cx| {
        let loaded = ws.active;
        let unloaded = add_removable_lane(ws);
        ws.lane_scoped_mut(loaded).files.tree = Some(FileTree::new(root.path().to_path_buf()));
        seed_git_state(ws, unloaded);
        let loaded_cache = ws.cached_or_rebuild_visible(loaded);
        let unloaded_cache = ws.cached_or_rebuild_visible(unloaded);
        assert!(ws.lane_file_tree(unloaded).is_none());

        ws.toggle_files_show_hidden(cx);

        assert!(!std::sync::Arc::ptr_eq(
            &loaded_cache,
            &ws.cached_or_rebuild_visible(loaded)
        ));
        assert!(
            std::sync::Arc::ptr_eq(&unloaded_cache, &ws.cached_or_rebuild_visible(unloaded)),
            "a lane without a file tree must remain outside file filter invalidation"
        );
    });
}
