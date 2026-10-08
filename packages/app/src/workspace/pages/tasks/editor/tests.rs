use crate::agent::tasks_global::GlobalTasks;
use crate::workspace::pages::tasks::editor::state::{BranchValidation, RunInChoice};
use crate::workspace::tests::build_workspace;
use daruda_store::tasks::{Task, TaskRunIn};
use gpui::{
    AppContext as _, BorrowAppContext as _, Focusable as _, TestAppContext, VisualTestContext,
};

#[gpui::test]
fn task_list_scope_preserves_worktree_and_seeds_new_task_project(cx: &mut TestAppContext) {
    use daruda_store::tasks::{TaskFilter, TaskScope};
    let (_a, window, ws) = build_workspace_with_project(cx);
    let b_root = tempfile::tempdir().unwrap();
    cx.update_window(window.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            let a = ws.projects[0].uuid;
            ws.add_project(b_root.path().to_path_buf(), window, cx);
            let active = ws.active;
            let b = ws.active_project().unwrap().uuid;
            ws.open_page(crate::workspace::pages::Page::Tasks, window, cx);
            ws.set_task_scope(TaskScope::Project(a), cx);
            ws.set_task_filter(TaskFilter::Failed, cx);
            ws.right_views
                .tasks
                .search
                .clone()
                .update(cx, |input, cx| input.set_value("login", window, cx));
            ws.clear_task_filters(window, cx);
            assert_eq!(ws.right_views.tasks.state.scope, TaskScope::Project(a));
            assert_eq!(ws.right_views.tasks.state.filter, TaskFilter::All);
            assert_eq!(ws.right_views.tasks.search.read(cx).value(), "");
            assert_eq!(ws.active, active);
            ws.new_task_in_scope(window, cx);
            let editor = ws
                .task_detail_id()
                .expect("the Tasks page holds the editor");
            let te = ws.task_editor(editor).unwrap();
            assert_eq!(te.project(cx), Some(a));
            assert_eq!(
                te.lane_value(cx),
                "",
                "the active project's lane cannot leak into another project"
            );
            let title = te.title_input.clone();
            title.update(cx, |input, cx| input.set_value("Scoped draft", window, cx));
            ws.commit_task_form(editor, cx).unwrap();
            let id = ws.task_editor(editor).unwrap().task_id.clone().unwrap();
            assert_eq!(cx.global::<GlobalTasks>().get(&id).unwrap().project, a);
            assert_eq!(ws.active, active);
            ws.set_task_scope(TaskScope::AllProjects, cx);
            ws.new_task_in_scope(window, cx);
            let editor = ws
                .task_detail_id()
                .expect("the Tasks page holds the editor");
            assert_eq!(ws.task_editor(editor).unwrap().project(cx), Some(b));
            let pane_count = ws.active_runtime().panes.len();
            ws.set_task_scope(
                TaskScope::Project(daruda_store::project::ProjectUuid::new()),
                cx,
            );
            ws.new_task_in_scope(window, cx);
            assert_eq!(
                ws.active_runtime().panes.len(),
                pane_count,
                "a closed scope must not create a task in the active project"
            );
        });
    })
    .unwrap();
}

#[gpui::test]
fn task_editor_watches_prompt_after_start_parks_its_lane(cx: &mut TestAppContext) {
    let source = tempfile::tempdir().unwrap();
    let (window, ws) = crate::workspace::tests::build_workspace_with(
        cx,
        &daruda_config::Config::default(),
        Some(daruda_store::project::Project::from_path(source.path())),
    );
    let other = tempfile::tempdir().unwrap();
    cx.update_window(window.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.open_task_editor(None, window, cx);
            let editor = ws
                .task_detail_id()
                .expect("the Tasks page holds the editor");
            let title = ws.task_editor(editor).unwrap().title_input.clone();
            title.update(cx, |s, cx| s.set_value("Keep watching", window, cx));
            let id = ws.commit_task_form(editor, cx).unwrap();
            let project = ws.active.project;
            let lane = ws.alloc_id();
            ws.project_for_mut(project).unwrap().lanes.push(
                crate::lane::Lane::default_for_project(lane, other.path().to_path_buf()),
            );
            ws.activate_lane(daruda_store::project::LaneRef { project, lane }, window, cx);
            cx.update_global::<GlobalTasks, _>(|g, _| {
                g.get_mut(&id).unwrap().state = daruda_store::tasks::TaskState::Running {
                    worktree_path: other.path().to_path_buf(),
                };
            });
            let task = cx.global::<GlobalTasks>().get(&id).unwrap().clone();
            let path = daruda_store::tasks::prompt_file::prompt_file_path(&task, other.path());
            daruda_store::tasks::prompt_file::write_prompt_file(&path, "External update").unwrap();
            ws.attach_prompt_watcher_if_editor_open(&id, window, cx);
            assert!(ws.task_editor(editor).unwrap()._prompt_watcher.is_some());
            ws.handle_prompt_file_changed(editor, path, window, cx);
            let te = ws.task_editor(editor).unwrap();
            assert_eq!(te.prompt_state.read(cx).value().as_ref(), "External update");
            assert!(!te.is_dirty(cx));
        });
    })
    .unwrap();
}

#[gpui::test]
fn task_editor_save_shortcut_and_tab_order(cx: &mut TestAppContext) {
    let (_project, window, ws) = build_workspace_with_project(cx);
    let mut vcx = VisualTestContext::from_window(window.into(), cx);
    vcx.cx.update(crate::bind_keys::register_static_bindings);
    vcx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            ws.open_task_editor(None, window, cx);
            let editor = ws
                .task_detail_id()
                .expect("the Tasks page holds the editor");
            ws.toggle_task_settings(editor, cx);
            let title = ws.task_editor(editor).unwrap().title_input.clone();
            title.update(cx, |s, cx| s.set_value("Keyboard save", window, cx));
            window.refresh();
        });
    });
    vcx.run_until_parked();
    ws.read_with(&vcx, |ws, cx| {
        let te = ws
            .task_editor(
                ws.task_detail_id()
                    .expect("the Tasks page holds the editor"),
            )
            .unwrap();
        let base = te.base_select.read(cx).focus_handle(cx);
        assert!(base.tab_stop);
        assert_eq!(base.tab_index, 4);
    });
    vcx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-s"
    } else {
        "ctrl-s"
    });
    vcx.run_until_parked();
    ws.read_with(&vcx, |ws, cx| {
        let tasks = &cx.global::<GlobalTasks>().0.tasks;
        assert_eq!(tasks.len(), 1, "shortcut saved the draft");
        assert_eq!(tasks[0].title, "Keyboard save");
        assert_eq!(ws.task_detail_id(), None, "a plain save closes the editor");
        assert_eq!(ws.active_page(), Some(crate::workspace::pages::Page::Tasks));
    });
}

#[gpui::test]
fn task_editor_rejects_blank_titles_and_invalid_branches(cx: &mut TestAppContext) {
    let (window, ws) = build_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.open_task_editor(None, window, cx);
            let id = ws
                .task_detail_id()
                .expect("the Tasks page holds the editor");
            assert!(ws.commit_task_form(id, cx).is_none());
            let te = ws.task_editor(id).unwrap();
            let title = te.title_input.clone();
            let branch = te.branch_input.clone();
            title.update(cx, |s, cx| s.set_value("   ", window, cx));
            assert!(ws.commit_task_form(id, cx).is_none());
            title.update(cx, |s, cx| s.set_value("A task", window, cx));
            branch.update(cx, |s, cx| s.set_value("invalid branch", window, cx));
            ws.on_task_edit_branch_typed(id, cx);
            assert!(!ws.task_editor(id).unwrap().can_save(cx));
            assert!(ws.commit_task_form(id, cx).is_none());
            assert!(cx.global::<GlobalTasks>().0.tasks.is_empty());
            ws.save_task_editor(id, false, window, cx);
            assert_eq!(
                ws.task_detail_id(),
                Some(id),
                "validation failure keeps the editor open"
            );
            assert_eq!(ws.active_page(), Some(crate::workspace::pages::Page::Tasks));
        });
    })
    .unwrap();
}

#[gpui::test]
fn task_editor_save_returns_to_list_and_keeps_saved_subtasks(cx: &mut TestAppContext) {
    let (_project, window, ws) = build_workspace_with_project(cx);
    cx.update_window(window.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.open_task_editor(None, window, cx);
            let editor = ws
                .task_detail_id()
                .expect("the Tasks page holds the editor");
            let te = ws.task_editor(editor).unwrap();
            let title = te.title_input.clone();
            let subtask = te.new_subtask_input.clone();
            title.update(cx, |s, cx| s.set_value("Ship editor", window, cx));
            ws.refresh_task_edit_title(editor, cx);
            subtask.update(cx, |s, cx| s.set_value("  Verify keyboard  ", window, cx));
            ws.submit_new_subtask(editor, window, cx);
            let sub = ws.task_editor(editor).unwrap().draft_subtasks[0].id.clone();
            ws.toggle_editor_subtask(editor, &sub, cx);
            ws.save_task_editor(editor, false, window, cx);
            assert!(
                ws.task_editor(editor).is_none(),
                "a saved editor leaves no tab behind"
            );
            let id = cx.global::<GlobalTasks>().0.tasks[0].id.clone();
            let task = cx.global::<GlobalTasks>().get(&id).unwrap();
            assert_eq!(task.subtasks[0].title, "Verify keyboard");
            assert!(task.subtasks[0].completed);
            assert_eq!(ws.active_page(), Some(crate::workspace::pages::Page::Tasks));
            ws.open_task_editor(Some(id.clone()), window, cx);
            let editor = ws
                .task_detail_id()
                .expect("the Tasks page holds the editor");
            assert!(!ws.task_editor(editor).unwrap().is_dirty(cx));
            ws.save_task_editor(editor, false, window, cx);
            assert_eq!(cx.global::<GlobalTasks>().0.tasks.len(), 1);
            assert_eq!(
                cx.global::<GlobalTasks>().get(&id).unwrap().subtasks.len(),
                1
            );
        });
    })
    .unwrap();
}

/// The editor is not a lane's tab, and a plain save returns to the list.
#[gpui::test]
fn task_editor_is_no_lane_tab_and_a_plain_save_shows_the_list(cx: &mut TestAppContext) {
    let (_project, window, ws) = build_workspace_with_project(cx);
    cx.update_window(window.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.open_task_editor(None, window, cx);
            let editor = ws
                .task_detail_id()
                .expect("the Tasks page holds the editor");
            assert!(
                ws.main_area
                    .runtimes
                    .values()
                    .all(|rt| rt.panes.iter().all(|p| p.id != editor.0)),
                "no lane's runtime holds the editor"
            );
            let title = ws.task_editor(editor).unwrap().title_input.clone();
            title.update(cx, |s, cx| s.set_value("No tab", window, cx));
            ws.save_task_editor(editor, false, window, cx);
            assert_eq!(ws.task_detail_id(), None, "a plain save shows the list");
            assert_eq!(ws.active_page(), Some(crate::workspace::pages::Page::Tasks));
            assert_eq!(cx.global::<GlobalTasks>().0.tasks[0].title, "No tab");
        });
    })
    .unwrap();
    cx.update_window(window.into(), |_, _, _| ())
        .expect("the window outlives the save");
}

/// Leaving the editor and choosing Save keeps what the editor's own Save
/// keeps, including a subtask typed but not yet submitted.
#[gpui::test]
fn leaving_with_save_keeps_an_unsubmitted_subtask(cx: &mut TestAppContext) {
    let (_project, window, ws) = build_workspace_with_project(cx);
    let editor = cx
        .update_window(window.into(), |_, window, cx| {
            ws.update(cx, |ws, cx| {
                ws.open_task_editor(None, window, cx);
                let editor = ws
                    .task_detail_id()
                    .expect("the Tasks page holds the editor");
                let te = ws.task_editor(editor).unwrap();
                let title = te.title_input.clone();
                let subtask = te.new_subtask_input.clone();
                title.update(cx, |s, cx| s.set_value("Closing", window, cx));
                subtask.update(cx, |s, cx| s.set_value("Typed only", window, cx));
                ws.leave_task_detail_then(window, cx, |_, _, _| {});
                editor
            })
        })
        .unwrap();
    cx.simulate_prompt_answer(&crate::surface::strings::task::edit_save_draft());
    cx.run_until_parked();
    ws.read_with(cx, |ws, cx| {
        assert!(ws.task_editor(editor).is_none(), "a landed save leaves");
        let task = &cx.global::<GlobalTasks>().0.tasks[0];
        assert_eq!(task.title, "Closing");
        assert_eq!(task.subtasks.len(), 1);
        assert_eq!(task.subtasks[0].title, "Typed only");
    });
}

#[gpui::test]
fn task_editor_save_reveals_task_without_switching_worktree(cx: &mut TestAppContext) {
    use crate::workspace::right_dock::tasks::{TaskGroupKey, TaskGrouping};
    use daruda_store::tasks::{TaskFilter, TaskScope};
    let (_project, window, ws) = build_workspace_with_project(cx);
    let other = tempfile::tempdir().unwrap();
    cx.update_window(window.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            let project = ws.active_project().unwrap().uuid;
            ws.add_project(other.path().to_owned(), window, cx);
            let active = ws.active;
            ws.open_task_draft_for_project(project, window, cx);
            let editor = ws
                .task_detail_id()
                .expect("the Tasks page holds the editor");
            let title = ws.task_editor(editor).unwrap().title_input.clone();
            title.update(cx, |s, cx| s.set_value("Saved task", window, cx));
            ws.set_task_scope(TaskScope::ActiveProject, cx);
            ws.set_task_filter(TaskFilter::Failed, cx);
            ws.right_views
                .tasks
                .search
                .clone()
                .update(cx, |s, cx| s.set_value("hidden", window, cx));
            ws.set_task_grouping(TaskGrouping::Project, cx);
            let key = TaskGroupKey::Project(project);
            ws.toggle_task_group(key, cx);
            ws.save_task_editor(editor, false, window, cx);
            assert_eq!(ws.active_page(), Some(crate::workspace::pages::Page::Tasks));
            assert_eq!(
                ws.right_views.tasks.state.scope,
                TaskScope::Project(project)
            );
            assert_eq!(ws.right_views.tasks.state.filter, TaskFilter::All);
            assert_eq!(ws.right_views.tasks.search.read(cx).value(), "");
            assert!(ws.right_views.tasks.state.groups.is_open(key));
            assert_eq!(ws.active, active);
            assert!(ws.task_editor(editor).is_none());

            ws.close_page(cx);
            let id = cx.global::<GlobalTasks>().0.tasks[0].id.clone();
            ws.open_task_editor(Some(id), window, cx);
            let editor = ws
                .task_detail_id()
                .expect("the Tasks page holds the editor");
            ws.set_task_scope(TaskScope::AllProjects, cx);
            ws.set_task_filter(TaskFilter::Backlog, cx);
            ws.right_views
                .tasks
                .search
                .clone()
                .update(cx, |s, cx| s.set_value("saved", window, cx));
            ws.save_task_editor(editor, false, window, cx);
            assert_eq!(ws.right_views.tasks.state.scope, TaskScope::AllProjects);
            assert_eq!(ws.right_views.tasks.state.filter, TaskFilter::Backlog);
            assert_eq!(ws.right_views.tasks.search.read(cx).value(), "saved");
            assert_eq!(ws.active_page(), Some(crate::workspace::pages::Page::Tasks));
        });
    })
    .unwrap();
}

/// Saving a renamed task persists the title and leaves its stored branch
/// alone; the draft form's live behavior is pinned below.
#[gpui::test]
fn task_editor_saving_a_rename_keeps_the_stored_branch(cx: &mut TestAppContext) {
    let (window, ws) = build_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            let task = Task::new(
                daruda_store::project::ProjectUuid::default(),
                "Original".into(),
                String::new(),
                None,
            );
            let branch = task.branch_name.clone();
            let id = task.id.clone();
            cx.update_global::<GlobalTasks, _>(|g, _| {
                g.add(task);
            });
            ws.open_task_editor(Some(id.clone()), window, cx);
            let editor = ws
                .task_detail_id()
                .expect("the Tasks page holds the editor");
            let title = ws.task_editor(editor).unwrap().title_input.clone();
            title.update(cx, |s, cx| s.set_value("Renamed", window, cx));
            ws.refresh_task_edit_title(editor, cx);
            ws.save_task_editor(editor, false, window, cx);
            let task = cx.global::<GlobalTasks>().get(&id).unwrap();
            assert_eq!(task.title, "Renamed");
            assert_eq!(task.branch_name, branch);
            assert!(ws.task_editor(editor).is_none());
        });
    })
    .unwrap();
}

/// While a draft is typed, the title never reshapes the branch — neither
/// the prefilled name nor one the user entered.
#[gpui::test]
fn task_editor_typing_a_title_never_touches_the_branch(cx: &mut TestAppContext) {
    let (window, ws) = build_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.open_task_editor(None, window, cx);
            let editor = ws
                .task_detail_id()
                .expect("the Tasks page holds the editor");
            let te = ws.task_editor(editor).unwrap();
            let (title, branch) = (te.title_input.clone(), te.branch_input.clone());
            let prefilled = branch.read(cx).value().to_string();
            title.update(cx, |s, cx| s.set_value("한글 제목 with spaces", window, cx));
            ws.refresh_task_edit_title(editor, cx);
            assert_eq!(branch.read(cx).value().as_ref(), prefilled);
            assert_eq!(
                ws.task_editor(editor).unwrap().cached_title.as_ref(),
                "한글 제목 with spaces"
            );
            branch.update(cx, |s, cx| s.set_value("custom", window, cx));
            ws.on_task_edit_branch_typed(editor, cx);
            title.update(cx, |s, cx| s.set_value("Other", window, cx));
            ws.refresh_task_edit_title(editor, cx);
            assert_eq!(branch.read(cx).value().as_ref(), "custom");
            ws.regenerate_task_branch(editor, window, cx);
            assert!(branch.read(cx).value().starts_with("task-"));
        });
    })
    .unwrap();
}

#[gpui::test]
fn task_editor_close_preserves_unsaved_draft_until_confirmed(cx: &mut TestAppContext) {
    let (window, ws) = build_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.open_task_editor(None, window, cx);
            let editor = ws
                .task_detail_id()
                .expect("the Tasks page holds the editor");
            let title = ws.task_editor(editor).unwrap().title_input.clone();
            title.update(cx, |s, cx| s.set_value("Keep this draft", window, cx));
            ws.leave_task_detail_then(window, cx, |_, _, _| {});
            assert!(
                ws.task_editor(editor).is_some(),
                "an unsaved draft is asked about"
            );
        });
    })
    .unwrap();
    assert!(cx.has_pending_prompt());
}

#[gpui::test]
fn task_editor_distinct_drafts_and_empty_branch_save(cx: &mut TestAppContext) {
    let (_project, window, ws) = build_workspace_with_project(cx);
    cx.update_window(window.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            let mut branches = Vec::new();
            for round in 0..2 {
                // One editor at a time: the first draft goes before the next.
                if round > 0 {
                    let open = ws.task_detail_id().expect("the first draft is open");
                    ws.close_task_detail(open, cx);
                }
                ws.open_task_editor(None, window, cx);
                let editor = ws
                    .task_detail_id()
                    .expect("the Tasks page holds the editor");
                let te = ws.task_editor(editor).unwrap();
                let title = te.title_input.clone();
                title.update(cx, |s, cx| s.set_value("Same title", window, cx));
                ws.refresh_task_edit_title(editor, cx);
                branches.push(
                    ws.task_editor(editor)
                        .unwrap()
                        .branch_input
                        .read(cx)
                        .value(),
                );
            }
            assert_ne!(branches[0], branches[1]);
            let editor = ws
                .task_detail_id()
                .expect("the Tasks page holds the editor");
            let branch = ws.task_editor(editor).unwrap().branch_input.clone();
            branch.update(cx, |s, cx| s.set_value("", window, cx));
            ws.on_task_edit_branch_typed(editor, cx);
            // Save & Start keeps the editor, so it must show the stored branch.
            ws.save_task_editor(editor, true, window, cx);
            let te = ws.task_editor(editor).unwrap();
            let task = cx
                .global::<GlobalTasks>()
                .get(te.task_id.as_ref().unwrap())
                .unwrap();
            assert!(!task.branch_name.is_empty(), "an empty branch is generated");
            assert_eq!(branch.read(cx).value().as_ref(), task.branch_name);
            assert!(!te.is_dirty(cx));
        });
    })
    .unwrap();
}

#[gpui::test]
fn task_editor_stores_the_branch_trimmed(cx: &mut TestAppContext) {
    let (_project, window, ws) = build_workspace_with_project(cx);
    cx.update_window(window.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.open_task_editor(None, window, cx);
            let editor = ws
                .task_detail_id()
                .expect("the Tasks page holds the editor");
            let te = ws.task_editor(editor).unwrap();
            let (title, branch) = (te.title_input.clone(), te.branch_input.clone());
            title.update(cx, |s, cx| s.set_value("Trim me", window, cx));
            branch.update(cx, |s, cx| s.set_value("  fix-trim  ", window, cx));
            ws.on_task_edit_branch_typed(editor, cx);
            let id = ws.commit_task_form(editor, cx).unwrap();
            assert_eq!(
                cx.global::<GlobalTasks>().get(&id).unwrap().branch_name,
                "fix-trim"
            );
        });
    })
    .unwrap();
}

/// A Backlog task that failed to start (e.g. on a branch that already
/// exists) is fixed in place rather than deleted and re-created.
#[gpui::test]
fn task_editor_edits_a_backlog_branch_but_not_a_started_one(cx: &mut TestAppContext) {
    let (window, ws) = build_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            let backlog = Task::new(
                daruda_store::project::ProjectUuid::default(),
                "Backlog".into(),
                String::new(),
                None,
            );
            let mut running = Task::new(
                daruda_store::project::ProjectUuid::default(),
                "Running".into(),
                String::new(),
                None,
            );
            running.state = daruda_store::tasks::TaskState::Running {
                worktree_path: std::env::temp_dir(),
            };
            let started_branch = running.branch_name.clone();
            let (backlog_id, running_id) = (backlog.id.clone(), running.id.clone());
            cx.update_global::<GlobalTasks, _>(|g, _| {
                g.add(backlog);
                g.add(running);
            });
            for id in [&backlog_id, &running_id] {
                ws.open_task_editor(Some(id.clone()), window, cx);
                let editor = ws
                    .task_detail_id()
                    .expect("the Tasks page holds the editor");
                let branch = ws.task_editor(editor).unwrap().branch_input.clone();
                branch.update(cx, |s, cx| s.set_value("fix-renamed", window, cx));
                ws.on_task_edit_branch_typed(editor, cx);
                ws.commit_task_form(editor, cx).unwrap();
            }
            let tasks = cx.global::<GlobalTasks>();
            assert_eq!(tasks.get(&backlog_id).unwrap().branch_name, "fix-renamed");
            assert_eq!(tasks.get(&running_id).unwrap().branch_name, started_branch);
        });
    })
    .unwrap();
}

/// A workspace with one project, since lanes belong to a project.
fn build_workspace_with_project(
    cx: &mut TestAppContext,
) -> (
    tempfile::TempDir,
    gpui::WindowHandle<crate::ui::Root>,
    gpui::Entity<crate::workspace::Workspace>,
) {
    let root = tempfile::tempdir().unwrap();
    let project = daruda_store::project::Project::from_path(root.path());
    let (window, ws) = crate::workspace::tests::build_workspace_with(
        cx,
        &daruda_config::Config::default(),
        Some(project),
    );
    (root, window, ws)
}

/// Registers a git lane on `branch` in the active project and returns its path.
fn push_git_lane(ws: &mut crate::workspace::Workspace, branch: &str) -> std::path::PathBuf {
    let project = ws.active.project;
    let id = ws.alloc_id();
    let path = std::env::temp_dir().join(format!("daruda-lane-{branch}"));
    ws.project_for_mut(project)
        .unwrap()
        .lanes
        .push(crate::lane::Lane::git(
            id,
            path.clone(),
            Some(branch.to_string()),
            path.clone(),
            path.clone(),
            0,
        ));
    path
}

#[gpui::test]
fn task_editor_blocks_a_new_worktree_on_a_branch_a_lane_uses(cx: &mut TestAppContext) {
    let (_root, window, ws) = build_workspace_with_project(cx);
    cx.update_window(window.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            push_git_lane(ws, "main");
            ws.open_task_editor(None, window, cx);
            let editor = ws
                .task_detail_id()
                .expect("the Tasks page holds the editor");
            let te = ws.task_editor(editor).unwrap();
            let (title, branch) = (te.title_input.clone(), te.branch_input.clone());
            title.update(cx, |s, cx| s.set_value("On main", window, cx));
            branch.update(cx, |s, cx| s.set_value("main", window, cx));
            ws.on_task_edit_branch_typed(editor, cx);
            let te = ws.task_editor(editor).unwrap();
            assert_eq!(te.branch_validation, BranchValidation::Exists);
            assert!(!te.can_save(cx));
            assert!(ws.commit_task_form(editor, cx).is_none());
        });
    })
    .unwrap();
}

/// The check re-runs at save: a lane made after the last keystroke counts.
#[gpui::test]
fn task_editor_save_catches_a_lane_created_after_typing(cx: &mut TestAppContext) {
    let (_root, window, ws) = build_workspace_with_project(cx);
    cx.update_window(window.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.open_task_editor(None, window, cx);
            let editor = ws
                .task_detail_id()
                .expect("the Tasks page holds the editor");
            let te = ws.task_editor(editor).unwrap();
            let (title, branch) = (te.title_input.clone(), te.branch_input.clone());
            title.update(cx, |s, cx| s.set_value("Late", window, cx));
            branch.update(cx, |s, cx| s.set_value("feat-late", window, cx));
            ws.on_task_edit_branch_typed(editor, cx);
            push_git_lane(ws, "feat-late");
            assert!(ws.commit_task_form(editor, cx).is_none());
            assert_eq!(
                ws.task_editor(editor).unwrap().branch_validation,
                BranchValidation::Exists
            );
        });
    })
    .unwrap();
}

#[gpui::test]
fn task_editor_saves_an_existing_lane_without_a_base(cx: &mut TestAppContext) {
    let (_root, window, ws) = build_workspace_with_project(cx);
    cx.update_window(window.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            let lane = ws.active_lane().unwrap().path.clone();
            ws.open_task_editor(None, window, cx);
            let editor = ws
                .task_detail_id()
                .expect("the Tasks page holds the editor");
            let title = ws.task_editor(editor).unwrap().title_input.clone();
            title.update(cx, |s, cx| s.set_value("In place", window, cx));
            ws.set_task_run_in(editor, RunInChoice::ExistingLane, cx);
            let id = ws.commit_task_form(editor, cx).unwrap();
            let task = cx.global::<GlobalTasks>().get(&id).unwrap();
            assert_eq!(task.run_in, TaskRunIn::ExistingLane { path: lane });
            assert_eq!(task.base_worktree_path, None);
            assert!(!ws.task_editor(editor).unwrap().is_dirty(cx));
        });
    })
    .unwrap();
}

/// A started task's branch is its own lane's; that must not block edits.
#[gpui::test]
fn task_editor_saves_a_running_task_whose_lane_holds_its_branch(cx: &mut TestAppContext) {
    let (_root, window, ws) = build_workspace_with_project(cx);
    cx.update_window(window.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            let mut task = Task::new(
                daruda_store::project::ProjectUuid::default(),
                "Running".into(),
                String::new(),
                None,
            );
            let lane = push_git_lane(ws, &task.branch_name);
            task.state = daruda_store::tasks::TaskState::Running {
                worktree_path: lane,
            };
            let id = task.id.clone();
            cx.update_global::<GlobalTasks, _>(|g, _| {
                g.add(task);
            });
            ws.open_task_editor(Some(id.clone()), window, cx);
            let editor = ws
                .task_detail_id()
                .expect("the Tasks page holds the editor");
            let title = ws.task_editor(editor).unwrap().title_input.clone();
            title.update(cx, |s, cx| s.set_value("Renamed", window, cx));
            assert!(ws.task_editor(editor).unwrap().can_save(cx));
            ws.commit_task_form(editor, cx).unwrap();
            assert_eq!(
                cx.global::<GlobalTasks>().get(&id).unwrap().title,
                "Renamed"
            );
        });
    })
    .unwrap();
}

#[gpui::test]
fn task_editor_location_change_is_dirty_until_saved(cx: &mut TestAppContext) {
    let (_root, window, ws) = build_workspace_with_project(cx);
    cx.update_window(window.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            let task = Task::new(
                daruda_store::project::ProjectUuid::default(),
                "Move me".into(),
                String::new(),
                None,
            );
            let id = task.id.clone();
            cx.update_global::<GlobalTasks, _>(|g, _| {
                g.add(task);
            });
            ws.open_task_editor(Some(id.clone()), window, cx);
            let editor = ws
                .task_detail_id()
                .expect("the Tasks page holds the editor");
            assert!(!ws.task_editor(editor).unwrap().is_dirty(cx));
            ws.set_task_run_in(editor, RunInChoice::ExistingLane, cx);
            assert!(ws.task_editor(editor).unwrap().is_dirty(cx));
            ws.commit_task_form(editor, cx).unwrap();
            assert!(!ws.task_editor(editor).unwrap().is_dirty(cx));
        });
    })
    .unwrap();
}

/// A reopened task resumes in the lane holding its branch, so the branch
/// check that guards new worktrees must not lock its form.
#[gpui::test]
fn task_editor_saves_a_reopened_task(cx: &mut TestAppContext) {
    let (_root, window, ws) = build_workspace_with_project(cx);
    cx.update_window(window.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            let mut task = Task::new(
                daruda_store::project::ProjectUuid::default(),
                "Reopened".into(),
                String::new(),
                None,
            );
            let lane = push_git_lane(ws, &task.branch_name);
            std::fs::create_dir_all(&lane).unwrap();
            task.state = daruda_store::tasks::TaskState::Done {
                worktree_path: lane,
                end_reason: daruda_store::tasks::SessionEndReason::Stop,
            };
            let id = task.id.clone();
            cx.update_global::<GlobalTasks, _>(|g, _| {
                g.add(task);
            });
            ws.reopen_task(&id, cx);
            ws.open_task_editor(Some(id.clone()), window, cx);
            let editor = ws
                .task_detail_id()
                .expect("the Tasks page holds the editor");
            let title = ws.task_editor(editor).unwrap().title_input.clone();
            title.update(cx, |s, cx| s.set_value("Edited", window, cx));
            assert!(ws.task_editor(editor).unwrap().can_save(cx));
            ws.commit_task_form(editor, cx).unwrap();
            assert_eq!(cx.global::<GlobalTasks>().get(&id).unwrap().title, "Edited");
        });
    })
    .unwrap();
}

/// A new task opens with a branch already filled in, and regenerating
/// works before anything is typed — no edit is needed to unlock it.
#[gpui::test]
fn task_editor_draft_prefills_a_branch_and_regenerates_it(cx: &mut TestAppContext) {
    let (window, ws) = build_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.open_task_editor(None, window, cx);
            let editor = ws
                .task_detail_id()
                .expect("the Tasks page holds the editor");
            let te = ws.task_editor(editor).unwrap();
            let branch = te.branch_input.clone();
            let first = branch.read(cx).value().to_string();
            assert!(first.starts_with("task-"), "prefilled: {first}");
            assert_eq!(te.branch_validation, BranchValidation::Valid);
            assert!(!te.is_dirty(cx), "the prefill is not an edit");

            ws.regenerate_task_branch(editor, window, cx);
            let second = branch.read(cx).value().to_string();
            assert!(second.starts_with("task-"));
            assert_ne!(first, second, "a new random suffix");
        });
    })
    .unwrap();
}

/// A prompt that differs from the saved one only by CRLF line endings —
/// what an external editor may write back — is not an edit.
#[gpui::test]
fn task_editor_crlf_only_prompt_change_is_not_dirty(cx: &mut TestAppContext) {
    let (window, ws) = build_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            let task = Task::new(
                daruda_store::project::ProjectUuid::default(),
                "Lines".into(),
                "one\ntwo".into(),
                None,
            );
            let id = task.id.clone();
            cx.update_global::<GlobalTasks, _>(|g, _| {
                g.add(task);
            });
            ws.open_task_editor(Some(id), window, cx);
            let editor = ws
                .task_detail_id()
                .expect("the Tasks page holds the editor");
            let prompt = ws.task_editor(editor).unwrap().prompt_state.clone();
            assert!(!ws.task_editor(editor).unwrap().is_dirty(cx));
            prompt.update(cx, |s, cx| s.set_value("one\r\ntwo", window, cx));
            assert!(!ws.task_editor(editor).unwrap().is_dirty(cx));
            prompt.update(cx, |s, cx| s.set_value("one\r\nthree", window, cx));
            assert!(ws.task_editor(editor).unwrap().is_dirty(cx));
        });
    })
    .unwrap();
}

/// A base lane that is no longer registered cannot be selected; the form
/// must not read as edited for it, neither when opened nor after a save.
#[gpui::test]
fn task_editor_unregistered_base_is_clean_at_open_and_after_save(cx: &mut TestAppContext) {
    let (_root, window, ws) = build_workspace_with_project(cx);
    cx.update_window(window.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            let task = Task::new(
                daruda_store::project::ProjectUuid::default(),
                "Old base".into(),
                String::new(),
                Some(std::path::PathBuf::from("/nonexistent/daruda-base")),
            );
            let id = task.id.clone();
            cx.update_global::<GlobalTasks, _>(|g, _| {
                g.add(task);
            });
            ws.open_task_editor(Some(id), window, cx);
            let editor = ws
                .task_detail_id()
                .expect("the Tasks page holds the editor");
            assert!(!ws.task_editor(editor).unwrap().is_dirty(cx));
            let title = ws.task_editor(editor).unwrap().title_input.clone();
            title.update(cx, |s, cx| s.set_value("Renamed", window, cx));
            assert!(ws.task_editor(editor).unwrap().is_dirty(cx));
            ws.commit_task_form(editor, cx).unwrap();
            assert!(!ws.task_editor(editor).unwrap().is_dirty(cx));
        });
    })
    .unwrap();
}

/// Another project's lanes replace the base and run-in choices, and the
/// task moves to that project on save.
#[gpui::test]
fn task_editor_project_change_refills_lane_pickers_and_saves(cx: &mut TestAppContext) {
    let (_a, window, ws) = build_workspace_with_project(cx);
    let b_root = tempfile::tempdir().unwrap();
    cx.update_window(window.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            let a = ws.projects[0].uuid;
            let a_lane = ws.projects[0].lanes[0].path.clone();
            ws.add_project(b_root.path().to_path_buf(), window, cx);
            let b = ws.projects[1].uuid;
            let mut task = Task::new(a, "Move".into(), String::new(), Some(a_lane.clone()));
            task.run_in = TaskRunIn::ExistingLane { path: a_lane };
            let id = task.id.clone();
            cx.update_global::<GlobalTasks, _>(|g, _| {
                g.add(task);
            });
            ws.open_task_editor(Some(id.clone()), window, cx);
            let editor = ws
                .task_detail_id()
                .expect("the Tasks page holds the editor");
            let te = ws.task_editor(editor).unwrap();
            assert_eq!(te.project(cx), Some(a));
            assert!(!te.lane_value(cx).is_empty(), "A's lane is preselected");
            let project_select = te.project_select.clone();
            project_select.update(cx, |s, cx| {
                s.set_selected_value(
                    &crate::workspace::pages::tasks::editor::state::project_value(b),
                    window,
                    cx,
                )
            });
            ws.on_task_edit_project_changed(editor, window, cx);
            let te = ws.task_editor(editor).unwrap();
            assert_eq!(te.lane_value(cx), "", "A's lane is not B's to pick");
            assert_eq!(
                te.base_select
                    .read(cx)
                    .selected_value()
                    .map(|v| v.to_string()),
                Some(String::new()),
                "the base falls back to B's own"
            );
            ws.set_task_run_in(editor, RunInChoice::NewWorktree, cx);
            ws.commit_task_form(editor, cx).unwrap();
            assert_eq!(cx.global::<GlobalTasks>().get(&id).unwrap().project, b);
        });
    })
    .unwrap();
}

/// A started task's project names the repository its lane is in, so the
/// form cannot move it.
#[gpui::test]
fn task_editor_keeps_a_started_tasks_project(cx: &mut TestAppContext) {
    let (_a, window, ws) = build_workspace_with_project(cx);
    let b_root = tempfile::tempdir().unwrap();
    cx.update_window(window.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            let a = ws.projects[0].uuid;
            ws.add_project(b_root.path().to_path_buf(), window, cx);
            let b = ws.projects[1].uuid;
            let mut task = Task::new(a, "Running".into(), String::new(), None);
            task.state = daruda_store::tasks::TaskState::Running {
                worktree_path: std::path::PathBuf::from("/tmp/elsewhere"),
            };
            let id = task.id.clone();
            cx.update_global::<GlobalTasks, _>(|g, _| {
                g.add(task);
            });
            ws.open_task_editor(Some(id.clone()), window, cx);
            let editor = ws
                .task_detail_id()
                .expect("the Tasks page holds the editor");
            let project_select = ws.task_editor(editor).unwrap().project_select.clone();
            project_select.update(cx, |s, cx| {
                s.set_selected_value(
                    &crate::workspace::pages::tasks::editor::state::project_value(b),
                    window,
                    cx,
                )
            });
            ws.commit_task_form(editor, cx).unwrap();
            assert_eq!(cx.global::<GlobalTasks>().get(&id).unwrap().project, a);
        });
    })
    .unwrap();
}

/// Escape leaves a clean editor for the list, and asks about a dirty one.
#[gpui::test]
fn escape_backs_out_of_the_editor(cx: &mut TestAppContext) {
    let (_project, window, ws) = build_workspace_with_project(cx);
    let mut vcx = VisualTestContext::from_window(window.into(), cx);
    vcx.cx.update(crate::bind_keys::register_static_bindings);
    vcx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            ws.open_task_editor(None, window, cx);
            window.refresh();
        });
    });
    vcx.run_until_parked();
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    ws.read_with(&vcx, |ws, _| {
        assert_eq!(ws.task_detail_id(), None, "a clean editor leaves at once");
        assert_eq!(ws.active_page(), Some(crate::workspace::pages::Page::Tasks));
    });

    vcx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            ws.open_task_editor(None, window, cx);
            let editor = ws
                .task_detail_id()
                .expect("the Tasks page holds the editor");
            let title = ws.task_editor(editor).unwrap().title_input.clone();
            title.update(cx, |s, cx| s.set_value("Half typed", window, cx));
            window.refresh();
        });
    });
    vcx.run_until_parked();
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    assert!(
        vcx.has_pending_prompt(),
        "a dirty editor asks before leaving"
    );
    ws.read_with(&vcx, |ws, _| assert!(ws.task_detail_id().is_some()));
}

/// Cmd+W closes the editor the page shows, not the page or a lane's tab.
#[gpui::test]
fn close_tab_leaves_the_editor_for_the_list(cx: &mut TestAppContext) {
    let (_project, window, ws) = build_workspace_with_project(cx);
    cx.update_window(window.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            let tabs = ws.total_open_tabs();
            ws.open_task_editor(None, window, cx);
            ws.on_close_tab(&crate::workspace::CloseTab, window, cx);
            assert_eq!(ws.task_detail_id(), None);
            assert_eq!(ws.active_page(), Some(crate::workspace::pages::Page::Tasks));
            assert_eq!(ws.total_open_tabs(), tabs, "no lane tab closed");
        });
    })
    .unwrap();
}

/// The conflict prompt's `[Diff]` shows the disk version beside the prompt,
/// read-only; closing it leaves the prompt as it was.
#[gpui::test]
fn the_disk_version_shows_read_only_beside_the_prompt(cx: &mut TestAppContext) {
    let (_project, window, ws) = build_workspace_with_project(cx);
    cx.update_window(window.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.open_task_editor(None, window, cx);
            let editor = ws
                .task_detail_id()
                .expect("the Tasks page holds the editor");
            let prompt = ws.task_editor(editor).unwrap().prompt_state.clone();
            prompt.update(cx, |s, cx| s.set_value("mine", window, cx));
            ws.show_prompt_disk_copy(editor, "on disk", window, cx);
            let copy = ws
                .task_editor(editor)
                .unwrap()
                .disk_copy
                .clone()
                .expect("shown");
            assert_eq!(copy.read(cx).value().as_ref(), "on disk");
            assert!(copy.read(cx).is_disabled(), "the disk version is read-only");
            ws.close_prompt_disk_copy(editor, cx);
            let te = ws.task_editor(editor).unwrap();
            assert!(te.disk_copy.is_none());
            assert_eq!(te.prompt_state.read(cx).value().as_ref(), "mine");
        });
    })
    .unwrap();
}

/// Restores `ws`'s snapshot into a fresh window and reports what the Tasks
/// page's editor came back holding: `None` for the list, else its task id.
fn restored_detail(
    ws: &gpui::Entity<crate::workspace::Workspace>,
    cx: &mut TestAppContext,
) -> Option<Option<daruda_store::tasks::TaskId>> {
    let (state, projects) = ws.read_with(cx, |ws, cx| ws.snapshot_for_disk(cx));
    let restored = cx.add_window(|window, cx| {
        let mut ws = crate::workspace::Workspace::new_with_project_for_test(
            &daruda_config::Config::default(),
            None,
            crate::workspace::tests::fresh_test_data_dir(),
            window,
            cx,
        );
        ws.restore_from_disk(&state, &projects, window, cx);
        ws
    });
    restored
        .read_with(cx, |ws, _| {
            let editor = ws.task_detail_id()?;
            Some(ws.task_editor(editor).unwrap().task_id.clone())
        })
        .unwrap()
}

/// The open editor survives a restart as a fresh form on the same task; a
/// task deleted meanwhile leaves the list.
#[gpui::test]
fn the_open_task_reopens_after_a_restart(cx: &mut TestAppContext) {
    let (_project, window, ws) = build_workspace_with_project(cx);
    let task_id = cx
        .update_window(window.into(), |_, window, cx| {
            ws.update(cx, |ws, cx| {
                let project = ws.active_project().unwrap().uuid;
                let task = Task::new(project, "Kept".into(), String::new(), None);
                let id = task.id.clone();
                cx.update_global::<GlobalTasks, _>(|g, _| {
                    g.add(task);
                });
                ws.open_task_editor(Some(id.clone()), window, cx);
                id
            })
        })
        .unwrap();
    assert_eq!(restored_detail(&ws, cx), Some(Some(task_id.clone())));

    cx.update(|cx| cx.update_global::<GlobalTasks, _>(|g, _| g.remove(&task_id)));
    assert_eq!(
        restored_detail(&ws, cx),
        None,
        "a deleted task leaves the list"
    );
}

/// A new task's draft reopens empty in the project its form named.
#[gpui::test]
fn a_new_draft_reopens_empty_after_a_restart(cx: &mut TestAppContext) {
    let (_project, window, ws) = build_workspace_with_project(cx);
    cx.update_window(window.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.open_task_editor(None, window, cx);
            let editor = ws.task_detail_id().unwrap();
            let title = ws.task_editor(editor).unwrap().title_input.clone();
            title.update(cx, |s, cx| s.set_value("not kept", window, cx));
        });
    })
    .unwrap();
    assert_eq!(restored_detail(&ws, cx), Some(None));
}
