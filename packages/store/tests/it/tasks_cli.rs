//! Task CLI execution records and chat access serialization.

use daruda_store::tasks::cli::*;

#[test]
fn snapshot_ownership_survives_serialization() {
    let access = AgentChatAccess::CliSnapshot(ExecutionRef {
        task_id: "task".into(),
        execution_id: "run".into(),
    });
    let saved = serde_json::to_string(&access).unwrap();
    // The wire shape restore depends on; a rename here is a format change.
    assert_eq!(
        saved,
        r#"{"cli_snapshot":{"task_id":"task","execution_id":"run"}}"#
    );
    let restored: AgentChatAccess = serde_json::from_str(&saved).unwrap();
    assert_eq!(restored, access);
    assert!(restored.is_read_only());
    assert!(!AgentChatAccess::default().is_read_only());
}

#[test]
fn legacy_execution_is_agent_chat() {
    let execution: daruda_store::tasks::TaskExecution = serde_json::from_value(serde_json::json!({
        "id": "run", "agent_id": "claude", "account_id": null,
        "cwd": "/tmp", "session_id": "session"
    }))
    .unwrap();
    assert_eq!(execution.source, TaskExecutionSource::AgentChat);
    assert!(execution.chat_available());
}

#[test]
fn cli_execution_opens_chat_only_once_its_session_is_known() {
    let mut execution = daruda_store::tasks::TaskExecution {
        source: TaskExecutionSource::ClaudeCli {
            transcript_path: None,
            process: CliProcessState::Discovering,
        },
        id: "run".into(),
        agent_id: "claude".into(),
        account_id: None,
        cwd: "/tmp".into(),
        session_id: None,
    };
    assert!(!execution.chat_available());
    execution.session_id = Some("session".into());
    assert!(execution.chat_available());
}

#[test]
fn cli_process_states_round_trip() {
    let ended_at = chrono::DateTime::from_timestamp(1_800_000_000, 0).unwrap();
    for process in [
        CliProcessState::Discovering,
        CliProcessState::Running { pid: 42 },
        CliProcessState::ExitConfirmed { ended_at },
        CliProcessState::Unknown,
    ] {
        let source = TaskExecutionSource::ClaudeCli {
            transcript_path: Some("/tmp/t.jsonl".into()),
            process,
        };
        let saved = serde_json::to_string(&source).unwrap();
        assert_eq!(
            serde_json::from_str::<TaskExecutionSource>(&saved).unwrap(),
            source
        );
    }
}

#[test]
fn a_process_record_follows_only_what_its_own_pid_proves() {
    let at = chrono::DateTime::from_timestamp(1_800_000_000, 0).unwrap();
    let mut process = CliProcessState::Discovering;
    assert!(process.observe_running(10));
    assert!(!process.observe_running(10));
    assert!(!process.confirm_exit(99, at));
    assert_eq!(process, CliProcessState::Running { pid: 10 });
    assert!(process.confirm_exit(10, at));
    assert!(!process.confirm_exit(10, at));
    // A resumed session is written again.
    assert!(process.observe_running(11));
    assert_eq!(process, CliProcessState::Running { pid: 11 });

    let mut orphan = CliProcessState::Discovering;
    assert!(orphan.orphan_discovery());
    assert_eq!(orphan, CliProcessState::Unknown);
    assert!(!process.orphan_discovery());
}

#[test]
fn a_ref_resolves_only_the_run_it_names() {
    let mut tasks = daruda_store::tasks::TasksState::default();
    let mut task = daruda_store::tasks::Task::new(
        daruda_store::project::ProjectUuid::new(),
        "t".into(),
        "p".into(),
        None,
    );
    task.execution = Some(daruda_store::tasks::TaskExecution::begin(
        TaskExecutionSource::AgentChat,
        "claude".into(),
        None,
        "/tmp".into(),
    ));
    let run = ExecutionRef {
        task_id: task.id.clone(),
        execution_id: task.execution.as_ref().unwrap().id.clone(),
    };
    let stale = ExecutionRef {
        execution_id: "earlier".into(),
        ..run.clone()
    };
    tasks.add(task);
    assert!(run.resolve(&tasks).is_some());
    assert!(stale.resolve(&tasks).is_none());
    tasks.remove(&run.task_id);
    assert!(run.resolve(&tasks).is_none());
}
