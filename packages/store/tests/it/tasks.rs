//! Task model, persistence, branch naming and prompt files.

use std::path::PathBuf;

use chrono::Utc;

use daruda_store::project::ProjectUuid;
use daruda_store::tasks::branch::{branch_name_for, random_branch_name};
use daruda_store::tasks::persistence::{load_tasks_in, tasks_path_in};
use daruda_store::tasks::prompt_file::{
    PROMPT_DIR_NAME, build_claude_command, existing_prompt_file_path, prompt_file_path,
    render_task_prompt, task_prompt_body, write_prompt_file,
};

use daruda_store::tasks::task::{
    AgentType, SCHEMA_VERSION, SessionEndReason, SubTask, Task, TaskAgentSurface, TaskFilter,
    TaskRunIn, TaskScope, TaskState, TasksState,
};

/// `task` as the task log holds it.
fn record_of(task: &Task) -> daruda_store::tasks::TaskRecord {
    let state = TasksState {
        tasks: vec![task.clone()],
        ..TasksState::default()
    };
    let daruda_store::tasks::TaskEvent::Imported { mut tasks } =
        daruda_store::tasks::migrate_legacy(&state)
    else {
        panic!("migration yields an import");
    };
    tasks.remove(0)
}

fn sample_task() -> Task {
    Task::new(
        ProjectUuid::new(),
        "Fix auth bug".to_string(),
        "Login flow drops the token on refresh.".to_string(),
        Some(PathBuf::from("/repo/main")),
    )
}

fn write_legacy_input(data_dir: &std::path::Path, rows: &TasksState) -> std::io::Result<()> {
    std::fs::write(
        tasks_path_in(data_dir),
        serde_json::to_vec_pretty(rows).unwrap(),
    )
}

#[test]
fn run_in_round_trips_and_legacy_tasks_create_a_worktree() {
    let mut task = sample_task();
    let mut legacy = serde_json::to_value(&task).unwrap();
    legacy.as_object_mut().unwrap().remove("run_in");
    assert_eq!(
        serde_json::from_value::<Task>(legacy).unwrap().run_in,
        TaskRunIn::NewWorktree
    );
    task.run_in = TaskRunIn::ExistingLane {
        path: PathBuf::from("/repo/main"),
    };
    let json = serde_json::to_string(&task).unwrap();
    assert_eq!(serde_json::from_str::<Task>(&json).unwrap(), task);
}

#[test]
fn execution_identity_round_trips_and_legacy_tasks_have_none() {
    let mut task = sample_task();
    let mut legacy = serde_json::to_value(&task).unwrap();
    legacy.as_object_mut().unwrap().remove("execution");
    assert!(
        serde_json::from_value::<Task>(legacy)
            .unwrap()
            .execution
            .is_none()
    );
    task.execution = Some(daruda_store::tasks::TaskExecution {
        source: Default::default(),
        id: "run-1".into(),
        agent_id: "claude".into(),
        account_id: Some(daruda_store::accounts::AccountId(uuid::Uuid::new_v4())),
        cwd: PathBuf::from("/repo/task"),
        session_id: Some("session-1".into()),
    });
    let restored: Task = serde_json::from_str(&serde_json::to_string(&task).unwrap()).unwrap();
    assert_eq!(restored.execution, task.execution);
}

// ---------------------------------------------------------------------------
// Task::new + ULID monotonicity
// ---------------------------------------------------------------------------

#[test]
fn task_new_assigns_ulid_and_backlog_state() {
    let t = sample_task();
    assert!(!t.id.is_empty());
    assert!(matches!(t.state, TaskState::Backlog));
    assert!(t.session_ids.is_empty());
    assert!(t.finished_at.is_none());
    assert!(t.notes.is_empty());
    assert!(t.auto_execute);
    assert_eq!(t.agent_type, AgentType::Claude);
    assert_eq!(t.created_at, t.updated_at);
}

#[test]
fn task_new_branch_name_ignores_the_title() {
    let t = sample_task();
    assert_eq!(t.branch_name, branch_name_for(&t.id));
}

#[test]
fn ulid_burst_has_no_collisions() {
    // Within the same millisecond ULID's random tail is uncorrelated,
    // so back-to-back IDs are NOT strictly monotonic. Uniqueness IS
    // guaranteed by the 80-bit random suffix — this test asserts the
    // contract daruda actually depends on.
    let n = 256;
    let mut ids: Vec<String> = (0..n).map(|_| ulid::Ulid::generate().to_string()).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), n, "ULIDs must not collide in a burst");
}

// ---------------------------------------------------------------------------
// TaskState helpers
// ---------------------------------------------------------------------------

#[test]
fn task_state_is_terminal_matrix() {
    let wt = PathBuf::from("/repo/wt");
    assert!(!TaskState::Backlog.is_terminal());
    assert!(
        !TaskState::Running {
            worktree_path: wt.clone()
        }
        .is_terminal()
    );
    assert!(
        TaskState::Done {
            worktree_path: wt.clone(),
            end_reason: SessionEndReason::Stop,
        }
        .is_terminal()
    );
    assert!(
        TaskState::Error {
            worktree_path: wt.clone(),
            message: "boom".into(),
        }
        .is_terminal()
    );
    assert!(TaskState::Cancelled { worktree_path: wt }.is_terminal());
}

#[test]
fn task_state_worktree_path_is_some_for_non_backlog() {
    let wt = PathBuf::from("/repo/wt");
    assert_eq!(TaskState::Backlog.worktree_path(), None);
    assert_eq!(
        TaskState::Running {
            worktree_path: wt.clone()
        }
        .worktree_path(),
        Some(&wt),
    );
    assert_eq!(
        TaskState::Done {
            worktree_path: wt.clone(),
            end_reason: SessionEndReason::Logout,
        }
        .worktree_path(),
        Some(&wt),
    );
    assert_eq!(
        TaskState::Error {
            worktree_path: wt.clone(),
            message: "x".into(),
        }
        .worktree_path(),
        Some(&wt),
    );
    assert_eq!(
        TaskState::Cancelled {
            worktree_path: wt.clone()
        }
        .worktree_path(),
        Some(&wt),
    );
}

// ---------------------------------------------------------------------------
// Serialization round-trips
// ---------------------------------------------------------------------------

#[test]
fn task_state_serde_round_trip_all_variants() {
    let wt = PathBuf::from("/repo/wt");
    let cases = vec![
        TaskState::Backlog,
        TaskState::Running {
            worktree_path: wt.clone(),
        },
        TaskState::Done {
            worktree_path: wt.clone(),
            end_reason: SessionEndReason::Stop,
        },
        TaskState::Done {
            worktree_path: wt.clone(),
            end_reason: SessionEndReason::PromptInputExit,
        },
        TaskState::Done {
            worktree_path: wt.clone(),
            end_reason: SessionEndReason::Logout,
        },
        TaskState::Done {
            worktree_path: wt.clone(),
            end_reason: SessionEndReason::Other,
        },
        TaskState::Error {
            worktree_path: wt.clone(),
            message: "exit nonzero".into(),
        },
        TaskState::Cancelled { worktree_path: wt },
    ];
    for case in cases {
        let json = serde_json::to_string(&case).expect("serialize");
        let back: TaskState = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(case, back, "round-trip mismatch for {json}");
    }
}

#[test]
fn session_end_reason_serializes_as_snake_case() {
    let reasons = [
        (SessionEndReason::Stop, "\"stop\""),
        (SessionEndReason::PromptInputExit, "\"prompt_input_exit\""),
        (SessionEndReason::Logout, "\"logout\""),
        (SessionEndReason::Other, "\"other\""),
        (SessionEndReason::Error, "\"error\""),
    ];
    for (reason, expected) in reasons {
        let json = serde_json::to_string(&reason).expect("serialize");
        assert_eq!(json, expected);
    }
}

#[test]
fn task_serde_round_trip_preserves_all_fields() {
    let mut t = sample_task();
    t.notes = "some notes".into();
    t.session_ids = vec!["abc12345".into(), "def67890".into()];
    t.finished_at = Some(Utc::now());
    t.auto_execute = false;
    t.agent_surface = TaskAgentSurface::AgentChat;
    t.state = TaskState::Done {
        worktree_path: PathBuf::from("/repo/main-fix-auth"),
        end_reason: SessionEndReason::Stop,
    };

    let json = serde_json::to_string_pretty(&t).expect("serialize");
    let back: Task = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(t, back);
}

#[test]
fn task_new_defaults_agent_surface_to_agent_chat() {
    assert_eq!(TaskAgentSurface::default(), TaskAgentSurface::AgentChat);
    assert_eq!(sample_task().agent_surface, TaskAgentSurface::AgentChat);
}

#[test]
fn task_serde_preserves_explicit_terminal_surface() {
    let mut task = sample_task();
    task.agent_surface = TaskAgentSurface::Terminal;
    let json = serde_json::to_string(&task).expect("serialize");
    let restored: Task = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(restored.agent_surface, TaskAgentSurface::Terminal);
}

#[test]
fn agent_surface_serializes_as_snake_case() {
    let json = serde_json::to_string(&TaskAgentSurface::AgentChat).expect("serialize");
    assert_eq!(json, "\"agent_chat\"");
    let back: TaskAgentSurface = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back, TaskAgentSurface::AgentChat);
}

#[test]
fn task_loads_with_default_for_new_optional_fields() {
    // JSON missing session_ids / notes / finished_at / agent_type / agent_surface /
    // base_worktree_path / auto_execute / subtasks — must default cleanly.
    let json = r#"{
        "id": "01HX",
        "project": "6f1c2b8e-0d4a-4f7e-9a51-3b2c1d0e9f8a",
        "title": "X",
        "prompt": "hi",
        "state": { "state": "backlog" },
        "created_at": "2026-05-08T00:00:00Z",
        "updated_at": "2026-05-08T00:00:00Z",
        "branch_name": "x-01hx"
    }"#;
    let t: Task = serde_json::from_str(json).expect("deserialize legacy task");
    assert!(t.session_ids.is_empty());
    assert!(t.finished_at.is_none());
    assert!(t.notes.is_empty());
    assert!(t.base_worktree_path.is_none());
    assert_eq!(t.agent_type, AgentType::Claude);
    assert_eq!(
        t.agent_surface,
        TaskAgentSurface::Terminal,
        "legacy tasks without agent_surface retain Terminal"
    );
    assert!(t.auto_execute, "auto_execute default is true");
    assert!(t.subtasks.is_empty(), "subtasks defaults to empty vec");
}

// ---------------------------------------------------------------------------
// SubTask
// ---------------------------------------------------------------------------

#[test]
fn subtask_new_stamps_ulid_and_defaults() {
    let s = SubTask::new("write tests".into());
    assert!(!s.id.is_empty(), "subtask id is a ULID");
    assert_eq!(s.title, "write tests");
    assert!(!s.completed);
    assert!(s.created_at.is_some());
    assert!(
        s.source_session_id.is_none(),
        "manually-added subtask carries no source_session_id"
    );
}

#[test]
fn task_subtask_progress_counts_completed() {
    let mut t = sample_task();
    assert_eq!(t.subtask_progress(), (0, 0));
    t.subtasks.push(SubTask::new("a".into()));
    let mut done = SubTask::new("b".into());
    done.completed = true;
    t.subtasks.push(done);
    t.subtasks.push(SubTask::new("c".into()));
    assert_eq!(t.subtask_progress(), (1, 3));
}

#[test]
fn legacy_input_preserves_subtask_identity_and_origin() {
    use tempfile::tempdir;
    let dir = tempdir().unwrap();
    let mut state = TasksState::default();

    let mut t = sample_task();
    let auto = SubTask {
        id: "01J0AUTO".into(),
        title: "Inspect session.rs".into(),
        completed: true,
        created_at: Some(Utc::now()),
        source_session_id: Some("sess_abc".into()),
    };
    let manual = SubTask::new("Add refresh logic".into());
    let manual_id = manual.id.clone();
    t.subtasks.push(auto.clone());
    t.subtasks.push(manual.clone());
    let task_id = t.id.clone();
    state.add(t);

    write_legacy_input(dir.path(), &state).expect("save");
    let loaded = load_tasks_in(dir.path()).expect("load");
    let back = loaded.get(&task_id).expect("task survives round-trip");
    assert_eq!(back.subtasks.len(), 2);
    assert_eq!(back.subtasks[0], auto, "auto subtask preserved verbatim");
    assert_eq!(back.subtasks[1].id, manual_id);
    assert_eq!(
        back.subtasks[1].source_session_id, None,
        "manual subtask stays manual (namespace separation)"
    );
}

#[test]
fn legacy_task_json_without_subtasks_loads_with_empty_vec() {
    // A `tasks.json` saved before this field existed has no `subtasks`
    // key — the loader must surface the task with an empty vec, not
    // refuse the file. Otherwise users would lose their entire Backlog
    // on upgrade.
    let json = format!(
        r#"{{
            "schema_version": {SCHEMA_VERSION},
            "tasks": [{{
                "id": "01HX",
                "project": "6f1c2b8e-0d4a-4f7e-9a51-3b2c1d0e9f8a",
                "title": "Legacy",
                "prompt": "",
                "state": {{ "state": "backlog" }},
                "created_at": "2026-05-08T00:00:00Z",
                "updated_at": "2026-05-08T00:00:00Z",
                "branch_name": "legacy-01hx"
            }}]
        }}"#
    );
    let state: TasksState = serde_json::from_str(&json).expect("deserialize legacy file");
    assert_eq!(state.tasks.len(), 1);
    assert!(state.tasks[0].subtasks.is_empty());
}

// ---------------------------------------------------------------------------
// TasksState helpers
// ---------------------------------------------------------------------------

#[test]
fn tasks_state_add_get_remove() {
    let mut state = TasksState::default();
    let t = sample_task();
    let id = t.id.clone();
    state.add(t);
    assert!(state.get(&id).is_some());

    state.get_mut(&id).unwrap().notes = "hello".into();
    assert_eq!(state.get(&id).unwrap().notes, "hello");

    state.remove(&id);
    assert!(state.get(&id).is_none());
}

#[test]
fn tasks_state_default_uses_current_schema_version() {
    let s = TasksState::default();
    assert_eq!(s.schema_version, SCHEMA_VERSION);
    assert!(s.tasks.is_empty());
}

// ---------------------------------------------------------------------------
// TaskFilter matrix (9 lifecycle states × 6 filters)
// ---------------------------------------------------------------------------

#[test]
fn task_filter_matrix_matches_each_pair() {
    use daruda_store::tasks::ExecutionRef;
    use daruda_store::tasks::machine::{
        ErrorCause, InterruptCause, Lifecycle, StartFailure, UndeliveredReason,
    };
    let wt = PathBuf::from("/repo/wt");
    let exec = ExecutionRef {
        task_id: "t".into(),
        execution_id: "r".into(),
    };
    let states = [
        ("backlog", Lifecycle::Backlog),
        (
            "preparing",
            Lifecycle::Preparing {
                exec: exec.clone(),
                source: Default::default(),
            },
        ),
        (
            "dispatching",
            Lifecycle::Dispatching {
                exec: exec.clone(),
                worktree: wt.clone(),
            },
        ),
        (
            "running",
            Lifecycle::Running {
                exec: exec.clone(),
                worktree: wt.clone(),
            },
        ),
        (
            "done",
            Lifecycle::Done {
                exec: exec.clone(),
                worktree: wt.clone(),
                end_reason: SessionEndReason::Stop,
            },
        ),
        (
            "error",
            Lifecycle::Error {
                exec: Some(exec.clone()),
                worktree: None,
                cause: ErrorCause::StartFailed(StartFailure::GitAddFailed),
            },
        ),
        (
            "cancelled",
            Lifecycle::Cancelled {
                exec: exec.clone(),
                worktree: None,
            },
        ),
        (
            "undelivered",
            Lifecycle::Undelivered {
                exec: exec.clone(),
                worktree: None,
                reason: UndeliveredReason::PaneClosed,
            },
        ),
        (
            "interrupted",
            Lifecycle::Interrupted {
                exec: Some(exec),
                worktree: wt,
                cause: InterruptCause::UserStop,
            },
        ),
    ];

    // Each filter's states; every other state is excluded.
    let cases: [(TaskFilter, &[&str]); 6] = [
        (
            TaskFilter::All,
            &[
                "backlog",
                "preparing",
                "dispatching",
                "running",
                "done",
                "error",
                "cancelled",
                "undelivered",
                "interrupted",
            ],
        ),
        (TaskFilter::Backlog, &["backlog"]),
        (
            TaskFilter::Running,
            &["preparing", "dispatching", "running"],
        ),
        (TaskFilter::Done, &["done"]),
        (TaskFilter::Failed, &["error", "undelivered"]),
        (TaskFilter::Cancelled, &["cancelled", "interrupted"]),
    ];

    for (filter, included) in cases {
        for (label, state) in &states {
            let want = included.contains(label);
            assert_eq!(
                filter.matches(state),
                want,
                "filter={filter:?} state={label} expected={want}",
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Branch derivation
// ---------------------------------------------------------------------------

#[test]
fn branch_name_is_task_plus_the_lowercased_ulid_tail() {
    assert_eq!(branch_name_for("01HXABCDEFGHIJ"), "task-cdefghij");
}

#[test]
fn branch_names_differ_for_ids_from_the_same_millisecond() {
    let a = branch_name_for("01J00000000000000000000001");
    let b = branch_name_for("01J00000000000000000000002");
    assert_ne!(a, b);
    assert!(daruda_core::git::validate_branch_name(&a).is_ok());
}

#[test]
fn random_branch_names_are_valid_and_fresh() {
    let a = random_branch_name();
    assert!(a.starts_with("task-"));
    assert!(daruda_core::git::validate_branch_name(&a).is_ok());
    assert_ne!(a, random_branch_name());
}

// ---------------------------------------------------------------------------
// Prompt file + claude command
// ---------------------------------------------------------------------------

/// Named by id alone, so neither a branch shaped `task-…` nor a Retry that
/// moves the task into its own lane changes the name.
#[test]
fn prompt_file_is_named_by_task_id_wherever_the_task_runs() {
    let mut t = sample_task();
    let wt = PathBuf::from("/repo/wt");
    let expected = wt.join(PROMPT_DIR_NAME).join(format!("task-{}.md", t.id));
    assert_eq!(prompt_file_path(&record_of(&t).spec, &wt), expected);
    t.run_in = TaskRunIn::ExistingLane { path: wt.clone() };
    assert_eq!(prompt_file_path(&record_of(&t).spec, &wt), expected);
}

#[test]
fn reading_falls_back_to_a_branch_named_file_only_when_it_is_the_one_on_disk() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let t = sample_task();
    let by_id = prompt_file_path(&record_of(&t).spec, tmp.path());
    let by_branch = tmp
        .path()
        .join(PROMPT_DIR_NAME)
        .join(format!("task-{}.md", t.branch_name));
    assert_eq!(
        existing_prompt_file_path(&record_of(&t).spec, tmp.path()),
        by_id
    );
    write_prompt_file(&by_branch, "older build").expect("write");
    assert_eq!(
        existing_prompt_file_path(&record_of(&t).spec, tmp.path()),
        by_branch
    );
    write_prompt_file(&by_id, "current").expect("write");
    assert_eq!(
        existing_prompt_file_path(&record_of(&t).spec, tmp.path()),
        by_id
    );
}

#[test]
fn write_prompt_file_creates_dir_and_writes_contents() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = prompt_file_path(&record_of(&sample_task()).spec, tmp.path());
    write_prompt_file(&path, "hello world").expect("write succeeds");
    let contents = std::fs::read_to_string(&path).expect("read");
    assert_eq!(contents, "hello world");
}

#[test]
fn write_prompt_file_overwrites_existing_file() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = prompt_file_path(&record_of(&sample_task()).spec, tmp.path());
    write_prompt_file(&path, "first").expect("write 1");
    write_prompt_file(&path, "second").expect("write 2");
    let contents = std::fs::read_to_string(&path).expect("read");
    assert_eq!(contents, "second");
}

#[test]
fn build_claude_command_appends_newline_when_auto_execute() {
    let cmd = build_claude_command(&PathBuf::from("/tmp/p.md"), true);
    assert!(cmd.starts_with("claude --dangerously-skip-permissions \"$(cat '/tmp/p.md')\""));
    assert!(cmd.ends_with('\n'));
}

#[test]
fn build_claude_command_skips_newline_when_manual() {
    let cmd = build_claude_command(&PathBuf::from("/tmp/p.md"), false);
    assert_eq!(
        cmd,
        "claude --dangerously-skip-permissions \"$(cat '/tmp/p.md')\""
    );
}

#[test]
fn build_claude_command_escapes_single_quotes_in_path() {
    let cmd = build_claude_command(&PathBuf::from("/Users/al'fred/p.md"), false);
    // POSIX-safe escape of `'` inside a single-quoted string.
    assert!(
        cmd.contains("'/Users/al'\\''fred/p.md'"),
        "single-quote escape missing in {cmd}",
    );
}

#[test]
fn render_task_prompt_wraps_user_prompt_with_metadata() {
    let t = sample_task();
    let out = render_task_prompt(&record_of(&t), "feature-x");
    assert!(out.starts_with("Task: \"Fix auth bug\" (feature-x)"));
    assert!(out.contains("Status: Backlog"));
    assert!(out.contains(&t.prompt));
    assert!(out.contains("final answer is recorded as the task's result"));
}

/// Lane agents have no tool that updates a task, so the prompt must not ask
/// them to; the final answer is what gets recorded instead.
#[test]
fn render_task_prompt_does_not_ask_the_agent_to_update_the_task() {
    let t = sample_task();
    let out = render_task_prompt(&record_of(&t), "feature-x");
    assert!(!out.contains("update task"), "{out}");
    assert!(!out.contains(&format!("\"{}\"", t.id)), "{out}");
}

/// The prompt file holds the framed prompt; reading it back must yield the
/// user's text alone, or a reload frames it a second time on the next run.
#[test]
fn task_prompt_body_inverts_render_task_prompt() {
    for prompt in [
        "Fix the bug.",
        "line one\n\nline two\n",
        "\nleading blank",
        "",
    ] {
        let mut t = sample_task();
        t.prompt = prompt.to_string();
        let out = render_task_prompt(&record_of(&t), "feature-x");
        assert_eq!(task_prompt_body(&out), prompt, "{out}");
        assert_eq!(task_prompt_body(out.trim_end_matches('\n')), prompt);
    }
}

#[test]
fn task_prompt_body_keeps_text_without_a_frame() {
    assert_eq!(task_prompt_body("External update"), "External update");
    assert_eq!(
        task_prompt_body("Task: \"T\" (b)\nStatus: Running\n\nEdited body"),
        "Edited body",
        "a removed closing instruction keeps the body after the header"
    );
}

/// A hand edit that trims the header or the gap before the closing
/// instruction must still leave the frame out of the prompt.
#[test]
fn task_prompt_body_drops_a_hand_trimmed_frame() {
    let mut t = sample_task();
    t.prompt = "Body".to_string();
    let out = render_task_prompt(&record_of(&t), "feature-x");
    let (header, rest) = out.split_once("\n\n").unwrap();
    let (task_line, status_line) = header.split_once('\n').unwrap();
    let without_status = format!("{task_line}\n\n{rest}");
    let without_gap = format!("{header}\n{rest}");
    let closing = rest.strip_prefix("Body\n\n").unwrap();
    let without_closing_gap = format!("{header}\n\nBody\n{closing}");
    for edited in [without_status, without_gap, without_closing_gap] {
        assert_eq!(task_prompt_body(&edited), "Body", "{edited:?}");
    }
    assert!(status_line.starts_with("Status: "));
}

// ---------------------------------------------------------------------------
// Persistence round-trip + schema-version rejection
// ---------------------------------------------------------------------------

#[test]
fn legacy_input_preserves_all_task_fields() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut state = TasksState::default();
    state.add(sample_task());
    state.add({
        let mut t = sample_task();
        t.state = TaskState::Done {
            worktree_path: PathBuf::from("/wt"),
            end_reason: SessionEndReason::Logout,
        };
        t
    });

    write_legacy_input(tmp.path(), &state).expect("save");
    let loaded = load_tasks_in(tmp.path()).expect("load");
    assert_eq!(loaded, state);
    assert!(tasks_path_in(tmp.path()).exists());
}

#[test]
fn load_returns_none_when_file_missing() {
    let tmp = tempfile::tempdir().expect("tempdir");
    assert!(load_tasks_in(tmp.path()).is_none());
}

#[test]
fn load_rejects_higher_schema_version() {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(tmp.path()).unwrap();
    // Write a tasks.json claiming a future schema version.
    let json = serde_json::json!({
        "schema_version": SCHEMA_VERSION + 1,
        "tasks": [],
    });
    std::fs::write(
        tasks_path_in(tmp.path()),
        serde_json::to_string(&json).unwrap(),
    )
    .unwrap();

    assert!(
        load_tasks_in(tmp.path()).is_none(),
        "loader must refuse newer schema"
    );
}

/// v1 rows name no project, so they cannot be kept — the file loads as an
/// empty current-version list instead of being treated as corrupt.
#[test]
fn load_drops_tasks_from_before_project_scoping() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let json = serde_json::json!({
        "schema_version": 1,
        "tasks": [{
            "id": "01HX",
            "title": "Unscoped",
            "prompt": "",
            "state": { "state": "backlog" },
            "created_at": "2026-05-08T00:00:00Z",
            "updated_at": "2026-05-08T00:00:00Z",
            "branch_name": "task-01hx"
        }],
    });
    std::fs::write(
        tasks_path_in(tmp.path()),
        serde_json::to_string(&json).unwrap(),
    )
    .unwrap();

    assert_eq!(load_tasks_in(tmp.path()), Some(TasksState::default()));
}

#[test]
fn legacy_input_preserves_the_project() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let task = sample_task();
    let project = task.project;
    let mut state = TasksState::default();
    state.add(task);
    write_legacy_input(tmp.path(), &state).expect("save");
    let loaded = load_tasks_in(tmp.path()).expect("load");
    assert_eq!(loaded.tasks[0].project, project);
}

#[test]
fn loading_legacy_input_never_creates_a_missing_directory() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let nested = tmp.path().join("nested").join("daruda");
    assert!(load_tasks_in(&nested).is_none());
    assert!(!nested.exists());
}

#[test]
fn task_scope_keeps_the_active_projects_tasks_or_every_task() {
    let task = sample_task();
    let other = ProjectUuid::new();
    assert!(TaskScope::ActiveProject.matches(task.project, Some(task.project)));
    assert!(!TaskScope::ActiveProject.matches(task.project, Some(other)));
    assert!(!TaskScope::ActiveProject.matches(task.project, None));
    assert!(TaskScope::AllProjects.matches(task.project, Some(other)));
    assert!(TaskScope::AllProjects.matches(task.project, None));
    assert!(TaskScope::Project(task.project).matches(task.project, Some(other)));
    assert!(TaskScope::Project(task.project).matches(task.project, None));
    assert!(!TaskScope::Project(other).matches(task.project, Some(task.project)));
    assert_eq!(TaskScope::ActiveProject.project(Some(other)), Some(other));
    assert_eq!(
        TaskScope::Project(task.project).project(Some(other)),
        Some(task.project)
    );
    assert_eq!(TaskScope::AllProjects.project(Some(other)), None);
}
