use crate::agent::tasks_global::GlobalTasks;
use crate::workspace::main_area::task_edit_pane::state::{BranchValidation, RunInChoice};
use crate::workspace::tests::build_workspace;
use daruda_store::tasks::{Task, TaskRunIn};
use gpui::{
    AppContext as _, BorrowAppContext as _, Focusable as _, TestAppContext, VisualTestContext,
};

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
            ws.open_task_edit_pane(None, window, cx);
            let pane = ws.active_runtime().focused_pane_id;
            let title = ws
                .task_edit_content_for_pane(pane)
                .unwrap()
                .title_input
                .clone();
            title.update(cx, |s, cx| s.set_value("Keep watching", window, cx));
            ws.save_task_editor(pane, false, window, cx);
            let id = ws
                .task_edit_content_for_pane(pane)
                .unwrap()
                .task_id
                .clone()
                .unwrap();
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
            ws.attach_prompt_watcher_if_pane_open(&id, window, cx);
            assert!(
                ws.task_edit_content_for_pane(pane)
                    .unwrap()
                    ._prompt_watcher
                    .is_some()
            );
            ws.handle_prompt_file_changed(pane, path, window, cx);
            let te = ws.task_edit_content_for_pane(pane).unwrap();
            assert_eq!(te.prompt_state.read(cx).value().as_ref(), "External update");
            assert!(!te.is_dirty(cx));
        });
    })
    .unwrap();
}

#[gpui::test]
fn task_editor_save_shortcut_and_tab_order(cx: &mut TestAppContext) {
    let (window, ws) = build_workspace(cx);
    let mut vcx = VisualTestContext::from_window(window.into(), cx);
    vcx.cx.update(crate::bind_keys::register_static_bindings);
    vcx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            ws.open_task_edit_pane(None, window, cx);
            let pane = ws.active_runtime().focused_pane_id;
            ws.toggle_task_settings(pane, cx);
            let title = ws
                .task_edit_content_for_pane(pane)
                .unwrap()
                .title_input
                .clone();
            title.update(cx, |s, cx| s.set_value("Keyboard save", window, cx));
            window.refresh();
        });
    });
    vcx.run_until_parked();
    ws.read_with(&vcx, |ws, cx| {
        let te = ws
            .task_edit_content_for_pane(ws.active_runtime().focused_pane_id)
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
        let te = ws
            .task_edit_content_for_pane(ws.active_runtime().focused_pane_id)
            .unwrap();
        let task = cx
            .global::<GlobalTasks>()
            .get(te.task_id.as_ref().expect("shortcut saved the draft"))
            .unwrap();
        assert_eq!(task.title, "Keyboard save");
        assert!(!te.is_dirty(cx));
    });
}

#[gpui::test]
fn task_editor_rejects_blank_titles_and_invalid_branches(cx: &mut TestAppContext) {
    let (window, ws) = build_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.open_task_edit_pane(None, window, cx);
            let id = ws.active_runtime().focused_pane_id;
            assert!(ws.commit_task_edit_pane(id, cx).is_none());
            let te = ws.task_edit_content_for_pane(id).unwrap();
            let title = te.title_input.clone();
            let branch = te.branch_input.clone();
            title.update(cx, |s, cx| s.set_value("   ", window, cx));
            assert!(ws.commit_task_edit_pane(id, cx).is_none());
            title.update(cx, |s, cx| s.set_value("A task", window, cx));
            branch.update(cx, |s, cx| s.set_value("invalid branch", window, cx));
            ws.on_task_edit_branch_typed(id, cx);
            assert!(!ws.task_edit_content_for_pane(id).unwrap().can_save(cx));
            assert!(ws.commit_task_edit_pane(id, cx).is_none());
            assert!(cx.global::<GlobalTasks>().0.tasks.is_empty());
        });
    })
    .unwrap();
}

#[gpui::test]
fn task_editor_save_keeps_pane_and_draft_subtasks(cx: &mut TestAppContext) {
    let (window, ws) = build_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.open_task_edit_pane(None, window, cx);
            let pane = ws.active_runtime().focused_pane_id;
            let te = ws.task_edit_content_for_pane(pane).unwrap();
            let title = te.title_input.clone();
            let subtask = te.new_subtask_input.clone();
            title.update(cx, |s, cx| s.set_value("Ship editor", window, cx));
            ws.refresh_task_edit_title(pane, cx);
            subtask.update(cx, |s, cx| s.set_value("  Verify keyboard  ", window, cx));
            ws.submit_new_subtask(pane, window, cx);
            let sub = ws.task_edit_content_for_pane(pane).unwrap().draft_subtasks[0]
                .id
                .clone();
            ws.toggle_editor_subtask(pane, &sub, cx);
            ws.save_task_editor(pane, false, window, cx);
            let te = ws
                .task_edit_content_for_pane(pane)
                .expect("save keeps editor open");
            assert!(!te.is_dirty(cx));
            let id = te.task_id.clone().unwrap();
            let task = cx.global::<GlobalTasks>().get(&id).unwrap();
            assert_eq!(task.subtasks[0].title, "Verify keyboard");
            assert!(task.subtasks[0].completed);
            ws.save_task_editor(pane, false, window, cx);
            assert_eq!(cx.global::<GlobalTasks>().0.tasks.len(), 1);
            assert_eq!(
                cx.global::<GlobalTasks>().get(&id).unwrap().subtasks.len(),
                1
            );
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
            let task = Task::new("Original".into(), String::new(), None);
            let branch = task.branch_name.clone();
            let id = task.id.clone();
            cx.update_global::<GlobalTasks, _>(|g, _| {
                g.add(task);
            });
            ws.open_task_edit_pane(Some(id.clone()), window, cx);
            let pane = ws.active_runtime().focused_pane_id;
            let title = ws
                .task_edit_content_for_pane(pane)
                .unwrap()
                .title_input
                .clone();
            title.update(cx, |s, cx| s.set_value("Renamed", window, cx));
            ws.refresh_task_edit_title(pane, cx);
            ws.save_task_editor(pane, false, window, cx);
            let task = cx.global::<GlobalTasks>().get(&id).unwrap();
            assert_eq!(task.title, "Renamed");
            assert_eq!(task.branch_name, branch);
            assert_eq!(
                ws.task_edit_content_for_pane(pane)
                    .unwrap()
                    .cached_title
                    .as_ref(),
                "Renamed"
            );
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
            ws.open_task_edit_pane(None, window, cx);
            let pane = ws.active_runtime().focused_pane_id;
            let te = ws.task_edit_content_for_pane(pane).unwrap();
            let (title, branch) = (te.title_input.clone(), te.branch_input.clone());
            let prefilled = branch.read(cx).value().to_string();
            title.update(cx, |s, cx| s.set_value("한글 제목 with spaces", window, cx));
            ws.refresh_task_edit_title(pane, cx);
            assert_eq!(branch.read(cx).value().as_ref(), prefilled);
            assert_eq!(
                ws.task_edit_content_for_pane(pane)
                    .unwrap()
                    .cached_title
                    .as_ref(),
                "한글 제목 with spaces"
            );
            branch.update(cx, |s, cx| s.set_value("custom", window, cx));
            ws.on_task_edit_branch_typed(pane, cx);
            title.update(cx, |s, cx| s.set_value("Other", window, cx));
            ws.refresh_task_edit_title(pane, cx);
            assert_eq!(branch.read(cx).value().as_ref(), "custom");
            ws.regenerate_task_branch(pane, window, cx);
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
            ws.open_task_edit_pane(None, window, cx);
            let pane = ws.active_runtime().focused_pane_id;
            let title = ws
                .task_edit_content_for_pane(pane)
                .unwrap()
                .title_input
                .clone();
            title.update(cx, |s, cx| s.set_value("Keep this draft", window, cx));
            ws.request_close_pane(pane, window, cx);
            assert!(ws.task_edit_content_for_pane(pane).is_some());
        });
    })
    .unwrap();
    assert!(cx.has_pending_prompt());
}

#[gpui::test]
fn task_editor_distinct_drafts_and_empty_branch_save(cx: &mut TestAppContext) {
    let (window, ws) = build_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            let mut branches = Vec::new();
            for _ in 0..2 {
                ws.open_task_edit_pane(None, window, cx);
                let pane = ws.active_runtime().focused_pane_id;
                let te = ws.task_edit_content_for_pane(pane).unwrap();
                let title = te.title_input.clone();
                title.update(cx, |s, cx| s.set_value("Same title", window, cx));
                ws.refresh_task_edit_title(pane, cx);
                branches.push(
                    ws.task_edit_content_for_pane(pane)
                        .unwrap()
                        .branch_input
                        .read(cx)
                        .value(),
                );
            }
            assert_ne!(branches[0], branches[1]);
            let pane = ws.active_runtime().focused_pane_id;
            let branch = ws
                .task_edit_content_for_pane(pane)
                .unwrap()
                .branch_input
                .clone();
            branch.update(cx, |s, cx| s.set_value("", window, cx));
            ws.on_task_edit_branch_typed(pane, cx);
            ws.save_task_editor(pane, false, window, cx);
            let te = ws.task_edit_content_for_pane(pane).unwrap();
            let task = cx
                .global::<GlobalTasks>()
                .get(te.task_id.as_ref().unwrap())
                .unwrap();
            assert_eq!(branch.read(cx).value().as_ref(), task.branch_name);
            assert!(!te.is_dirty(cx));
        });
    })
    .unwrap();
}

#[gpui::test]
fn task_editor_stores_the_branch_trimmed(cx: &mut TestAppContext) {
    let (window, ws) = build_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.open_task_edit_pane(None, window, cx);
            let pane = ws.active_runtime().focused_pane_id;
            let te = ws.task_edit_content_for_pane(pane).unwrap();
            let (title, branch) = (te.title_input.clone(), te.branch_input.clone());
            title.update(cx, |s, cx| s.set_value("Trim me", window, cx));
            branch.update(cx, |s, cx| s.set_value("  fix-trim  ", window, cx));
            ws.on_task_edit_branch_typed(pane, cx);
            let id = ws.commit_task_edit_pane(pane, cx).unwrap();
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
            let backlog = Task::new("Backlog".into(), String::new(), None);
            let mut running = Task::new("Running".into(), String::new(), None);
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
                ws.open_task_edit_pane(Some(id.clone()), window, cx);
                let pane = ws.active_runtime().focused_pane_id;
                let branch = ws
                    .task_edit_content_for_pane(pane)
                    .unwrap()
                    .branch_input
                    .clone();
                branch.update(cx, |s, cx| s.set_value("fix-renamed", window, cx));
                ws.on_task_edit_branch_typed(pane, cx);
                ws.commit_task_edit_pane(pane, cx).unwrap();
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
            ws.open_task_edit_pane(None, window, cx);
            let pane = ws.active_runtime().focused_pane_id;
            let te = ws.task_edit_content_for_pane(pane).unwrap();
            let (title, branch) = (te.title_input.clone(), te.branch_input.clone());
            title.update(cx, |s, cx| s.set_value("On main", window, cx));
            branch.update(cx, |s, cx| s.set_value("main", window, cx));
            ws.on_task_edit_branch_typed(pane, cx);
            let te = ws.task_edit_content_for_pane(pane).unwrap();
            assert_eq!(te.branch_validation, BranchValidation::Exists);
            assert!(!te.can_save(cx));
            assert!(ws.commit_task_edit_pane(pane, cx).is_none());
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
            ws.open_task_edit_pane(None, window, cx);
            let pane = ws.active_runtime().focused_pane_id;
            let te = ws.task_edit_content_for_pane(pane).unwrap();
            let (title, branch) = (te.title_input.clone(), te.branch_input.clone());
            title.update(cx, |s, cx| s.set_value("Late", window, cx));
            branch.update(cx, |s, cx| s.set_value("feat-late", window, cx));
            ws.on_task_edit_branch_typed(pane, cx);
            push_git_lane(ws, "feat-late");
            assert!(ws.commit_task_edit_pane(pane, cx).is_none());
            assert_eq!(
                ws.task_edit_content_for_pane(pane)
                    .unwrap()
                    .branch_validation,
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
            ws.open_task_edit_pane(None, window, cx);
            let pane = ws.active_runtime().focused_pane_id;
            let title = ws
                .task_edit_content_for_pane(pane)
                .unwrap()
                .title_input
                .clone();
            title.update(cx, |s, cx| s.set_value("In place", window, cx));
            ws.set_task_run_in(pane, RunInChoice::ExistingLane, cx);
            let id = ws.commit_task_edit_pane(pane, cx).unwrap();
            let task = cx.global::<GlobalTasks>().get(&id).unwrap();
            assert_eq!(task.run_in, TaskRunIn::ExistingLane { path: lane });
            assert_eq!(task.base_worktree_path, None);
            assert!(!ws.task_edit_content_for_pane(pane).unwrap().is_dirty(cx));
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
            let mut task = Task::new("Running".into(), String::new(), None);
            let lane = push_git_lane(ws, &task.branch_name);
            task.state = daruda_store::tasks::TaskState::Running {
                worktree_path: lane,
            };
            let id = task.id.clone();
            cx.update_global::<GlobalTasks, _>(|g, _| {
                g.add(task);
            });
            ws.open_task_edit_pane(Some(id.clone()), window, cx);
            let pane = ws.active_runtime().focused_pane_id;
            let title = ws
                .task_edit_content_for_pane(pane)
                .unwrap()
                .title_input
                .clone();
            title.update(cx, |s, cx| s.set_value("Renamed", window, cx));
            assert!(ws.task_edit_content_for_pane(pane).unwrap().can_save(cx));
            ws.commit_task_edit_pane(pane, cx).unwrap();
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
            let task = Task::new("Move me".into(), String::new(), None);
            let id = task.id.clone();
            cx.update_global::<GlobalTasks, _>(|g, _| {
                g.add(task);
            });
            ws.open_task_edit_pane(Some(id.clone()), window, cx);
            let pane = ws.active_runtime().focused_pane_id;
            assert!(!ws.task_edit_content_for_pane(pane).unwrap().is_dirty(cx));
            ws.set_task_run_in(pane, RunInChoice::ExistingLane, cx);
            assert!(ws.task_edit_content_for_pane(pane).unwrap().is_dirty(cx));
            ws.commit_task_edit_pane(pane, cx).unwrap();
            assert!(!ws.task_edit_content_for_pane(pane).unwrap().is_dirty(cx));
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
            let mut task = Task::new("Reopened".into(), String::new(), None);
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
            ws.open_task_edit_pane(Some(id.clone()), window, cx);
            let pane = ws.active_runtime().focused_pane_id;
            let title = ws
                .task_edit_content_for_pane(pane)
                .unwrap()
                .title_input
                .clone();
            title.update(cx, |s, cx| s.set_value("Edited", window, cx));
            assert!(ws.task_edit_content_for_pane(pane).unwrap().can_save(cx));
            ws.commit_task_edit_pane(pane, cx).unwrap();
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
            ws.open_task_edit_pane(None, window, cx);
            let pane = ws.active_runtime().focused_pane_id;
            let te = ws.task_edit_content_for_pane(pane).unwrap();
            let branch = te.branch_input.clone();
            let first = branch.read(cx).value().to_string();
            assert!(first.starts_with("task-"), "prefilled: {first}");
            assert_eq!(te.branch_validation, BranchValidation::Valid);
            assert!(!te.is_dirty(cx), "the prefill is not an edit");

            ws.regenerate_task_branch(pane, window, cx);
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
            let task = Task::new("Lines".into(), "one\ntwo".into(), None);
            let id = task.id.clone();
            cx.update_global::<GlobalTasks, _>(|g, _| {
                g.add(task);
            });
            ws.open_task_edit_pane(Some(id), window, cx);
            let pane = ws.active_runtime().focused_pane_id;
            let prompt = ws
                .task_edit_content_for_pane(pane)
                .unwrap()
                .prompt_state
                .clone();
            assert!(!ws.task_edit_content_for_pane(pane).unwrap().is_dirty(cx));
            prompt.update(cx, |s, cx| s.set_value("one\r\ntwo", window, cx));
            assert!(!ws.task_edit_content_for_pane(pane).unwrap().is_dirty(cx));
            prompt.update(cx, |s, cx| s.set_value("one\r\nthree", window, cx));
            assert!(ws.task_edit_content_for_pane(pane).unwrap().is_dirty(cx));
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
                "Old base".into(),
                String::new(),
                Some(std::path::PathBuf::from("/nonexistent/daruda-base")),
            );
            let id = task.id.clone();
            cx.update_global::<GlobalTasks, _>(|g, _| {
                g.add(task);
            });
            ws.open_task_edit_pane(Some(id), window, cx);
            let pane = ws.active_runtime().focused_pane_id;
            assert!(!ws.task_edit_content_for_pane(pane).unwrap().is_dirty(cx));
            let title = ws
                .task_edit_content_for_pane(pane)
                .unwrap()
                .title_input
                .clone();
            title.update(cx, |s, cx| s.set_value("Renamed", window, cx));
            assert!(ws.task_edit_content_for_pane(pane).unwrap().is_dirty(cx));
            ws.commit_task_edit_pane(pane, cx).unwrap();
            assert!(!ws.task_edit_content_for_pane(pane).unwrap().is_dirty(cx));
        });
    })
    .unwrap();
}
