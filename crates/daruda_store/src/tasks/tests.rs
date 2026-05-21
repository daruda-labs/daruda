//! Unit tests for `daruda_tasks`. Coverage target ≥ 21 cases mirrors
//! `daruda_panels::tests` so persistence regressions surface fast.

use std::path::PathBuf;

use chrono::Utc;

use super::branch::derive_branch_name;
use super::persistence::{load_tasks_in, save_tasks_in, tasks_path_in};
use super::prompt_file::{
    PROMPT_DIR_NAME, build_claude_command, render_task_prompt, task_prompt_file_path,
    write_prompt_file,
};
use super::sanitize::sanitize_branch_name;
use super::task::{
    AgentType, SCHEMA_VERSION, SessionEndReason, SubTask, Task, TaskFilter, TaskState, TasksState,
};

fn sample_task() -> Task {
    Task::new(
        "Fix auth bug".to_string(),
        "Login flow drops the token on refresh.".to_string(),
        Some(PathBuf::from("/repo/main")),
    )
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
fn task_new_branch_name_uses_derive_branch_name() {
    let t = sample_task();
    let expected = derive_branch_name("Fix auth bug", &t.id);
    assert_eq!(t.branch_name, expected);
}

#[test]
fn ulid_burst_has_no_collisions() {
    // Within the same millisecond ULID's random tail is uncorrelated,
    // so back-to-back IDs are NOT strictly monotonic. Uniqueness IS
    // guaranteed by the 80-bit random suffix — this test asserts the
    // contract daruda actually depends on.
    let n = 256;
    let mut ids: Vec<String> = (0..n).map(|_| ulid::Ulid::new().to_string()).collect();
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
    t.state = TaskState::Done {
        worktree_path: PathBuf::from("/repo/main-fix-auth"),
        end_reason: SessionEndReason::Stop,
    };

    let json = serde_json::to_string_pretty(&t).expect("serialize");
    let back: Task = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(t, back);
}

#[test]
fn task_loads_with_default_for_new_optional_fields() {
    // JSON missing session_ids / notes / finished_at / agent_type /
    // base_worktree_path / auto_execute / subtasks — must default cleanly.
    let json = r#"{
        "id": "01HX",
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
    assert!(t.auto_execute, "auto_execute default is true");
    assert!(t.subtasks.is_empty(), "subtasks defaults to empty vec");
}

// ---------------------------------------------------------------------------
// SubTask (R-21)
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
fn task_with_subtasks_round_trips_through_disk() {
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

    save_tasks_in(dir.path(), &state).expect("save");
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
    // A `tasks.json` saved before R-21 has no `subtasks` key — the
    // loader must surface the task with an empty vec, not refuse the
    // file. Otherwise users would lose their entire Backlog on upgrade.
    let json = format!(
        r#"{{
            "schema_version": {SCHEMA_VERSION},
            "tasks": [{{
                "id": "01HX",
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
// TaskFilter matrix (5 states × 4 filters)
// ---------------------------------------------------------------------------

#[test]
fn task_filter_matrix_matches_each_pair() {
    let wt = PathBuf::from("/repo/wt");
    let states = [
        ("backlog", TaskState::Backlog),
        (
            "running",
            TaskState::Running {
                worktree_path: wt.clone(),
            },
        ),
        (
            "done",
            TaskState::Done {
                worktree_path: wt.clone(),
                end_reason: SessionEndReason::Stop,
            },
        ),
        (
            "error",
            TaskState::Error {
                worktree_path: wt.clone(),
                message: "x".into(),
            },
        ),
        ("cancelled", TaskState::Cancelled { worktree_path: wt }),
    ];

    // (filter, state_label) -> expected match
    let cases = [
        (TaskFilter::All, "backlog", true),
        (TaskFilter::All, "running", true),
        (TaskFilter::All, "done", true),
        (TaskFilter::All, "error", true),
        (TaskFilter::All, "cancelled", true),
        (TaskFilter::Backlog, "backlog", true),
        (TaskFilter::Backlog, "running", false),
        (TaskFilter::Backlog, "done", false),
        (TaskFilter::Backlog, "error", false),
        (TaskFilter::Backlog, "cancelled", false),
        (TaskFilter::Running, "backlog", false),
        (TaskFilter::Running, "running", true),
        (TaskFilter::Running, "done", false),
        (TaskFilter::Running, "error", false),
        (TaskFilter::Running, "cancelled", false),
        (TaskFilter::Done, "backlog", false),
        (TaskFilter::Done, "running", false),
        (TaskFilter::Done, "done", true),
        (TaskFilter::Done, "error", true),
        (TaskFilter::Done, "cancelled", true),
    ];

    for (filter, label, want) in cases {
        let state = &states.iter().find(|(l, _)| *l == label).unwrap().1;
        assert_eq!(
            filter.matches(state),
            want,
            "filter={:?} state={label} expected={want}",
            filter,
        );
    }
}

#[test]
fn tasks_state_filter_by_state_returns_matching_subset() {
    let mut state = TasksState::default();
    let mut t1 = sample_task();
    t1.title = "a".into();
    t1.state = TaskState::Backlog;
    let mut t2 = sample_task();
    t2.title = "b".into();
    t2.state = TaskState::Running {
        worktree_path: PathBuf::from("/wt"),
    };
    let mut t3 = sample_task();
    t3.title = "c".into();
    t3.state = TaskState::Done {
        worktree_path: PathBuf::from("/wt"),
        end_reason: SessionEndReason::Stop,
    };
    state.add(t1);
    state.add(t2);
    state.add(t3);

    let backlog: Vec<_> = state
        .filter_by_state(TaskFilter::Backlog)
        .map(|t| t.title.clone())
        .collect();
    assert_eq!(backlog, vec!["a"]);

    let running: Vec<_> = state
        .filter_by_state(TaskFilter::Running)
        .map(|t| t.title.clone())
        .collect();
    assert_eq!(running, vec!["b"]);

    let terminal: Vec<_> = state
        .filter_by_state(TaskFilter::Done)
        .map(|t| t.title.clone())
        .collect();
    assert_eq!(terminal, vec!["c"]);

    let all: Vec<_> = state
        .filter_by_state(TaskFilter::All)
        .map(|t| t.title.clone())
        .collect();
    assert_eq!(all, vec!["a", "b", "c"]);
}

// ---------------------------------------------------------------------------
// Branch derivation
// ---------------------------------------------------------------------------

#[test]
fn derive_branch_name_falls_back_when_title_has_spaces() {
    // sanitize_branch_name rejects spaces outright (no auto-kebab),
    // so a title with spaces takes the fallback path.
    let b = derive_branch_name("Fix auth bug", "01HX9YZAAA000000000000");
    assert_eq!(b, "task-01hx9yza", "spaces in title trip sanitize fallback");
}

#[test]
fn derive_branch_name_keeps_valid_kebab() {
    let b = derive_branch_name("fix-auth-bug", "01HX9YZAAA");
    assert_eq!(b, "fix-auth-bug-01hx");
}

#[test]
fn derive_branch_name_truncates_long_titles_to_40_chars() {
    let long = "abcdefghijabcdefghijabcdefghijabcdefghij-extra-extra";
    let b = derive_branch_name(long, "01HX9YZAAA");
    let prefix = "abcdefghijabcdefghijabcdefghijabcdefghij";
    assert_eq!(prefix.chars().count(), 40);
    assert_eq!(b, format!("{prefix}-01hx"));
}

#[test]
fn derive_branch_name_falls_back_when_title_is_empty_or_special() {
    // empty -> fallback
    let b = derive_branch_name("   ", "01HXABCDEFGHIJ");
    assert_eq!(b, "task-01hxabcd");

    // sanitize-rejected (path-traversal) -> fallback
    let b = derive_branch_name("..foo", "01HXABCDEFGHIJ");
    assert_eq!(b, "task-01hxabcd");
}

#[test]
fn derive_branch_name_handles_korean_title() {
    // sanitize_branch_name rejects spaces / control chars but accepts
    // CJK characters. Truncation must be character-based, not byte-based.
    let b = derive_branch_name("한글-제목-입니다", "01HXabcd");
    assert_eq!(b, "한글-제목-입니다-01hx");
}

// ---------------------------------------------------------------------------
// sanitize parity (a subset of lane_ops::sanitize_branch_name cases)
// ---------------------------------------------------------------------------

#[test]
fn sanitize_branch_name_rejects_obvious_bad_inputs() {
    assert_eq!(sanitize_branch_name(""), None);
    assert_eq!(sanitize_branch_name("   "), None);
    assert_eq!(sanitize_branch_name("foo bar"), None); // space
    assert_eq!(sanitize_branch_name("foo:bar"), None);
    assert_eq!(sanitize_branch_name("foo~bar"), None);
    assert_eq!(sanitize_branch_name("foo^bar"), None);
    assert_eq!(sanitize_branch_name("foo?bar"), None);
    assert_eq!(sanitize_branch_name("foo*bar"), None);
    assert_eq!(sanitize_branch_name("foo[bar"), None);
    assert_eq!(sanitize_branch_name("foo\\bar"), None);
    assert_eq!(sanitize_branch_name("foo\u{0007}"), None); // control
    assert_eq!(sanitize_branch_name("foo..bar"), None);
    assert_eq!(sanitize_branch_name("/foo"), None);
    assert_eq!(sanitize_branch_name("foo/"), None);
    assert_eq!(sanitize_branch_name(".foo"), None);
    assert_eq!(sanitize_branch_name("foo."), None);
}

#[test]
fn sanitize_branch_name_accepts_typical_kebab() {
    assert_eq!(
        sanitize_branch_name("fix-auth-bug").as_deref(),
        Some("fix-auth-bug"),
    );
    assert_eq!(
        sanitize_branch_name("feature/login").as_deref(),
        Some("feature/login"),
    );
}

// ---------------------------------------------------------------------------
// Prompt file + claude command
// ---------------------------------------------------------------------------

#[test]
fn task_prompt_file_path_is_under_daruda_subdir() {
    let p = task_prompt_file_path(&PathBuf::from("/repo/wt"), "fix-auth-01hx");
    assert_eq!(
        p,
        PathBuf::from("/repo/wt")
            .join(PROMPT_DIR_NAME)
            .join("task-fix-auth-01hx.md"),
    );
}

#[test]
fn write_prompt_file_creates_dir_and_writes_contents() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = write_prompt_file(tmp.path(), "branch-aaaa", "hello world").expect("write succeeds");
    assert!(path.exists(), "prompt file written");
    let contents = std::fs::read_to_string(&path).expect("read");
    assert_eq!(contents, "hello world");
    assert_eq!(
        path.parent().unwrap().file_name().unwrap(),
        std::ffi::OsStr::new(PROMPT_DIR_NAME),
    );
}

#[test]
fn write_prompt_file_overwrites_existing_file() {
    let tmp = tempfile::tempdir().expect("tempdir");
    write_prompt_file(tmp.path(), "branch-aaaa", "first").expect("write 1");
    let path = write_prompt_file(tmp.path(), "branch-aaaa", "second").expect("write 2");
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
    let out = render_task_prompt(&t);
    assert!(out.starts_with("Task: \"Fix auth bug\""));
    assert!(out.contains(&t.branch_name));
    assert!(out.contains("Status: Backlog"));
    assert!(out.contains(&t.prompt));
    assert!(out.contains(&format!("update task \"{}\"", t.id)));
}

// ---------------------------------------------------------------------------
// Persistence round-trip + schema-version rejection
// ---------------------------------------------------------------------------

#[test]
fn save_and_load_round_trip_preserves_state() {
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

    save_tasks_in(tmp.path(), &state).expect("save");
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

#[test]
fn save_creates_data_dir_if_missing() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let nested = tmp.path().join("nested").join("daruda");
    let state = TasksState::default();
    save_tasks_in(&nested, &state).expect("save creates dirs");
    assert!(tasks_path_in(&nested).exists());
}
