use crate::agent::tasks_global::GlobalTasks;
use crate::workspace::tests::build_workspace;
use daruda_store::tasks::Task;
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
            let branch = cx
                .global::<GlobalTasks>()
                .get(&id)
                .unwrap()
                .branch_name
                .clone();

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
            let path = daruda_store::tasks::prompt_file::write_prompt_file(
                other.path(),
                &branch,
                "External update",
            )
            .unwrap();
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
            ws.refresh_task_edit_branch(pane, window, cx);
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

#[gpui::test]
fn task_editor_title_edits_keep_custom_and_saved_branches(cx: &mut TestAppContext) {
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
            ws.refresh_task_edit_branch(pane, window, cx);
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

#[gpui::test]
fn task_editor_auto_branch_follows_title_until_overridden(cx: &mut TestAppContext) {
    let (window, ws) = build_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.open_task_edit_pane(None, window, cx);
            let pane = ws.active_runtime().focused_pane_id;
            let te = ws.task_edit_content_for_pane(pane).unwrap();
            let title = te.title_input.clone();
            let branch = te.branch_input.clone();
            title.update(cx, |s, cx| s.set_value("First", window, cx));
            ws.refresh_task_edit_branch(pane, window, cx);
            ws.on_task_edit_branch_typed(pane, cx);
            assert!(!ws.task_edit_content_for_pane(pane).unwrap().branch_override);
            title.update(cx, |s, cx| s.set_value("Second", window, cx));
            ws.refresh_task_edit_branch(pane, window, cx);
            assert!(branch.read(cx).value().starts_with("second-"));
            branch.update(cx, |s, cx| s.set_value("custom", window, cx));
            ws.on_task_edit_branch_typed(pane, cx);
            title.update(cx, |s, cx| s.set_value("Third", window, cx));
            ws.refresh_task_edit_branch(pane, window, cx);
            assert_eq!(branch.read(cx).value().as_ref(), "custom");
            assert_eq!(
                ws.task_edit_content_for_pane(pane)
                    .unwrap()
                    .cached_title
                    .as_ref(),
                "Third"
            );
            ws.reset_task_branch(pane, window, cx);
            assert!(branch.read(cx).value().starts_with("third-"));
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
                ws.refresh_task_edit_branch(pane, window, cx);
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
