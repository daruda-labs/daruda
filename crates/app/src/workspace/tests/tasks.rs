//! Right-panel Tasks tab — workspace-side behaviour tests.
//!
//! Covers the lifecycle paths whose correctness depends on
//! `Workspace` state (counter cleanup, escalation thresholds, hook
//! mapping). Pure data-model checks live next to the data in
//! `daruda_store::tasks::tests`.

use std::path::PathBuf;

use super::build_workspace;

use daruda_store::tasks::{SessionEndReason, TASK_TOOL_USE_FAILURE_THRESHOLD, Task, TaskState};
use gpui::{BorrowAppContext, TestAppContext};

/// Insert a `Running` task with one attached session and return the
/// task id. All P2-C tests share this fixture.
fn seed_running_task(
    workspace: &gpui::Entity<crate::workspace::Workspace>,
    cx: &mut TestAppContext,
    session_id: &str,
) -> String {
    workspace.update(cx, |_ws, cx| {
        let mut task = Task::new(
            "fix-bug".to_string(),
            "Investigate the auth flow.".to_string(),
            None,
        );
        task.state = TaskState::Running {
            worktree_path: PathBuf::from("/tmp/wt"),
        };
        task.session_ids.push(session_id.to_string());
        let id = task.id.clone();
        cx.update_global::<crate::agent::tasks_global::GlobalTasks, _>(|g, _| {
            g.add(task);
        });
        id
    })
}

#[gpui::test]
fn bump_below_threshold_keeps_task_running(cx: &mut TestAppContext) {
    let (_wh, workspace) = build_workspace(cx);
    let task_id = seed_running_task(&workspace, cx, "sess-1");

    workspace.update(cx, |ws, cx| {
        // Bump just under the threshold — task must stay Running and
        // the counter must reflect the bumps.
        for _ in 0..(TASK_TOOL_USE_FAILURE_THRESHOLD - 1) {
            ws.bump_tool_use_failure("sess-1", cx);
        }
        assert_eq!(
            ws.claude.tool_use_failure_counts.get("sess-1"),
            Some(&(TASK_TOOL_USE_FAILURE_THRESHOLD - 1))
        );
        let g = cx.global::<crate::agent::tasks_global::GlobalTasks>();
        let task = g.get(&task_id).unwrap();
        assert!(matches!(task.state, TaskState::Running { .. }));
    });
}

#[gpui::test]
fn bump_at_threshold_escalates_to_error(cx: &mut TestAppContext) {
    let (_wh, workspace) = build_workspace(cx);
    let task_id = seed_running_task(&workspace, cx, "sess-1");

    workspace.update(cx, |ws, cx| {
        for _ in 0..TASK_TOOL_USE_FAILURE_THRESHOLD {
            ws.bump_tool_use_failure("sess-1", cx);
        }
        // Counter is removed once escalation fires.
        assert!(!ws.claude.tool_use_failure_counts.contains_key("sess-1"));
        let g = cx.global::<crate::agent::tasks_global::GlobalTasks>();
        let task = g.get(&task_id).unwrap();
        match &task.state {
            TaskState::Error { message, .. } => {
                assert!(
                    message.starts_with("tool_use_failure x"),
                    "expected escalation message, got {message}"
                );
            }
            other => panic!("expected Error, got {other:?}"),
        }
        // session_ids is cleared so the next Retry starts fresh.
        assert!(task.session_ids.is_empty());
        // finished_at marks the escalation moment.
        assert!(task.finished_at.is_some());
    });
}

#[gpui::test]
fn bump_for_unowned_session_is_ignored(cx: &mut TestAppContext) {
    // No Running task owns "sess-orphan" — this models the case where
    // Claude is running outside daruda's task system (CLI, IDE plugin,
    // or a task that already moved out of Running). The counter must
    // not grow, so the orphan-escalation warning never fires.
    let (_wh, workspace) = build_workspace(cx);

    workspace.update(cx, |ws, cx| {
        for _ in 0..(TASK_TOOL_USE_FAILURE_THRESHOLD + 2) {
            ws.bump_tool_use_failure("sess-orphan", cx);
        }
        assert!(
            !ws.claude
                .tool_use_failure_counts
                .contains_key("sess-orphan"),
            "unowned sessions must not allocate a counter entry"
        );
    });
}

#[gpui::test]
fn session_end_clears_failure_counter(cx: &mut TestAppContext) {
    let (_wh, workspace) = build_workspace(cx);
    let _task_id = seed_running_task(&workspace, cx, "sess-1");

    workspace.update(cx, |ws, cx| {
        ws.bump_tool_use_failure("sess-1", cx);
        ws.bump_tool_use_failure("sess-1", cx);
        assert_eq!(ws.claude.tool_use_failure_counts.get("sess-1"), Some(&2));

        // Simulate a graceful session end. `apply_task_session_ended`
        // itself must wipe the counter — no manual remove from the
        // test (otherwise we'd be asserting against our own cleanup).
        ws.apply_task_session_ended("sess-1", SessionEndReason::Stop, cx);
        assert!(
            !ws.claude.tool_use_failure_counts.contains_key("sess-1"),
            "apply_task_session_ended must drop the failure counter"
        );
    });
}

#[gpui::test]
fn cancel_task_clears_failure_counter(cx: &mut TestAppContext) {
    let (_wh, workspace) = build_workspace(cx);
    let task_id = seed_running_task(&workspace, cx, "sess-cancel");

    workspace.update(cx, |ws, cx| {
        ws.bump_tool_use_failure("sess-cancel", cx);
        ws.bump_tool_use_failure("sess-cancel", cx);
        assert_eq!(
            ws.claude.tool_use_failure_counts.get("sess-cancel"),
            Some(&2)
        );

        ws.cancel_task(&task_id, cx);
        assert!(
            !ws.claude
                .tool_use_failure_counts
                .contains_key("sess-cancel"),
            "cancel_task must drop the failure counter"
        );
    });
}

#[gpui::test]
fn reopen_task_clears_failure_counter(cx: &mut TestAppContext) {
    let (_wh, workspace) = build_workspace(cx);
    let task_id = seed_running_task(&workspace, cx, "sess-reopen");

    workspace.update(cx, |ws, cx| {
        ws.bump_tool_use_failure("sess-reopen", cx);
        ws.bump_tool_use_failure("sess-reopen", cx);
        ws.reopen_task(&task_id, cx);
        assert!(
            !ws.claude
                .tool_use_failure_counts
                .contains_key("sess-reopen"),
            "reopen_task must drop the failure counter"
        );
    });
}

#[gpui::test]
fn delete_task_clears_failure_counter(cx: &mut TestAppContext) {
    let (_wh, workspace) = build_workspace(cx);
    let task_id = seed_running_task(&workspace, cx, "sess-delete");

    workspace.update(cx, |ws, cx| {
        ws.bump_tool_use_failure("sess-delete", cx);
        ws.delete_task(&task_id, cx);
        assert!(
            !ws.claude
                .tool_use_failure_counts
                .contains_key("sess-delete"),
            "delete_task must drop the failure counter"
        );
    });
}

#[gpui::test]
fn classify_hook_end_reason_maps_known_events(_cx: &mut TestAppContext) {
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
fn apply_task_session_changed_attaches_session_idempotently(cx: &mut TestAppContext) {
    let (_wh, workspace) = build_workspace(cx);
    workspace.update(cx, |ws, cx| {
        let mut task = Task::new("a".into(), "b".into(), None);
        task.state = TaskState::Running {
            worktree_path: PathBuf::from("/tmp/wt"),
        };
        let id = task.id.clone();
        cx.update_global::<crate::agent::tasks_global::GlobalTasks, _>(|g, _| {
            g.add(task);
        });

        ws.apply_task_session_changed(&PathBuf::from("/tmp/wt"), "sess-2", cx);
        ws.apply_task_session_changed(&PathBuf::from("/tmp/wt"), "sess-2", cx);

        let g = cx.global::<crate::agent::tasks_global::GlobalTasks>();
        let t = g.get(&id).unwrap();
        assert_eq!(t.session_ids.as_slice(), &["sess-2".to_string()]);
    });
}

#[gpui::test]
fn apply_task_session_ended_with_error_uses_session_error_message(cx: &mut TestAppContext) {
    let (_wh, workspace) = build_workspace(cx);
    let id = seed_running_task(&workspace, cx, "sess-3");

    workspace.update(cx, |ws, cx| {
        ws.apply_task_session_ended("sess-3", SessionEndReason::Error, cx);
        let g = cx.global::<crate::agent::tasks_global::GlobalTasks>();
        let t = g.get(&id).unwrap();
        match &t.state {
            TaskState::Error { message, .. } => {
                assert_eq!(message, "session error");
            }
            other => panic!("expected Error, got {other:?}"),
        }
    });
}

// ---------------------------------------------------------------------------
// R-21 subtask ops
// ---------------------------------------------------------------------------

/// Insert a Backlog task and return its id. Subtask tests don't need a
/// session or lane.
fn seed_backlog_task(
    workspace: &gpui::Entity<crate::workspace::Workspace>,
    cx: &mut TestAppContext,
) -> String {
    workspace.update(cx, |_ws, cx| {
        let task = Task::new("subtask-host".into(), "".into(), None);
        let id = task.id.clone();
        cx.update_global::<crate::agent::tasks_global::GlobalTasks, _>(|g, _| {
            g.add(task);
        });
        id
    })
}

#[gpui::test]
fn add_subtask_appends_and_skips_empty(cx: &mut TestAppContext) {
    let (_wh, workspace) = build_workspace(cx);
    let task_id = seed_backlog_task(&workspace, cx);

    workspace.update(cx, |ws, cx| {
        ws.add_subtask(&task_id, "first".into(), cx);
        ws.add_subtask(&task_id, "   ".into(), cx); // whitespace → no-op
        ws.add_subtask(&task_id, "  second  ".into(), cx); // trimmed

        let g = cx.global::<crate::agent::tasks_global::GlobalTasks>();
        let t = g.get(&task_id).unwrap();
        assert_eq!(t.subtasks.len(), 2);
        assert_eq!(t.subtasks[0].title, "first");
        assert_eq!(t.subtasks[1].title, "second");
        assert!(t.subtasks.iter().all(|s| s.source_session_id.is_none()));
    });
}

#[gpui::test]
fn toggle_subtask_round_trips(cx: &mut TestAppContext) {
    let (_wh, workspace) = build_workspace(cx);
    let task_id = seed_backlog_task(&workspace, cx);

    workspace.update(cx, |ws, cx| {
        ws.add_subtask(&task_id, "a".into(), cx);
        let sid = cx
            .global::<crate::agent::tasks_global::GlobalTasks>()
            .get(&task_id)
            .unwrap()
            .subtasks[0]
            .id
            .clone();
        ws.toggle_subtask(&task_id, &sid, cx);
        assert!(
            cx.global::<crate::agent::tasks_global::GlobalTasks>()
                .get(&task_id)
                .unwrap()
                .subtasks[0]
                .completed
        );
        ws.toggle_subtask(&task_id, &sid, cx);
        assert!(
            !cx.global::<crate::agent::tasks_global::GlobalTasks>()
                .get(&task_id)
                .unwrap()
                .subtasks[0]
                .completed
        );
    });
}

#[gpui::test]
fn delete_subtask_drops_only_matching_id(cx: &mut TestAppContext) {
    let (_wh, workspace) = build_workspace(cx);
    let task_id = seed_backlog_task(&workspace, cx);

    workspace.update(cx, |ws, cx| {
        ws.add_subtask(&task_id, "keep".into(), cx);
        ws.add_subtask(&task_id, "drop".into(), cx);
        let drop_id = cx
            .global::<crate::agent::tasks_global::GlobalTasks>()
            .get(&task_id)
            .unwrap()
            .subtasks[1]
            .id
            .clone();
        ws.delete_subtask(&task_id, &drop_id, cx);

        let g = cx.global::<crate::agent::tasks_global::GlobalTasks>();
        let t = g.get(&task_id).unwrap();
        assert_eq!(t.subtasks.len(), 1);
        assert_eq!(t.subtasks[0].title, "keep");
    });
}

#[gpui::test]
fn rename_subtask_updates_title_and_ignores_empty(cx: &mut TestAppContext) {
    let (_wh, workspace) = build_workspace(cx);
    let task_id = seed_backlog_task(&workspace, cx);

    workspace.update(cx, |ws, cx| {
        ws.add_subtask(&task_id, "old".into(), cx);
        let sid = cx
            .global::<crate::agent::tasks_global::GlobalTasks>()
            .get(&task_id)
            .unwrap()
            .subtasks[0]
            .id
            .clone();
        ws.rename_subtask(&task_id, &sid, "  new title  ".into(), cx);
        assert_eq!(
            cx.global::<crate::agent::tasks_global::GlobalTasks>()
                .get(&task_id)
                .unwrap()
                .subtasks[0]
                .title,
            "new title",
            "rename trims whitespace"
        );
        ws.rename_subtask(&task_id, &sid, "   ".into(), cx);
        assert_eq!(
            cx.global::<crate::agent::tasks_global::GlobalTasks>()
                .get(&task_id)
                .unwrap()
                .subtasks[0]
                .title,
            "new title",
            "empty rename is ignored"
        );
    });
}

// ---------------------------------------------------------------------------
// R-22 TodoWrite hook merge
// ---------------------------------------------------------------------------

/// Seed a `Running` task already attached to `session_id`. R-22 needs
/// the task↔session linkage in place so `apply_todo_write` can locate
/// the row to fold into. Returns the task id.
fn seed_running_task_with_subtasks(
    workspace: &gpui::Entity<crate::workspace::Workspace>,
    cx: &mut TestAppContext,
    session_id: &str,
) -> String {
    workspace.update(cx, |_ws, cx| {
        let mut task = Task::new("host".into(), "".into(), None);
        task.state = TaskState::Running {
            worktree_path: PathBuf::from("/tmp/wt"),
        };
        task.session_ids.push(session_id.to_string());
        let id = task.id.clone();
        cx.update_global::<crate::agent::tasks_global::GlobalTasks, _>(|g, _| {
            g.add(task);
        });
        id
    })
}

#[gpui::test]
fn apply_todo_write_pushes_new_auto_subtasks(cx: &mut TestAppContext) {
    let (_wh, workspace) = build_workspace(cx);
    let task_id = seed_running_task_with_subtasks(&workspace, cx, "sess-todo-1");

    workspace.update(cx, |ws, cx| {
        let payload = serde_json::json!({
            "todos": [
                { "content": "Inspect session.rs", "status": "in_progress" },
                { "content": "Add refresh logic", "status": "completed" },
            ]
        });
        ws.apply_todo_write("sess-todo-1", &payload, cx);

        let g = cx.global::<crate::agent::tasks_global::GlobalTasks>();
        let t = g.get(&task_id).unwrap();
        assert_eq!(t.subtasks.len(), 2);
        assert_eq!(t.subtasks[0].title, "Inspect session.rs");
        assert!(!t.subtasks[0].completed);
        assert_eq!(
            t.subtasks[0].source_session_id.as_deref(),
            Some("sess-todo-1"),
            "hook-injected items carry the session id (auto namespace)",
        );
        assert!(
            t.subtasks[1].completed,
            "status: completed → SubTask.completed = true",
        );
    });
}

#[gpui::test]
fn apply_todo_write_toggles_completed_on_repeat_emission(cx: &mut TestAppContext) {
    let (_wh, workspace) = build_workspace(cx);
    let task_id = seed_running_task_with_subtasks(&workspace, cx, "sess-todo-2");

    workspace.update(cx, |ws, cx| {
        let first = serde_json::json!({
            "todos": [{ "content": "step A", "status": "in_progress" }]
        });
        ws.apply_todo_write("sess-todo-2", &first, cx);
        let second = serde_json::json!({
            "todos": [{ "content": "step A", "status": "completed" }]
        });
        ws.apply_todo_write("sess-todo-2", &second, cx);

        let g = cx.global::<crate::agent::tasks_global::GlobalTasks>();
        let t = g.get(&task_id).unwrap();
        assert_eq!(t.subtasks.len(), 1, "second emission folds into row 1");
        assert!(t.subtasks[0].completed);
    });
}

#[gpui::test]
fn apply_todo_write_never_overwrites_manual_subtask(cx: &mut TestAppContext) {
    let (_wh, workspace) = build_workspace(cx);
    let task_id = seed_running_task_with_subtasks(&workspace, cx, "sess-todo-3");

    workspace.update(cx, |ws, cx| {
        // User typed an identical title manually first.
        ws.add_subtask(&task_id, "step A".into(), cx);
        let manual_id = cx
            .global::<crate::agent::tasks_global::GlobalTasks>()
            .get(&task_id)
            .unwrap()
            .subtasks[0]
            .id
            .clone();

        let payload = serde_json::json!({
            "todos": [{ "content": "step A", "status": "completed" }]
        });
        ws.apply_todo_write("sess-todo-3", &payload, cx);

        let g = cx.global::<crate::agent::tasks_global::GlobalTasks>();
        let t = g.get(&task_id).unwrap();
        assert_eq!(t.subtasks.len(), 2, "manual + auto coexist (I-14)");
        let manual = t.subtasks.iter().find(|s| s.id == manual_id).unwrap();
        assert!(
            !manual.completed,
            "manual subtask completion is left alone by the hook",
        );
        assert!(manual.source_session_id.is_none());
        let auto = t
            .subtasks
            .iter()
            .find(|s| s.source_session_id.as_deref() == Some("sess-todo-3"))
            .unwrap();
        assert!(auto.completed);
    });
}

#[gpui::test]
fn apply_todo_write_keeps_items_dropped_in_later_emission(cx: &mut TestAppContext) {
    let (_wh, workspace) = build_workspace(cx);
    let task_id = seed_running_task_with_subtasks(&workspace, cx, "sess-todo-4");

    workspace.update(cx, |ws, cx| {
        let first = serde_json::json!({
            "todos": [
                { "content": "alpha", "status": "in_progress" },
                { "content": "beta",  "status": "in_progress" },
            ]
        });
        ws.apply_todo_write("sess-todo-4", &first, cx);
        // Second emission drops "beta" — no stale marking, the row stays put.
        let second = serde_json::json!({
            "todos": [{ "content": "alpha", "status": "completed" }]
        });
        ws.apply_todo_write("sess-todo-4", &second, cx);

        let g = cx.global::<crate::agent::tasks_global::GlobalTasks>();
        let t = g.get(&task_id).unwrap();
        assert_eq!(t.subtasks.len(), 2, "dropped item is preserved");
        assert!(t.subtasks.iter().any(|s| s.title == "beta"));
    });
}

#[gpui::test]
fn apply_todo_write_isolates_sessions_on_same_task(cx: &mut TestAppContext) {
    let (_wh, workspace) = build_workspace(cx);
    let task_id = workspace.update(cx, |_ws, cx| {
        let mut task = Task::new("host".into(), "".into(), None);
        task.state = TaskState::Running {
            worktree_path: PathBuf::from("/tmp/wt"),
        };
        task.session_ids.push("sess-A".to_string());
        task.session_ids.push("sess-B".to_string());
        let id = task.id.clone();
        cx.update_global::<crate::agent::tasks_global::GlobalTasks, _>(|g, _| {
            g.add(task);
        });
        id
    });

    workspace.update(cx, |ws, cx| {
        let payload = serde_json::json!({
            "todos": [{ "content": "shared title", "status": "in_progress" }]
        });
        ws.apply_todo_write("sess-A", &payload, cx);
        // Same title from a different session → must push a *new* row,
        // not fold into the sess-A entry.
        ws.apply_todo_write("sess-B", &payload, cx);

        let g = cx.global::<crate::agent::tasks_global::GlobalTasks>();
        let t = g.get(&task_id).unwrap();
        assert_eq!(t.subtasks.len(), 2);
        let owners: Vec<&str> = t
            .subtasks
            .iter()
            .map(|s| s.source_session_id.as_deref().unwrap_or(""))
            .collect();
        assert!(owners.contains(&"sess-A"));
        assert!(owners.contains(&"sess-B"));
    });
}

#[gpui::test]
fn apply_todo_write_silently_skips_unparseable_payload(cx: &mut TestAppContext) {
    let (_wh, workspace) = build_workspace(cx);
    let task_id = seed_running_task_with_subtasks(&workspace, cx, "sess-todo-bad");

    workspace.update(cx, |ws, cx| {
        // `todos` is a string instead of an array — schema drift.
        let payload = serde_json::json!({ "todos": "not-an-array" });
        ws.apply_todo_write("sess-todo-bad", &payload, cx);

        let g = cx.global::<crate::agent::tasks_global::GlobalTasks>();
        let t = g.get(&task_id).unwrap();
        assert!(t.subtasks.is_empty(), "parse failure must not mutate state");
    });
}

#[gpui::test]
fn apply_todo_write_no_matching_session_is_noop(cx: &mut TestAppContext) {
    let (_wh, workspace) = build_workspace(cx);
    let task_id = seed_running_task_with_subtasks(&workspace, cx, "sess-known");

    workspace.update(cx, |ws, cx| {
        let payload = serde_json::json!({
            "todos": [{ "content": "x", "status": "completed" }]
        });
        ws.apply_todo_write("sess-unknown", &payload, cx);

        let g = cx.global::<crate::agent::tasks_global::GlobalTasks>();
        let t = g.get(&task_id).unwrap();
        assert!(
            t.subtasks.is_empty(),
            "no task owns sess-unknown, so the merge is a no-op",
        );
    });
}
