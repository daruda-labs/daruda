use daruda_store::tasks::{
    ExecutionRef, SessionEndReason, TaskEvent, TaskMachine, TaskRunIn, TaskSpec, TaskStore,
    machine::{SettleOutcome, TaskEdit},
};

fn edit(id: &str, title: &str) -> TaskEvent {
    TaskEvent::Edited {
        id: id.into(),
        edit: TaskEdit {
            title: title.into(),
            prompt: format!("{title} prompt"),
            notes: String::new(),
            auto_execute: true,
            agent_surface: Default::default(),
            base_worktree_path: None,
            branch: None,
            run_in: None,
            project: None,
        },
        at: chrono::Utc::now(),
    }
}

#[test]
fn review_regression_delivery_and_durable_history_use_the_dispatch_description() {
    let dir = tempfile::tempdir().unwrap();
    let open = || TaskStore::in_directory(dir.path(), None, "test", Box::new(|_, _| {})).unwrap();
    let store = open();
    let spec = TaskSpec::new(
        Default::default(),
        "original".into(),
        "original prompt".into(),
        None,
        chrono::Utc::now(),
    );
    let id = spec.id.clone();
    let exec = ExecutionRef {
        task_id: id.clone(),
        execution_id: "first".into(),
    };
    let commit = |event| {
        smol::block_on(store.propose(event)).unwrap();
    };
    commit(TaskEvent::Created { spec });
    commit(TaskEvent::DispatchRequested {
        id: id.clone(),
        execution_id: exec.execution_id.clone(),
        source: Default::default(),
        started_by: None,
        at: chrono::Utc::now(),
    });
    commit(edit(&id, "edited while preparing"));
    let (_, book) = store.current().unwrap();
    let rendered = daruda_store::tasks::render_task_prompt(book.get(&id).unwrap(), "main");
    assert!(rendered.contains("Task: \"original\""), "{rendered}");
    assert!(rendered.contains("original prompt"));
    commit(TaskEvent::LanePrepared {
        exec: exec.clone(),
        worktree: "/work".into(),
        agent_id: "claude".into(),
        account_id: None,
        cwd: "/work".into(),
        at: chrono::Utc::now(),
    });
    commit(TaskEvent::Dispatched {
        exec: exec.clone(),
        session_id: None,
        at: chrono::Utc::now(),
    });
    commit(edit(&id, "edited while running"));
    commit(TaskEvent::Settled {
        exec: exec.clone(),
        outcome: SettleOutcome::Completed {
            end_reason: SessionEndReason::Stop,
        },
        result: Some("original answer".into()),
        at: chrono::Utc::now(),
    });
    commit(TaskEvent::Reopened {
        id: id.clone(),
        run_in: TaskRunIn::default(),
        at: chrono::Utc::now(),
    });
    commit(edit(&id, "next"));
    drop(store);
    let (_, book) = open().current().unwrap();
    let record = book.get(&id).unwrap();
    let end = record.ended_executions.last().unwrap();
    assert_eq!(end.exec, exec);
    assert_eq!(end.result.as_deref(), Some("original answer"));
    let description = serde_json::to_value(end).unwrap()["description"].clone();
    assert_eq!(
        description,
        serde_json::json!({"title":"original", "prompt":"original prompt"})
    );
    assert_eq!(record.spec.title, "next");
    assert_eq!(<TaskMachine as daruda_store::tasks::Machine>::SCHEMA, 4);
}

#[test]
fn old_snapshots_leave_execution_descriptions_unknown() {
    let spec = TaskSpec::new(
        Default::default(),
        "current edited title".into(),
        "current edited prompt".into(),
        None,
        chrono::Utc::now(),
    );
    let mut book = daruda_store::tasks::TasksBook::default();
    use daruda_store::tasks::Machine as _;
    TaskMachine::apply(&mut book, &TaskEvent::Created { spec: spec.clone() });
    TaskMachine::apply(
        &mut book,
        &TaskEvent::DispatchRequested {
            id: spec.id.clone(),
            execution_id: "old".into(),
            source: Default::default(),
            started_by: None,
            at: chrono::Utc::now(),
        },
    );
    TaskMachine::apply(
        &mut book,
        &TaskEvent::DispatchFailed {
            exec: ExecutionRef {
                task_id: spec.id.clone(),
                execution_id: "old".into(),
            },
            cause: daruda_store::tasks::machine::StartFailure::WindowClosed,
            at: chrono::Utc::now(),
        },
    );
    let mut value = serde_json::to_value(book.get(&spec.id).unwrap()).unwrap();
    value.as_object_mut().unwrap().remove("description");
    value["ended_executions"][0]
        .as_object_mut()
        .unwrap()
        .remove("description");
    let record: daruda_store::tasks::TaskRecord = serde_json::from_value(value).unwrap();
    assert!(record.description.is_none());
    assert!(record.ended_executions[0].description.is_none());
}
