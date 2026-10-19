//! Right-panel Tasks tab — workspace-side behaviour tests.
//!
//! Covers the lifecycle paths whose correctness depends on
//! `Workspace` state (counter cleanup, escalation thresholds, hook
//! mapping). Pure data-model checks live next to the data in
//! `daruda_store::tasks::tests`.

use super::build_workspace;

use daruda_store::tasks::machine::ErrorCause;
use daruda_store::tasks::{
    CliProcessState, Lifecycle, SessionEndReason, TASK_TOOL_USE_FAILURE_THRESHOLD, TaskAgentSurface,
};
use gpui::{AppContext as _, TestAppContext};

use crate::runs::GlobalTasks;
use crate::runs::seed::{SeedEnd, SeedRun};

#[gpui::test]
fn illegal_task_commands_report_without_changing_state(cx: &mut TestAppContext) {
    let (_window, workspace) = build_workspace(cx);
    let (done, running) = workspace.update(cx, |_ws, cx| {
        let run = SeedRun::chat("/wt");
        let done = crate::runs::seed::task(
            crate::runs::seed::spec(Default::default(), "done"),
            Some((&run, SeedEnd::Done)),
            cx,
        );
        let running = crate::runs::seed::task(
            crate::runs::seed::spec(Default::default(), "running"),
            Some((&run, SeedEnd::Running)),
            cx,
        );
        (done, running)
    });
    workspace.update(cx, |ws, cx| {
        ws.cancel_task(&done, cx);
        ws.reopen_task(&running, cx);
    });
    cx.run_until_parked();
    assert!(matches!(lifecycle(&done, cx), Lifecycle::Done { .. }));
    assert!(matches!(lifecycle(&running, cx), Lifecycle::Running { .. }));
    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.error_history().len(), 2);
        assert!(
            ws.error_history()
                .iter()
                .all(|report| report.dedup_key.as_deref() == Some("tasks.command_refused"))
        );
    });
}

/// A running CLI task with `sessions` attached; answers its id.
fn seed_cli_task(
    workspace: &gpui::Entity<crate::workspace::Workspace>,
    cx: &mut TestAppContext,
    session_id: Option<&str>,
    sessions: &[&str],
) -> String {
    workspace.update(cx, |_ws, cx| {
        let mut spec =
            crate::runs::seed::spec(daruda_store::project::ProjectUuid::default(), "fix-bug");
        spec.agent_surface = TaskAgentSurface::Terminal;
        let mut run = SeedRun::cli("/tmp/wt", CliProcessState::Running { pid: 1 });
        if let Some(session) = session_id {
            run = run.session(session);
        }
        for session in sessions {
            run = run.attach(*session);
        }
        crate::runs::seed::task(spec, Some((&run, SeedEnd::Running)), cx)
    })
}

/// Insert a running task with one attached session and return the task
/// id. Shared fixture for the escalation-counter tests.
fn seed_running_task(
    workspace: &gpui::Entity<crate::workspace::Workspace>,
    cx: &mut TestAppContext,
    session_id: &str,
) -> String {
    seed_cli_task(workspace, cx, Some(session_id), &[session_id])
}

fn lifecycle(id: &str, cx: &mut TestAppContext) -> Lifecycle {
    cx.update(|cx| {
        cx.global::<GlobalTasks>()
            .get(id)
            .unwrap()
            .lifecycle
            .clone()
    })
}

#[gpui::test]
fn bump_tool_use_failure_thresholds_and_orphans(cx: &mut TestAppContext) {
    let (_wh, workspace) = build_workspace(cx);
    let below_id = seed_running_task(&workspace, cx, "sess-below");
    let threshold_id = seed_running_task(&workspace, cx, "sess-threshold");

    workspace.update(cx, |ws, cx| {
        // Bump just under the threshold — task must stay Running and
        // the counter must reflect the bumps.
        for _ in 0..(TASK_TOOL_USE_FAILURE_THRESHOLD - 1) {
            ws.bump_tool_use_failure("sess-below", cx);
        }
        assert_eq!(
            ws.claude_sessions.tool_use_failure_counts.get("sess-below"),
            Some(&(TASK_TOOL_USE_FAILURE_THRESHOLD - 1))
        );
        for _ in 0..TASK_TOOL_USE_FAILURE_THRESHOLD {
            ws.bump_tool_use_failure("sess-threshold", cx);
        }
        // Counter is removed once escalation fires.
        assert!(
            !ws.claude_sessions
                .tool_use_failure_counts
                .contains_key("sess-threshold")
        );
    });
    cx.run_until_parked();
    assert!(matches!(
        lifecycle(&below_id, cx),
        Lifecycle::Running { .. }
    ));
    cx.update(|cx| {
        let task = cx.global::<GlobalTasks>().get(&threshold_id).unwrap();
        match &task.lifecycle {
            Lifecycle::Error { cause, .. } => assert_eq!(*cause, ErrorCause::ToolFailures),
            other => panic!("expected Error, got {other:?}"),
        }
        // session_ids is cleared so the next Retry starts fresh.
        assert!(task.session_ids.is_empty());
        // finished_at marks the escalation moment.
        assert!(task.finished_at.is_some());
    });

    workspace.update(cx, |ws, cx| {
        for _ in 0..(TASK_TOOL_USE_FAILURE_THRESHOLD + 2) {
            ws.bump_tool_use_failure("sess-orphan", cx);
        }
        assert!(
            !ws.claude_sessions
                .tool_use_failure_counts
                .contains_key("sess-orphan"),
            "unowned sessions must not allocate a counter entry"
        );
    });
}

#[gpui::test]
fn failure_counter_clears_when_task_leaves_running_or_session_ends(cx: &mut TestAppContext) {
    let (wh, workspace) = build_workspace(cx);
    let _task_id = seed_running_task(&workspace, cx, "sess-end");

    workspace.update(cx, |ws, cx| {
        ws.bump_tool_use_failure("sess-end", cx);
        ws.bump_tool_use_failure("sess-end", cx);
        assert_eq!(
            ws.claude_sessions.tool_use_failure_counts.get("sess-end"),
            Some(&2)
        );

        ws.apply_task_session_ended("sess-end", SessionEndReason::Stop, cx);
        assert!(
            !ws.claude_sessions
                .tool_use_failure_counts
                .contains_key("sess-end"),
            "apply_task_session_ended must drop the failure counter"
        );
    });
    cx.run_until_parked();

    let task_id = seed_running_task(&workspace, cx, "sess-cancel");
    workspace.update(cx, |ws, cx| {
        ws.bump_tool_use_failure("sess-cancel", cx);
        ws.bump_tool_use_failure("sess-cancel", cx);
        assert_eq!(
            ws.claude_sessions
                .tool_use_failure_counts
                .get("sess-cancel"),
            Some(&2)
        );

        ws.cancel_task(&task_id, cx);
        assert!(
            !ws.claude_sessions
                .tool_use_failure_counts
                .contains_key("sess-cancel"),
            "cancel_task must drop the failure counter"
        );
    });
    cx.run_until_parked();

    let task_id = seed_running_task(&workspace, cx, "sess-reopen");
    workspace.update(cx, |ws, cx| {
        ws.bump_tool_use_failure("sess-reopen", cx);
        ws.bump_tool_use_failure("sess-reopen", cx);
        ws.reopen_task(&task_id, cx);
        assert!(
            !ws.claude_sessions
                .tool_use_failure_counts
                .contains_key("sess-reopen"),
            "reopen_task must drop the failure counter"
        );
    });
    cx.run_until_parked();

    let task_id = seed_running_task(&workspace, cx, "sess-delete");
    cx.update_window(wh.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.bump_tool_use_failure("sess-delete", cx);
            ws.delete_task(&task_id, window, cx);
            assert!(
                !ws.claude_sessions
                    .tool_use_failure_counts
                    .contains_key("sess-delete"),
                "delete_task must drop the failure counter"
            );
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(
            cx.global::<GlobalTasks>().get(&task_id).is_none(),
            "a running task is cancelled, then deleted"
        );
    });
}

#[test]
fn classify_hook_end_reason_maps_known_events() {
    use crate::workspace::Workspace;

    assert_eq!(
        Workspace::classify_hook_end_reason("Stop"),
        Some(SessionEndReason::Stop),
    );
    assert_eq!(
        Workspace::classify_hook_end_reason("SessionEnd"),
        Some(SessionEndReason::Other),
    );
    assert_eq!(
        Workspace::classify_hook_end_reason("PreToolUse"),
        None,
        "non-terminal events must not transition tasks",
    );
    assert_eq!(
        Workspace::classify_hook_end_reason("PostToolUseFailure"),
        None,
        "PostToolUseFailure routes through the counter, not the end-reason classifier",
    );
}

#[gpui::test]
fn apply_task_session_changed_attaches_idempotently_then_error_ends(cx: &mut TestAppContext) {
    let (_wh, workspace) = build_workspace(cx);
    // Attachment follows the run's recorded session, never the cwd alone.
    let id = seed_cli_task(&workspace, cx, Some("sess-2"), &[]);
    workspace.update(cx, |ws, cx| {
        ws.apply_task_session_changed("sess-other", cx);
        ws.apply_task_session_changed("sess-2", cx);
    });
    cx.run_until_parked();
    workspace.update(cx, |ws, cx| ws.apply_task_session_changed("sess-2", cx));
    cx.run_until_parked();
    cx.update(|cx| {
        let t = cx.global::<GlobalTasks>().get(&id).unwrap();
        assert_eq!(t.session_ids.as_slice(), &["sess-2".to_string()]);
    });

    workspace.update(cx, |ws, cx| {
        ws.apply_task_session_ended("sess-2", SessionEndReason::Error, cx);
    });
    cx.run_until_parked();
    match lifecycle(&id, cx) {
        Lifecycle::Error { cause, .. } => assert_eq!(cause, ErrorCause::SessionFailed),
        other => panic!("expected Error, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Subtask ops
// ---------------------------------------------------------------------------

/// Insert a Backlog task and return its id. Subtask tests don't need a
/// session or lane.
fn seed_backlog_task(
    workspace: &gpui::Entity<crate::workspace::Workspace>,
    cx: &mut TestAppContext,
) -> String {
    workspace.update(cx, |_ws, cx| {
        let spec = crate::runs::seed::spec(
            daruda_store::project::ProjectUuid::default(),
            "subtask-host",
        );
        crate::runs::seed::task(spec, None, cx)
    })
}

fn subtasks(id: &str, cx: &mut TestAppContext) -> Vec<daruda_store::tasks::SubTask> {
    cx.update(|cx| cx.global::<GlobalTasks>().get(id).unwrap().subtasks.clone())
}

#[gpui::test]
fn subtask_ops_add_toggle_rename_and_delete(cx: &mut TestAppContext) {
    let (_wh, workspace) = build_workspace(cx);
    let task_id = seed_backlog_task(&workspace, cx);

    workspace.update(cx, |ws, cx| {
        ws.add_subtask(&task_id, "first".into(), cx);
        ws.add_subtask(&task_id, "   ".into(), cx); // whitespace → no-op
        ws.add_subtask(&task_id, "  second  ".into(), cx); // trimmed
    });
    cx.run_until_parked();
    let t = subtasks(&task_id, cx);
    assert_eq!(t.len(), 2);
    assert_eq!(t[0].title, "first");
    assert_eq!(t[1].title, "second");
    assert!(t.iter().all(|s| s.source_session_id.is_none()));
    let (first_id, second_id) = (t[0].id.clone(), t[1].id.clone());

    workspace.update(cx, |ws, cx| ws.toggle_subtask(&task_id, &first_id, cx));
    cx.run_until_parked();
    assert!(subtasks(&task_id, cx)[0].completed);
    workspace.update(cx, |ws, cx| ws.toggle_subtask(&task_id, &first_id, cx));
    cx.run_until_parked();
    assert!(!subtasks(&task_id, cx)[0].completed);

    workspace.update(cx, |ws, cx| {
        ws.rename_subtask(&task_id, &second_id, "  new title  ".into(), cx)
    });
    cx.run_until_parked();
    assert_eq!(
        subtasks(&task_id, cx)[1].title,
        "new title",
        "rename trims whitespace"
    );
    workspace.update(cx, |ws, cx| {
        ws.rename_subtask(&task_id, &second_id, "   ".into(), cx)
    });
    cx.run_until_parked();
    assert_eq!(
        subtasks(&task_id, cx)[1].title,
        "new title",
        "empty rename is ignored"
    );

    workspace.update(cx, |ws, cx| ws.delete_subtask(&task_id, &first_id, cx));
    cx.run_until_parked();
    let t = subtasks(&task_id, cx);
    assert_eq!(t.len(), 1);
    assert_eq!(t[0].title, "new title");
}

// ---------------------------------------------------------------------------
// TodoWrite hook merge
// ---------------------------------------------------------------------------

#[gpui::test]
fn apply_todo_write_merges_auto_items_and_keeps_boundaries(cx: &mut TestAppContext) {
    let (_wh, workspace) = build_workspace(cx);
    let auto_task_id = seed_running_task(&workspace, cx, "sess-todo-1");
    let manual_task_id = seed_running_task(&workspace, cx, "sess-todo-3");
    let bad_task_id = seed_running_task(&workspace, cx, "sess-todo-bad");
    let isolated_task_id = seed_cli_task(&workspace, cx, None, &["sess-A", "sess-B"]);

    workspace.update(cx, |ws, cx| {
        let payload = serde_json::json!({
            "todos": [
                { "content": "Inspect session.rs", "status": "in_progress" },
                { "content": "Add refresh logic", "status": "completed" },
            ]
        });
        ws.apply_todo_write("sess-todo-1", &payload, cx);
    });
    cx.run_until_parked();
    let t = subtasks(&auto_task_id, cx);
    assert_eq!(t.len(), 2);
    assert_eq!(t[0].title, "Inspect session.rs");
    assert!(!t[0].completed);
    assert_eq!(
        t[0].source_session_id.as_deref(),
        Some("sess-todo-1"),
        "hook-injected items carry the session id (auto namespace)",
    );
    assert!(
        t[1].completed,
        "status: completed -> SubTask.completed = true",
    );

    workspace.update(cx, |ws, cx| {
        let second = serde_json::json!({
            "todos": [{ "content": "Inspect session.rs", "status": "completed" }]
        });
        ws.apply_todo_write("sess-todo-1", &second, cx);
        // User typed an identical title manually first.
        ws.add_subtask(&manual_task_id, "step A".into(), cx);
    });
    cx.run_until_parked();
    let t = subtasks(&auto_task_id, cx);
    assert_eq!(t.len(), 2, "second emission folds into row 1");
    let inspect = t.iter().find(|s| s.title == "Inspect session.rs").unwrap();
    assert!(inspect.completed);
    assert!(
        t.iter().any(|s| s.title == "Add refresh logic"),
        "dropped items are preserved"
    );
    let manual_id = subtasks(&manual_task_id, cx)[0].id.clone();

    workspace.update(cx, |ws, cx| {
        let payload = serde_json::json!({
            "todos": [{ "content": "step A", "status": "completed" }]
        });
        ws.apply_todo_write("sess-todo-3", &payload, cx);
        let payload = serde_json::json!({
            "todos": [{ "content": "shared title", "status": "in_progress" }]
        });
        ws.apply_todo_write("sess-A", &payload, cx);
        // Same title from a different session -> must push a new row, not fold
        // into the sess-A entry.
        ws.apply_todo_write("sess-B", &payload, cx);
        // `todos` is a string instead of an array — schema drift.
        let payload = serde_json::json!({ "todos": "not-an-array" });
        ws.apply_todo_write("sess-todo-bad", &payload, cx);
        let payload = serde_json::json!({
            "todos": [{ "content": "x", "status": "completed" }]
        });
        ws.apply_todo_write("sess-unknown", &payload, cx);
    });
    cx.run_until_parked();

    let t = subtasks(&manual_task_id, cx);
    assert_eq!(t.len(), 2, "manual + auto coexist");
    let manual = t.iter().find(|s| s.id == manual_id).unwrap();
    assert!(
        !manual.completed,
        "manual subtask completion is left alone by the hook",
    );
    assert!(manual.source_session_id.is_none());
    let auto = t
        .iter()
        .find(|s| s.source_session_id.as_deref() == Some("sess-todo-3"))
        .unwrap();
    assert!(auto.completed);

    let t = subtasks(&isolated_task_id, cx);
    assert_eq!(t.len(), 2);
    let owners: Vec<&str> = t
        .iter()
        .map(|s| s.source_session_id.as_deref().unwrap_or(""))
        .collect();
    assert!(owners.contains(&"sess-A"));
    assert!(owners.contains(&"sess-B"));

    assert!(
        subtasks(&bad_task_id, cx).is_empty(),
        "parse failure must not mutate state, and no task owns sess-unknown"
    );
}

fn git_repo(dir: &std::path::Path) {
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["config", "user.email", "daruda@test"],
        &["config", "user.name", "daruda"],
        &["commit", "-q", "--allow-empty", "-m", "initial"],
    ] {
        let status = std::process::Command::new("git")
            .current_dir(dir)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?} failed in {dir:?}");
    }
}

/// A task's worktree is made in the repository of the project it belongs
/// to, not in whichever project the window happens to be showing.
#[gpui::test]
fn a_task_starts_in_its_own_project_while_another_is_active(cx: &mut TestAppContext) {
    if !crate::lane::git::has_git() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let (a_root, b_root) = (temp.path().join("a"), temp.path().join("b"));
    for root in [&a_root, &b_root] {
        std::fs::create_dir_all(root).unwrap();
        git_repo(root);
    }
    let (window, ws) = super::build_workspace_with(
        cx,
        &daruda_config::Config::default(),
        Some(daruda_store::project::Project::from_path(&a_root)),
    );
    ws.update(cx, |ws, cx| ws.reconcile_bootstrapped_lanes(cx));
    cx.run_until_parked();
    let (task_id, expected, rx) = cx
        .update_window(window.into(), |_, window, cx| {
            ws.update(cx, |ws, cx| {
                let a = &ws.projects[0];
                let (a_id, a_uuid) = (a.id, a.uuid);
                ws.add_project(b_root.clone(), window, cx);
                assert_ne!(ws.active.project, a_id, "project B is the active one");
                let mut task = crate::runs::seed::spec(a_uuid, "In A");
                task.agent_surface = TaskAgentSurface::Terminal;
                let expected = crate::workspace::lane_ops::lane_checkout_path(
                    &ws.git_repo_root_for_project(a_id)
                        .expect("A is a git project"),
                    &task.branch_name,
                );
                let id = crate::runs::seed::task(task, None, cx);
                let rx =
                    ws.begin_task_start(&id, daruda_store::tasks::StartedBy::Desktop, window, cx);
                assert!(rx.try_recv().is_err(), "git has yet to run");
                (id, expected, rx)
            })
        })
        .unwrap();
    cx.run_until_parked();
    let started = rx
        .try_recv()
        .expect("answered once git ran")
        .expect("started");
    assert_eq!(started.worktree, expected);
    ws.read_with(cx, |_, cx| {
        let task = cx.global::<GlobalTasks>().get(&task_id).unwrap();
        assert_eq!(
            task.lifecycle.worktree(),
            Some(expected.as_path()),
            "{:?}",
            task.lifecycle
        );
        assert!(matches!(task.lifecycle, Lifecycle::Running { .. }));
    });
}

/// Focusing a task's lane reaches it in whichever project holds it — the
/// active project need not be the task's.
#[gpui::test]
fn focusing_a_task_lane_crosses_projects(cx: &mut TestAppContext) {
    let a_root = tempfile::tempdir().unwrap();
    let b_root = tempfile::tempdir().unwrap();
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path(a_root.path());
    let (wh, workspace) = super::build_workspace_with(cx, &config, Some(project));
    cx.update_window(wh.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let a = ws.active;
            let a_path = ws.lane_for(a).unwrap().path.clone();
            let spec = crate::runs::seed::spec(ws.projects[0].uuid, "elsewhere");
            let id =
                crate::runs::seed::task(spec, Some((&SeedRun::chat(a_path), SeedEnd::Running)), cx);
            ws.add_project(b_root.path().to_path_buf(), window, cx);
            assert_ne!(ws.active.project, a.project, "the other project is active");
            ws.focus_task_lane(&id, window, cx);
            assert_eq!(ws.active, a, "the task's own lane, in its own project");
        });
    })
    .unwrap();
}
