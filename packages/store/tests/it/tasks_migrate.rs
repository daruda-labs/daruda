//! `TaskStore`'s one-time import of the legacy `tasks.json`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use daruda_runlog::{AfterCommit, Body, CatchUp, CommitError, Machine as _};
use daruda_store::project::ProjectUuid;
use daruda_store::tasks::machine::{
    ErrorCause, InterruptCause, Lifecycle, TaskMachine, TaskRejected, TasksBook,
};
use daruda_store::tasks::migrate::LEGACY_EXECUTION_ID;
use daruda_store::tasks::{
    ExecutionRef, SessionEndReason, SubTask, Task, TaskEvent, TaskExecution, TaskState, TaskStore,
    TasksState, save_tasks_in, tasks_path_in,
};

const BUILD: &str = "daruda_store-test";

fn no_hook() -> AfterCommit<TaskMachine> {
    Box::new(|_, _| {})
}

fn open(log_dir: &Path, legacy: &Path) -> TaskStore {
    TaskStore::in_directory(log_dir, Some(legacy), BUILD, no_hook()).unwrap()
}

/// The book the store holds, and how many events built it.
fn contents(store: &TaskStore) -> (TasksBook, usize) {
    store.flush(Duration::from_secs(5)).unwrap();
    let (catch_up, _live) = store.subscribe(0).unwrap();
    let CatchUp::Records(records) = catch_up else {
        panic!("a fresh log is never compacted");
    };
    let mut book = TasksBook::default();
    let mut events = 0;
    for record in records {
        if let Body::Event { event } = record.body {
            TaskMachine::apply(&mut book, &event);
            events += 1;
        }
    }
    (book, events)
}

fn wt(name: &str) -> PathBuf {
    PathBuf::from("/repo").join(name)
}

fn legacy_task(title: &str, state: TaskState) -> Task {
    let mut task = Task::new(ProjectUuid::new(), title.into(), "prompt".into(), None);
    task.state = state;
    task
}

fn legacy_file() -> TasksState {
    let mut running = legacy_task(
        "running",
        TaskState::Running {
            worktree_path: wt("a"),
        },
    );
    running.execution = Some(TaskExecution::begin(
        Default::default(),
        "claude".into(),
        None,
        wt("a"),
    ));
    running.session_ids = vec!["s1".into()];
    running.subtasks = vec![SubTask::new("step".into())];
    running.notes = "keep me".into();
    let tasks = vec![
        legacy_task("backlog", TaskState::Backlog),
        running,
        legacy_task(
            "done",
            TaskState::Done {
                worktree_path: wt("b"),
                end_reason: SessionEndReason::Stop,
            },
        ),
        legacy_task(
            "error",
            TaskState::Error {
                worktree_path: wt("c"),
                message: "tool_use_failure x5".into(),
            },
        ),
        legacy_task(
            "cancelled",
            TaskState::Cancelled {
                worktree_path: wt("d"),
            },
        ),
    ];
    TasksState {
        tasks,
        ..TasksState::default()
    }
}

#[test]
fn a_v2_tasks_file_imports_once_and_running_becomes_interrupted() {
    let (logs, data) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let legacy = legacy_file();
    save_tasks_in(data.path(), &legacy).unwrap();

    let store = open(logs.path(), data.path());
    let (book, events) = contents(&store);
    assert_eq!(events, 1, "one Imported event");
    assert_eq!(book.len(), legacy.tasks.len());
    for record in book.records() {
        if record.lifecycle == Lifecycle::Backlog {
            assert!(record.execution.is_none(), "backlog carries no execution");
        }
        if record.lifecycle.is_terminal() {
            assert!(record.session_ids.is_empty(), "terminal row has sessions");
        }
    }

    let by_title = |title: &str| {
        book.records()
            .find(|record| record.spec.title == title)
            .unwrap_or_else(|| panic!("{title} imported"))
    };
    let synthesized = |task: &Task| ExecutionRef {
        task_id: task.id.clone(),
        execution_id: LEGACY_EXECUTION_ID.into(),
    };
    let running = &legacy.tasks[1];
    let imported = by_title("running");
    assert_eq!(
        imported.lifecycle,
        Lifecycle::Interrupted {
            exec: Some(ExecutionRef {
                task_id: running.id.clone(),
                execution_id: running.execution.as_ref().unwrap().id.clone(),
            }),
            worktree: wt("a"),
            cause: InterruptCause::AppRestart,
        }
    );
    assert_eq!(imported.execution, running.execution);
    assert!(
        imported.session_ids.is_empty(),
        "a finished row has no live sessions"
    );
    assert_eq!(imported.subtasks, running.subtasks);
    assert_eq!(imported.spec.notes, "keep me");
    assert_eq!(imported.spec.created_at, running.created_at);
    assert_eq!(imported.updated_at, running.updated_at);

    assert_eq!(by_title("backlog").lifecycle, Lifecycle::Backlog);
    assert_eq!(
        by_title("done").lifecycle,
        Lifecycle::Done {
            exec: synthesized(&legacy.tasks[2]),
            worktree: wt("b"),
            end_reason: SessionEndReason::Stop,
        }
    );
    assert_eq!(
        by_title("error").lifecycle,
        Lifecycle::Error {
            exec: None,
            worktree: Some(wt("c")),
            cause: ErrorCause::Imported {
                message: "tool_use_failure x5".into()
            },
        }
    );
    assert_eq!(
        by_title("cancelled").lifecycle,
        Lifecycle::Cancelled {
            exec: synthesized(&legacy.tasks[4]),
            worktree: Some(wt("d")),
        }
    );
}

#[test]
fn import_leaves_the_source_file_byte_identical() {
    let (logs, data) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    save_tasks_in(data.path(), &legacy_file()).unwrap();
    let path = tasks_path_in(data.path());
    let before = std::fs::read(&path).unwrap();
    let modified = std::fs::metadata(&path).unwrap().modified().unwrap();

    let store = open(logs.path(), data.path());
    contents(&store);
    drop(store);

    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(
        std::fs::metadata(&path).unwrap().modified().unwrap(),
        modified
    );
}

#[test]
fn a_second_open_does_not_import_again() {
    let (logs, data) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    save_tasks_in(data.path(), &legacy_file()).unwrap();
    let first = contents(&open(logs.path(), data.path()));

    // The old build keeps writing the file until the app switches over.
    let mut grown = legacy_file();
    grown.add(legacy_task("later", TaskState::Backlog));
    save_tasks_in(data.path(), &grown).unwrap();

    let second = contents(&open(logs.path(), data.path()));
    assert_eq!(second, first);
}

#[test]
fn an_emptied_log_is_not_imported_again() {
    let (logs, data) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let legacy = legacy_file();
    save_tasks_in(data.path(), &legacy).unwrap();
    let store = open(logs.path(), data.path());
    for task in &legacy.tasks {
        if task.state == TaskState::Backlog {
            let delete = daruda_store::tasks::TaskEvent::Deleted {
                id: task.id.clone(),
                at: task.updated_at,
            };
            smol::block_on(store.propose(delete)).unwrap();
        }
    }
    drop(store);

    let (book, events) = contents(&open(logs.path(), data.path()));
    assert_eq!(events, 2);
    assert_eq!(book.len(), legacy.tasks.len() - 1);
}

#[test]
fn a_newer_tasks_file_is_not_imported() {
    let (logs, data) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let newer = r#"{"schema_version": 99, "tasks": []}"#;
    std::fs::write(tasks_path_in(data.path()), newer).unwrap();

    let (book, events) = contents(&open(logs.path(), data.path()));
    assert_eq!((book.len(), events), (0, 0));
    assert_eq!(
        std::fs::read_to_string(tasks_path_in(data.path())).unwrap(),
        newer
    );
}

#[test]
fn no_legacy_directory_opens_an_empty_store() {
    let logs = tempfile::tempdir().unwrap();
    let store = TaskStore::in_directory(logs.path(), None, BUILD, no_hook()).unwrap();
    assert_eq!(contents(&store).1, 0);
}

fn imported_book(legacy: &TasksState) -> (TaskStore, TasksBook) {
    let (logs, data) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    save_tasks_in(data.path(), legacy).unwrap();
    let store = open(logs.path(), data.path());
    let book = contents(&store).0;
    // The tempdirs must outlive the store's use; leak them to the process.
    std::mem::forget((logs, data));
    (store, book)
}

fn run_of(session: &str) -> TaskExecution {
    let mut run = TaskExecution::begin(Default::default(), "claude".into(), None, wt("a"));
    run.session_id = Some(session.into());
    run
}

#[test]
fn an_imported_backlog_row_carries_no_execution() {
    let mut backlog = legacy_task("backlog", TaskState::Backlog);
    backlog.execution = Some(run_of("s1"));
    let (_store, book) = imported_book(&TasksState {
        tasks: vec![backlog],
        ..TasksState::default()
    });
    let record = book.records().next().unwrap();
    assert_eq!(record.lifecycle, Lifecycle::Backlog);
    assert_eq!(record.execution, None);
}

#[test]
fn an_imported_finished_row_has_no_live_sessions() {
    let mut done = legacy_task(
        "done",
        TaskState::Done {
            worktree_path: wt("b"),
            end_reason: SessionEndReason::Stop,
        },
    );
    done.execution = Some(run_of("s1"));
    done.session_ids = vec!["s1".into()];
    let mut running = legacy_task(
        "running",
        TaskState::Running {
            worktree_path: wt("a"),
        },
    );
    running.execution = Some(run_of("s2"));
    running.session_ids = vec!["s2".into()];
    let (_store, book) = imported_book(&TasksState {
        tasks: vec![done.clone(), running.clone()],
        ..TasksState::default()
    });
    for (task, kind) in [(&done, "done"), (&running, "interrupted")] {
        let record = book.get(&task.id).unwrap();
        assert!(record.session_ids.is_empty(), "{kind} keeps no sessions");
        assert_eq!(
            record.execution, task.execution,
            "{kind} keeps its execution"
        );
    }
    assert!(matches!(
        book.get(&running.id).unwrap().lifecycle,
        Lifecycle::Interrupted { .. }
    ));
}

#[test]
fn todo_merge_into_an_imported_finished_row_is_rejected() {
    let mut done = legacy_task(
        "done",
        TaskState::Done {
            worktree_path: wt("b"),
            end_reason: SessionEndReason::Stop,
        },
    );
    done.session_ids = vec!["s1".into()];
    let id = done.id.clone();
    let (store, _book) = imported_book(&TasksState {
        tasks: vec![done],
        ..TasksState::default()
    });
    let merge = TaskEvent::TodoMerged {
        id,
        session_id: "s1".into(),
        items: Vec::new(),
        at: chrono::Utc::now(),
    };
    let outcome = smol::block_on(store.propose(merge));
    assert!(
        matches!(
            outcome,
            Err(CommitError::Rejected(TaskRejected::UnknownSession { .. }))
        ),
        "a finished row owns no session to merge from"
    );
}
