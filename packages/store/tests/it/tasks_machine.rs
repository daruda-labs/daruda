//! `TaskMachine`: the transition table, its preconditions, and the run-log
//! contract.

use std::path::PathBuf;

use chrono::{DateTime, TimeZone, Utc};
use daruda_runlog::Machine as _;
use daruda_runlog::testing::{assert_machine_laws, enumerate_crashes};
use daruda_store::project::ProjectUuid;
use daruda_store::tasks::machine::{
    ErrorCause, EventKind, InterruptCause, Lifecycle, LifecycleKind, SettleOutcome, StartFailure,
    StopCause, TaskEdit, TaskEvent, TaskMachine, TaskRecord, TaskRejected, TaskSpec, TasksBook,
    TodoItem, UndeliveredReason,
};
use daruda_store::tasks::{
    AgentType, CliProcessState, ExecutionRef, SessionEndReason, SubTask, TaskAgentSurface,
    TaskExecution, TaskExecutionSource, TaskRunIn,
};

const ID: &str = "01TASK";
const RUN: &str = "run-1";
const SESSION: &str = "s1";
const SUBTASK: &str = "sub-1";

fn at(seconds: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000 + seconds, 0).unwrap()
}

fn worktree() -> PathBuf {
    PathBuf::from("/repo/task-wt")
}

fn exec_of(execution_id: &str) -> ExecutionRef {
    ExecutionRef {
        task_id: ID.into(),
        execution_id: execution_id.into(),
    }
}

fn exec() -> ExecutionRef {
    exec_of(RUN)
}

fn spec() -> TaskSpec {
    TaskSpec {
        id: ID.into(),
        project: ProjectUuid(uuid::Uuid::from_u128(1)),
        title: "Fix auth".into(),
        prompt: "Login drops the token".into(),
        notes: String::new(),
        base_worktree_path: None,
        branch_name: "task-auth".into(),
        run_in: TaskRunIn::NewWorktree,
        agent_type: AgentType::Claude,
        agent_surface: TaskAgentSurface::Terminal,
        auto_execute: true,
        created_at: at(0),
    }
}

#[derive(Clone, Copy)]
enum Source {
    Chat,
    Cli,
}

fn execution(id: &str, source: Source) -> TaskExecution {
    TaskExecution {
        source: source_of(source),
        id: id.into(),
        agent_id: "claude".into(),
        account_id: None,
        cwd: worktree(),
        session_id: Some(SESSION.into()),
    }
}

fn source_of(source: Source) -> TaskExecutionSource {
    match source {
        Source::Chat => TaskExecutionSource::AgentChat,
        Source::Cli => TaskExecutionSource::ClaudeCli {
            transcript_path: None,
            process: CliProcessState::Discovering,
        },
    }
}

fn lifecycle(kind: LifecycleKind, source: Source) -> Lifecycle {
    let (exec, worktree) = (exec(), worktree());
    match kind {
        LifecycleKind::Backlog => Lifecycle::Backlog,
        LifecycleKind::Preparing => Lifecycle::Preparing {
            exec,
            source: source_of(source),
        },
        LifecycleKind::Dispatching => Lifecycle::Dispatching { exec, worktree },
        LifecycleKind::Running => Lifecycle::Running { exec, worktree },
        LifecycleKind::Done => Lifecycle::Done {
            exec,
            worktree,
            end_reason: SessionEndReason::Stop,
        },
        LifecycleKind::Error => Lifecycle::Error {
            exec: Some(exec),
            worktree: Some(worktree),
            cause: ErrorCause::SessionFailed,
        },
        LifecycleKind::Cancelled => Lifecycle::Cancelled {
            exec,
            worktree: Some(worktree),
        },
        LifecycleKind::Undelivered => Lifecycle::Undelivered {
            exec,
            worktree: Some(worktree),
            reason: UndeliveredReason::PaneClosed,
        },
        LifecycleKind::Interrupted => Lifecycle::Interrupted {
            exec: Some(exec),
            worktree,
            cause: InterruptCause::UserStop,
        },
    }
}

/// A record in `kind` whose every payload precondition is satisfiable, so
/// only the table decides.
fn record(kind: LifecycleKind, source: Source) -> TaskRecord {
    TaskRecord {
        spec: spec(),
        lifecycle: lifecycle(kind, source),
        // A run is recorded from `LanePrepared` on.
        execution: (!matches!(kind, LifecycleKind::Backlog | LifecycleKind::Preparing))
            .then(|| execution(RUN, source)),
        session_ids: vec![SESSION.into()],
        subtasks: vec![SubTask {
            id: SUBTASK.into(),
            title: "manual".into(),
            ..SubTask::default()
        }],
        updated_at: at(0),
        finished_at: None,
    }
}

fn book_of(records: Vec<TaskRecord>) -> TasksBook {
    let mut book = TasksBook::default();
    let import = TaskEvent::Imported { tasks: records };
    TaskMachine::check(&book, &import).unwrap();
    TaskMachine::apply(&mut book, &import);
    book
}

fn book(kind: LifecycleKind, source: Source) -> TasksBook {
    book_of(vec![record(kind, source)])
}

fn edit() -> TaskEdit {
    TaskEdit {
        title: "Renamed".into(),
        prompt: "New prompt".into(),
        notes: "notes".into(),
        auto_execute: false,
        agent_surface: TaskAgentSurface::AgentChat,
        base_worktree_path: Some(PathBuf::from("/repo/main")),
        branch: Some("task-other".into()),
        run_in: Some(TaskRunIn::ExistingLane {
            path: PathBuf::from("/repo/lane"),
        }),
        project: Some(ProjectUuid(uuid::Uuid::from_u128(2))),
    }
}

fn todo(content: &str, completed: bool, id: &str) -> TodoItem {
    TodoItem {
        content: content.into(),
        completed,
        id: id.into(),
    }
}

/// One event of `kind`, aimed at the fixture record.
fn sample(kind: EventKind) -> TaskEvent {
    let (id, exec, t) = (ID.to_owned(), exec(), at(10));
    match kind {
        EventKind::Imported => TaskEvent::Imported { tasks: Vec::new() },
        EventKind::Created => TaskEvent::Created { spec: spec() },
        EventKind::Edited => TaskEvent::Edited {
            id,
            edit: edit(),
            at: t,
        },
        EventKind::Deleted => TaskEvent::Deleted { id, at: t },
        EventKind::SubtaskAdded => TaskEvent::SubtaskAdded {
            id,
            subtask: SubTask {
                id: "sub-2".into(),
                title: "added".into(),
                ..SubTask::default()
            },
            at: t,
        },
        EventKind::SubtaskToggled => TaskEvent::SubtaskToggled {
            id,
            subtask_id: SUBTASK.into(),
            at: t,
        },
        EventKind::SubtaskRenamed => TaskEvent::SubtaskRenamed {
            id,
            subtask_id: SUBTASK.into(),
            title: "renamed".into(),
            at: t,
        },
        EventKind::SubtaskDeleted => TaskEvent::SubtaskDeleted {
            id,
            subtask_id: SUBTASK.into(),
            at: t,
        },
        EventKind::TodoMerged => TaskEvent::TodoMerged {
            id,
            session_id: SESSION.into(),
            items: vec![todo("write tests", false, "todo-1")],
            at: t,
        },
        EventKind::DispatchRequested => TaskEvent::DispatchRequested {
            id,
            execution_id: RUN.into(),
            source: source_of(Source::Cli),
            at: t,
        },
        EventKind::LanePrepared => TaskEvent::LanePrepared {
            exec,
            worktree: worktree(),
            agent_id: "claude".into(),
            account_id: None,
            cwd: worktree(),
            at: t,
        },
        EventKind::DispatchFailed => TaskEvent::DispatchFailed {
            exec,
            cause: StartFailure::GitAddFailed,
            at: t,
        },
        EventKind::Dispatched => TaskEvent::Dispatched {
            exec,
            session_id: None,
            at: t,
        },
        EventKind::Undelivered => TaskEvent::Undelivered {
            exec,
            reason: UndeliveredReason::QueueFull,
            at: t,
        },
        EventKind::ExecutionSessionBound => TaskEvent::ExecutionSessionBound {
            exec,
            session_id: SESSION.into(),
            at: t,
        },
        EventKind::SessionAttached => TaskEvent::SessionAttached {
            exec,
            session_id: "s2".into(),
            at: t,
        },
        EventKind::SessionDetached => TaskEvent::SessionDetached {
            exec,
            session_id: SESSION.into(),
            at: t,
        },
        EventKind::CliProcessObserved => TaskEvent::CliProcessObserved {
            exec,
            process: CliProcessState::Running { pid: 7 },
            at: t,
        },
        EventKind::CliTranscriptFound => TaskEvent::CliTranscriptFound {
            exec,
            transcript_path: PathBuf::from("/home/.claude/s1.jsonl"),
            at: t,
        },
        EventKind::Settled => TaskEvent::Settled {
            exec,
            outcome: SettleOutcome::Completed {
                end_reason: SessionEndReason::Stop,
            },
            at: t,
        },
        EventKind::ToolFailuresEscalated => TaskEvent::ToolFailuresEscalated { exec, at: t },
        EventKind::CancelRequested => TaskEvent::CancelRequested { exec, at: t },
        EventKind::LaneLost => TaskEvent::LaneLost { exec, at: t },
        EventKind::Reconciled => TaskEvent::Reconciled { exec, at: t },
        EventKind::Reopened => TaskEvent::Reopened {
            id,
            run_in: TaskRunIn::NewWorktree,
            at: t,
        },
    }
}

fn all_samples() -> Vec<TaskEvent> {
    enum_iterator::all::<EventKind>().map(sample).collect()
}

/// What an event does from one state.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Cell {
    /// `IllegalTransition`.
    No,
    /// Accepted; the lifecycle does not change.
    Same,
    /// Accepted; the lifecycle becomes this kind.
    To(LifecycleKind),
    /// Accepted; the record is removed.
    Gone,
}

use Cell::{Gone, No, Same, To};
use LifecycleKind as L;

/// Columns: Backlog, Preparing, Dispatching, Running, Done, Error,
/// Cancelled, Undelivered, Interrupted. `Imported` and `Created` target no
/// existing record; they are asserted separately.
const TABLE: &[(EventKind, [Cell; 9])] = &[
    (EventKind::Edited, [Same; 9]),
    (EventKind::SubtaskAdded, [Same; 9]),
    (EventKind::SubtaskToggled, [Same; 9]),
    (EventKind::SubtaskRenamed, [Same; 9]),
    (EventKind::SubtaskDeleted, [Same; 9]),
    (EventKind::TodoMerged, [Same; 9]),
    (
        EventKind::Deleted,
        [Gone, No, No, No, Gone, Gone, Gone, Gone, Gone],
    ),
    (
        EventKind::DispatchRequested,
        [To(L::Preparing), No, No, No, No, No, No, No, No],
    ),
    (
        EventKind::LanePrepared,
        [No, To(L::Dispatching), No, No, No, No, No, No, No],
    ),
    (
        EventKind::DispatchFailed,
        [No, To(L::Error), To(L::Error), No, No, No, No, No, No],
    ),
    (
        EventKind::Dispatched,
        [No, No, To(L::Running), No, No, No, No, No, No],
    ),
    (
        EventKind::Undelivered,
        [No, No, To(L::Undelivered), No, No, No, No, No, No],
    ),
    (
        EventKind::ExecutionSessionBound,
        [No, No, Same, Same, Same, Same, Same, Same, Same],
    ),
    (
        EventKind::SessionAttached,
        [No, No, No, Same, No, No, No, No, No],
    ),
    (
        EventKind::SessionDetached,
        [No, No, No, Same, No, No, No, No, No],
    ),
    (
        EventKind::CliProcessObserved,
        [No, No, Same, Same, Same, Same, Same, Same, Same],
    ),
    (
        EventKind::CliTranscriptFound,
        [No, No, Same, Same, Same, Same, Same, Same, Same],
    ),
    (
        EventKind::Settled,
        [No, No, No, To(L::Done), No, No, No, No, No],
    ),
    (
        EventKind::ToolFailuresEscalated,
        [No, No, No, To(L::Error), No, No, No, No, No],
    ),
    (
        EventKind::CancelRequested,
        [
            No,
            To(L::Cancelled),
            To(L::Cancelled),
            To(L::Cancelled),
            No,
            No,
            No,
            No,
            No,
        ],
    ),
    (
        EventKind::LaneLost,
        [No, No, No, To(L::Interrupted), No, No, No, No, No],
    ),
    (
        EventKind::Reconciled,
        [
            No,
            To(L::Undelivered),
            To(L::Undelivered),
            To(L::Interrupted),
            No,
            No,
            No,
            No,
            No,
        ],
    ),
    (
        EventKind::Reopened,
        [
            No,
            No,
            No,
            No,
            To(L::Backlog),
            To(L::Backlog),
            To(L::Backlog),
            To(L::Backlog),
            To(L::Backlog),
        ],
    ),
];

const COLUMNS: [LifecycleKind; 9] = [
    L::Backlog,
    L::Preparing,
    L::Dispatching,
    L::Running,
    L::Done,
    L::Error,
    L::Cancelled,
    L::Undelivered,
    L::Interrupted,
];

#[test]
fn every_state_event_pair_matches_the_table() {
    assert_eq!(
        COLUMNS.to_vec(),
        enum_iterator::all::<LifecycleKind>().collect::<Vec<_>>()
    );
    let listed: Vec<_> = TABLE.iter().map(|(kind, _)| *kind).collect();
    for kind in enum_iterator::all::<EventKind>() {
        let separate = matches!(kind, EventKind::Imported | EventKind::Created);
        assert_eq!(listed.contains(&kind), !separate, "{kind:?} row");
    }

    for &(kind, row) in TABLE {
        for (from, cell) in COLUMNS.into_iter().zip(row) {
            // A running CLI run is left to its tracker; the AgentChat
            // fixture exercises the table's Reconciled row.
            let source = if kind == EventKind::Reconciled {
                Source::Chat
            } else {
                Source::Cli
            };
            let mut state = book(from, source);
            let event = sample(kind);
            let verdict = TaskMachine::check(&state, &event);
            let at = format!("{from:?} x {kind:?}");
            let expected = match cell {
                No => {
                    let illegal = TaskRejected::IllegalTransition { from, event: kind };
                    assert_eq!(verdict, Err(illegal), "{at}");
                    continue;
                }
                Same => Some(from),
                To(next) => Some(next),
                Gone => None,
            };
            assert_eq!(verdict, Ok(()), "{at}");
            TaskMachine::apply(&mut state, &event);
            let after = state.get(ID).map(|r| r.lifecycle.kind());
            assert_eq!(after, expected, "{at}");
        }
    }

    for from in COLUMNS {
        let state = book(from, Source::Cli);
        assert_eq!(
            TaskMachine::check(&state, &sample(EventKind::Imported)),
            Err(TaskRejected::ImportIntoNonEmptyBook)
        );
        assert_eq!(
            TaskMachine::check(&state, &sample(EventKind::Created)),
            Err(TaskRejected::DuplicateTask { id: ID.into() })
        );
    }
    let empty = TasksBook::default();
    assert_eq!(
        TaskMachine::check(&empty, &sample(EventKind::Created)),
        Ok(())
    );
    assert_eq!(
        TaskMachine::check(&empty, &sample(EventKind::Imported)),
        Ok(())
    );
    assert_eq!(
        TaskMachine::check(&empty, &sample(EventKind::Edited)),
        Err(TaskRejected::UnknownTask { id: ID.into() })
    );
}

fn commit(book: &mut TasksBook, event: TaskEvent) {
    TaskMachine::check(book, &event).unwrap_or_else(|err| panic!("{event:?}: {err}"));
    TaskMachine::apply(book, &event);
}

fn running_book(source: Source, run: &str) -> TasksBook {
    let mut book = TasksBook::default();
    commit(&mut book, TaskEvent::Created { spec: spec() });
    start(&mut book, source, run);
    book
}

fn start(book: &mut TasksBook, source: Source, run: &str) {
    commit(
        book,
        TaskEvent::DispatchRequested {
            id: ID.into(),
            execution_id: run.into(),
            source: source_of(source),
            at: at(1),
        },
    );
    commit(
        book,
        TaskEvent::LanePrepared {
            exec: exec_of(run),
            worktree: worktree(),
            agent_id: "claude".into(),
            account_id: None,
            cwd: worktree(),
            at: at(2),
        },
    );
    let mut expected = execution(run, source);
    expected.session_id = None;
    assert_eq!(task(book).execution, Some(expected));
    commit(
        book,
        TaskEvent::Dispatched {
            exec: exec_of(run),
            session_id: None,
            at: at(3),
        },
    );
}

fn task(book: &TasksBook) -> &TaskRecord {
    book.get(ID).expect("the task exists")
}

#[test]
fn a_late_settle_from_an_earlier_execution_is_stale() {
    let mut book = running_book(Source::Chat, "run-1");
    let settle = |run: &str| TaskEvent::Settled {
        exec: exec_of(run),
        outcome: SettleOutcome::Completed {
            end_reason: SessionEndReason::Stop,
        },
        at: at(5),
    };
    commit(&mut book, settle("run-1"));
    commit(
        &mut book,
        TaskEvent::Reopened {
            id: ID.into(),
            run_in: TaskRunIn::NewWorktree,
            at: at(6),
        },
    );
    start(&mut book, Source::Chat, "run-2");

    assert_eq!(
        TaskMachine::check(&book, &settle("run-1")),
        Err(TaskRejected::StaleExecution { id: ID.into() })
    );
    assert_eq!(task(&book).lifecycle.kind(), LifecycleKind::Running);
}

#[test]
fn launch_fields_only_change_in_backlog() {
    let edited = |mut book: TasksBook| {
        commit(
            &mut book,
            TaskEvent::Edited {
                id: ID.into(),
                edit: edit(),
                at: at(9),
            },
        );
        task(&book).clone()
    };

    let backlog = edited(book(L::Backlog, Source::Chat));
    let expected = edit();
    assert_eq!(backlog.spec.branch_name, "task-other");
    assert_eq!(Some(backlog.spec.run_in), expected.run_in);
    assert_eq!(Some(backlog.spec.project), expected.project);
    assert_eq!(backlog.spec.title, "Renamed");
    assert_eq!(backlog.updated_at, at(9));

    let original = spec();
    let running = edited(book(L::Running, Source::Chat));
    assert_eq!(running.spec.branch_name, original.branch_name);
    assert_eq!(running.spec.run_in, original.run_in);
    assert_eq!(running.spec.project, original.project);
    // Non-launch fields still change in every state.
    assert_eq!(running.spec.title, "Renamed");
    assert_eq!(running.spec.base_worktree_path, expected.base_worktree_path);
}

#[test]
fn dispatch_facts_are_frozen_once_the_lane_is_prepared() {
    let mut book = running_book(Source::Cli, RUN);
    let frozen = task(&book).execution.clone();
    commit(
        &mut book,
        TaskEvent::Edited {
            id: ID.into(),
            edit: edit(),
            at: at(9),
        },
    );
    assert_eq!(task(&book).spec.agent_surface, TaskAgentSurface::AgentChat);
    assert_eq!(task(&book).execution, frozen);
    assert!(matches!(
        frozen.unwrap().source,
        TaskExecutionSource::ClaudeCli { .. }
    ));
}

#[test]
fn a_reopen_from_running_is_illegal() {
    let book = running_book(Source::Chat, RUN);
    assert_eq!(
        TaskMachine::check(&book, &sample(EventKind::Reopened)),
        Err(TaskRejected::IllegalTransition {
            from: L::Running,
            event: EventKind::Reopened
        })
    );
}

#[test]
fn a_cancel_after_done_is_illegal() {
    let book = book(L::Done, Source::Chat);
    assert_eq!(
        TaskMachine::check(&book, &sample(EventKind::CancelRequested)),
        Err(TaskRejected::IllegalTransition {
            from: L::Done,
            event: EventKind::CancelRequested
        })
    );
}

#[test]
fn a_stop_is_interrupted_not_done() {
    for (cause, expected) in [
        (StopCause::UserStop, InterruptCause::UserStop),
        (StopCause::ExtraUsage, InterruptCause::ExtraUsage),
    ] {
        let mut book = running_book(Source::Chat, RUN);
        commit(
            &mut book,
            TaskEvent::Settled {
                exec: exec(),
                outcome: SettleOutcome::Stopped { cause },
                at: at(5),
            },
        );
        assert_eq!(
            task(&book).lifecycle,
            Lifecycle::Interrupted {
                exec: Some(exec()),
                worktree: worktree(),
                cause: expected,
            }
        );
        assert_eq!(task(&book).finished_at, Some(at(5)));
    }
}

#[test]
fn settling_maps_the_session_end_like_the_legacy_hooks() {
    let settled = |outcome| {
        let mut book = running_book(Source::Chat, RUN);
        commit(
            &mut book,
            TaskEvent::Settled {
                exec: exec(),
                outcome,
                at: at(5),
            },
        );
        task(&book).lifecycle.clone()
    };
    let failed = Lifecycle::Error {
        exec: Some(exec()),
        worktree: Some(worktree()),
        cause: ErrorCause::SessionFailed,
    };
    assert_eq!(settled(SettleOutcome::Errored), failed);
    assert_eq!(
        settled(SettleOutcome::Completed {
            end_reason: SessionEndReason::Error
        }),
        failed
    );
    assert_eq!(
        settled(SettleOutcome::Completed {
            end_reason: SessionEndReason::PromptInputExit
        }),
        Lifecycle::Done {
            exec: exec(),
            worktree: worktree(),
            end_reason: SessionEndReason::PromptInputExit,
        }
    );
}

#[test]
fn every_terminal_move_clears_the_sessions_and_stamps_the_end() {
    let mut book = running_book(Source::Cli, RUN);
    commit(
        &mut book,
        TaskEvent::SessionAttached {
            exec: exec(),
            session_id: SESSION.into(),
            at: at(4),
        },
    );
    commit(
        &mut book,
        TaskEvent::CancelRequested {
            exec: exec(),
            at: at(7),
        },
    );
    let record = task(&book);
    assert!(record.session_ids.is_empty());
    assert_eq!(
        (record.finished_at, record.updated_at),
        (Some(at(7)), at(7))
    );

    commit(
        &mut book,
        TaskEvent::Reopened {
            id: ID.into(),
            run_in: TaskRunIn::ExistingLane { path: worktree() },
            at: at(8),
        },
    );
    let record = task(&book);
    assert_eq!(record.lifecycle, Lifecycle::Backlog);
    assert_eq!(record.execution, None);
    assert_eq!(record.finished_at, None);
    assert_eq!(
        record.spec.run_in,
        TaskRunIn::ExistingLane { path: worktree() }
    );
}

#[test]
fn deleting_an_active_task_requires_cancel_first() {
    let mut book = running_book(Source::Chat, RUN);
    let delete = TaskEvent::Deleted {
        id: ID.into(),
        at: at(8),
    };
    assert_eq!(
        TaskMachine::check(&book, &delete),
        Err(TaskRejected::IllegalTransition {
            from: L::Running,
            event: EventKind::Deleted
        })
    );
    commit(
        &mut book,
        TaskEvent::CancelRequested {
            exec: exec(),
            at: at(7),
        },
    );
    commit(&mut book, delete);
    assert!(book.is_empty());
}

#[test]
fn a_vanished_lane_never_rewrites_a_finished_task() {
    let book = book(L::Done, Source::Chat);
    assert_eq!(
        TaskMachine::check(
            &book,
            &TaskEvent::LaneLost {
                exec: exec(),
                at: at(9)
            }
        ),
        Err(TaskRejected::IllegalTransition {
            from: L::Done,
            event: EventKind::LaneLost
        })
    );
}

#[test]
fn a_running_cli_run_is_left_to_its_tracker_on_restart() {
    let book = running_book(Source::Cli, RUN);
    assert_eq!(
        TaskMachine::check(
            &book,
            &TaskEvent::Reconciled {
                exec: exec(),
                at: at(9)
            }
        ),
        Err(TaskRejected::CliRunOwnedByTracker { id: ID.into() })
    );
}

#[test]
fn the_cli_tracker_can_reconcile_a_run_only_after_it_confirms_ownership_was_lost() {
    for process in [
        CliProcessState::Unknown,
        CliProcessState::ExitConfirmed { ended_at: at(8) },
    ] {
        let mut book = running_book(Source::Cli, RUN);
        commit(
            &mut book,
            TaskEvent::CliProcessObserved {
                exec: exec(),
                process,
                at: at(8),
            },
        );
        commit(
            &mut book,
            TaskEvent::Reconciled {
                exec: exec(),
                at: at(9),
            },
        );
        assert!(matches!(
            task(&book).lifecycle,
            Lifecycle::Interrupted {
                cause: InterruptCause::AppRestart,
                ..
            }
        ));
    }
}

#[test]
fn reconciling_before_a_lane_exists_is_undelivered_without_a_worktree() {
    let mut book = TasksBook::default();
    commit(&mut book, TaskEvent::Created { spec: spec() });
    commit(
        &mut book,
        TaskEvent::DispatchRequested {
            id: ID.into(),
            execution_id: RUN.into(),
            source: source_of(Source::Chat),
            at: at(1),
        },
    );
    commit(
        &mut book,
        TaskEvent::Reconciled {
            exec: exec(),
            at: at(2),
        },
    );
    assert_eq!(
        task(&book).lifecycle,
        Lifecycle::Undelivered {
            exec: exec(),
            worktree: None,
            reason: UndeliveredReason::AppRestart,
        }
    );
}

#[test]
fn cli_facts_are_recorded_after_the_run_ended() {
    let mut book = book(L::Done, Source::Cli);
    let ended = CliProcessState::ExitConfirmed { ended_at: at(20) };
    commit(
        &mut book,
        TaskEvent::CliProcessObserved {
            exec: exec(),
            process: ended.clone(),
            at: at(20),
        },
    );
    commit(&mut book, sample(EventKind::CliTranscriptFound));
    let source = &task(&book).execution.as_ref().unwrap().source;
    assert_eq!(
        source,
        &TaskExecutionSource::ClaudeCli {
            transcript_path: Some(PathBuf::from("/home/.claude/s1.jsonl")),
            process: ended,
        }
    );
    assert_eq!(task(&book).lifecycle.kind(), L::Done);

    let chat = book_of(vec![record(L::Done, Source::Chat)]);
    assert_eq!(
        TaskMachine::check(&chat, &sample(EventKind::CliProcessObserved)),
        Err(TaskRejected::NotCliExecution { id: ID.into() })
    );
}

#[test]
fn an_execution_binds_one_session() {
    let mut book = running_book(Source::Chat, RUN);
    let bind = |session: &str| TaskEvent::ExecutionSessionBound {
        exec: exec(),
        session_id: session.into(),
        at: at(4),
    };
    commit(&mut book, bind("first"));
    assert_eq!(
        task(&book)
            .execution
            .as_ref()
            .unwrap()
            .session_id
            .as_deref(),
        Some("first")
    );
    // The same session again is an idempotent no-op.
    commit(&mut book, bind("first"));
    assert_eq!(
        TaskMachine::check(&book, &bind("second")),
        Err(TaskRejected::SessionAlreadyBound { id: ID.into() })
    );
}

#[test]
fn todo_merge_matches_the_legacy_rule() {
    let mut book = running_book(Source::Cli, RUN);
    for session in [SESSION, "s2"] {
        commit(
            &mut book,
            TaskEvent::SessionAttached {
                exec: exec(),
                session_id: session.into(),
                at: at(4),
            },
        );
    }
    commit(
        &mut book,
        TaskEvent::SubtaskAdded {
            id: ID.into(),
            subtask: SubTask {
                id: "manual".into(),
                title: "write tests".into(),
                ..SubTask::default()
            },
            at: at(4),
        },
    );
    let merge = |session: &str, items: Vec<TodoItem>, seconds| TaskEvent::TodoMerged {
        id: ID.into(),
        session_id: session.into(),
        items,
        at: at(seconds),
    };

    // A manual row with the same title is never touched; blanks are skipped;
    // a repeated title in one payload folds into the row it just created.
    commit(
        &mut book,
        merge(
            SESSION,
            vec![
                todo("  write tests ", false, "t1"),
                todo("   ", true, "blank"),
                todo("ship", false, "t2"),
                todo("ship", true, "t3"),
            ],
            10,
        ),
    );
    let rows = |book: &TasksBook| {
        task(book)
            .subtasks
            .iter()
            .map(|s| {
                (
                    s.id.clone(),
                    s.title.clone(),
                    s.completed,
                    s.source_session_id.clone(),
                )
            })
            .collect::<Vec<_>>()
    };
    let auto = Some(SESSION.to_owned());
    assert_eq!(
        rows(&book),
        vec![
            ("manual".into(), "write tests".into(), false, None),
            ("t1".into(), "write tests".into(), false, auto.clone()),
            ("t2".into(), "ship".into(), true, auto.clone()),
        ]
    );
    assert_eq!(task(&book).subtasks[1].created_at, Some(at(10)));
    assert_eq!(task(&book).updated_at, at(10));

    // Matches flip completion only; dropped items stay; another session's
    // identical title is its own row.
    commit(
        &mut book,
        merge(SESSION, vec![todo("write tests", true, "t4")], 11),
    );
    commit(&mut book, merge("s2", vec![todo("ship", false, "t5")], 12));
    assert_eq!(
        rows(&book),
        vec![
            ("manual".into(), "write tests".into(), false, None),
            ("t1".into(), "write tests".into(), true, auto.clone()),
            ("t2".into(), "ship".into(), true, auto),
            ("t5".into(), "ship".into(), false, Some("s2".into())),
        ]
    );

    // A payload that changes nothing leaves the timestamp alone.
    commit(
        &mut book,
        merge(SESSION, vec![todo("ship", true, "t6")], 13),
    );
    assert_eq!(task(&book).updated_at, at(12));

    assert_eq!(
        TaskMachine::check(&book, &merge("stranger", vec![todo("x", false, "t7")], 14)),
        Err(TaskRejected::UnknownSession {
            id: ID.into(),
            session_id: "stranger".into()
        })
    );
    assert_eq!(
        TaskMachine::check(&book, &merge(SESSION, vec![todo("new", false, "t1")], 14)),
        Err(TaskRejected::DuplicateSubtask {
            id: ID.into(),
            subtask_id: "t1".into()
        })
    );
}

#[test]
fn subtask_edits_follow_the_editor_rules() {
    let mut book = book(L::Backlog, Source::Chat);
    let rename = |title: &str| TaskEvent::SubtaskRenamed {
        id: ID.into(),
        subtask_id: SUBTASK.into(),
        title: title.into(),
        at: at(5),
    };
    assert_eq!(
        TaskMachine::check(&book, &rename("  ")),
        Err(TaskRejected::BlankSubtaskTitle { id: ID.into() })
    );
    // Renaming to the same title is no change.
    commit(&mut book, rename(" manual "));
    assert_eq!(task(&book).updated_at, at(0));
    assert_eq!(
        TaskMachine::check(
            &book,
            &TaskEvent::SubtaskToggled {
                id: ID.into(),
                subtask_id: "missing".into(),
                at: at(5)
            }
        ),
        Err(TaskRejected::UnknownSubtask {
            id: ID.into(),
            subtask_id: "missing".into()
        })
    );
}

fn law_states() -> Vec<TasksBook> {
    let mut states = vec![TasksBook::default()];
    for kind in COLUMNS {
        for source in [Source::Chat, Source::Cli] {
            states.push(book(kind, source));
        }
    }
    states
}

#[test]
fn the_machine_obeys_the_run_log_laws() {
    assert_machine_laws::<TaskMachine>(&law_states(), &all_samples());
}

#[test]
fn every_crash_point_recovers_a_committed_prefix() {
    let scenario = vec![
        TaskEvent::Created { spec: spec() },
        sample(EventKind::Edited),
        sample(EventKind::SubtaskAdded),
        TaskEvent::DispatchRequested {
            id: ID.into(),
            execution_id: RUN.into(),
            source: source_of(Source::Cli),
            at: at(11),
        },
        sample(EventKind::LanePrepared),
        TaskEvent::Dispatched {
            exec: exec(),
            session_id: Some(SESSION.into()),
            at: at(12),
        },
        TaskEvent::SessionAttached {
            exec: exec(),
            session_id: SESSION.into(),
            at: at(13),
        },
        sample(EventKind::TodoMerged),
        sample(EventKind::CliProcessObserved),
        sample(EventKind::Settled),
        TaskEvent::CliProcessObserved {
            exec: exec(),
            process: CliProcessState::ExitConfirmed { ended_at: at(20) },
            at: at(20),
        },
        // Rejected: a finished task cannot be cancelled.
        sample(EventKind::CancelRequested),
        sample(EventKind::Reopened),
        sample(EventKind::Deleted),
    ];
    assert!(enumerate_crashes::<TaskMachine>(scenario).cases > 0);
}

#[test]
fn every_lifecycle_and_event_round_trips_through_json() {
    let mut lifecycles: Vec<_> = COLUMNS
        .into_iter()
        .map(|kind| lifecycle(kind, Source::Cli))
        .collect();
    let causes = [
        ErrorCause::StartFailed(StartFailure::RepoBusy),
        ErrorCause::ToolFailures,
        ErrorCause::Imported {
            message: "lane gone".into(),
        },
    ];
    lifecycles.extend(causes.into_iter().map(|cause| Lifecycle::Error {
        exec: None,
        worktree: None,
        cause,
    }));
    for value in lifecycles {
        let json = serde_json::to_string(&value).unwrap();
        assert_eq!(
            serde_json::from_str::<Lifecycle>(&json).unwrap(),
            value,
            "{json}"
        );
    }

    let mut events = all_samples();
    events.push(TaskEvent::Imported {
        tasks: vec![record(L::Interrupted, Source::Cli)],
    });
    events.extend(
        [
            SettleOutcome::Errored,
            SettleOutcome::Stopped {
                cause: StopCause::ExtraUsage,
            },
        ]
        .into_iter()
        .map(|outcome| TaskEvent::Settled {
            exec: exec(),
            outcome,
            at: at(1),
        }),
    );
    for event in events {
        let json = serde_json::to_string(&event).unwrap();
        assert_eq!(
            serde_json::from_str::<TaskEvent>(&json).unwrap(),
            event,
            "{json}"
        );
    }
}

/// Why a prompt went undelivered survives the task log as written: a
/// session that failed after connecting reads back as that, not as one that
/// never connected.
#[test]
fn an_undelivered_reason_round_trips_through_the_task_log() {
    for reason in [
        UndeliveredReason::ConnectFailed,
        UndeliveredReason::SessionFailed,
    ] {
        let dir = tempfile::tempdir().unwrap();
        let open = || {
            daruda_store::tasks::TaskStore::in_directory(
                dir.path(),
                None,
                "test",
                Box::new(|_, _| {}),
            )
            .unwrap()
        };
        let store = open();
        let events = [
            TaskEvent::Created { spec: spec() },
            TaskEvent::DispatchRequested {
                id: ID.into(),
                execution_id: RUN.into(),
                source: source_of(Source::Chat),
                at: at(1),
            },
            TaskEvent::LanePrepared {
                exec: exec(),
                worktree: worktree(),
                agent_id: "claude".into(),
                account_id: None,
                cwd: worktree(),
                at: at(2),
            },
            TaskEvent::Undelivered {
                exec: exec(),
                reason,
                at: at(3),
            },
        ];
        for event in events {
            smol::block_on(store.propose(event)).unwrap();
        }
        drop(store);

        let (_, book) = open().current().unwrap();
        assert!(
            matches!(
                book.get(ID).unwrap().lifecycle,
                Lifecycle::Undelivered { reason: read, .. } if read == reason
            ),
            "{reason:?} did not survive the log"
        );
    }
    assert_eq!(
        serde_json::to_string(&UndeliveredReason::SessionFailed).unwrap(),
        "\"session_failed\""
    );
}
