use super::*;
use crate::runs::seed::{self, SeedEnd, SeedRun};
use daruda_runlog::Machine as _;
use daruda_store::tasks::{TaskMachine, TasksBook};

fn record(end: SeedEnd) -> (TaskRecord, ExecutionRef) {
    let mut book = TasksBook::default();
    let spec = seed::spec(Default::default(), "title");
    let id = spec.id.clone();
    let run = SeedRun::chat("/work");
    for event in seed::created(spec)
        .into_iter()
        .chain(seed::run(&id, &run, end))
    {
        TaskMachine::apply(&mut book, &event);
    }
    (book.get(&id).unwrap().clone(), run.exec(&id))
}

#[test]
fn task_lifecycles_preserve_their_meaning() {
    for (end, expected) in [
        (SeedEnd::Done, EndedAs::Completed),
        (SeedEnd::Errored, EndedAs::Error),
        (SeedEnd::Cancelled, EndedAs::Canceled),
        (SeedEnd::Interrupted, EndedAs::Interrupted),
        (SeedEnd::Undelivered, EndedAs::Undelivered),
        (SeedEnd::StartFailed, EndedAs::Error),
    ] {
        let (record, exec) = record(end);
        assert!(
            matches!(task_snapshot(&record, &exec).unwrap().state, RunState::Ended {ended_as, ..} if ended_as == expected)
        );
    }
    for end in [SeedEnd::Preparing, SeedEnd::Dispatching, SeedEnd::Running] {
        let (record, exec) = record(end);
        assert!(matches!(
            task_snapshot(&record, &exec).unwrap().state,
            RunState::Running {
                waiting_on: None,
                ..
            }
        ));
    }
}

#[test]
fn result_bounds_and_why_use_unicode_characters_and_keep_full_read_pointers() {
    let (mut record, exec) = record(SeedEnd::Done);
    record
        .ended_executions
        .last_mut()
        .unwrap()
        .description
        .as_mut()
        .unwrap()
        .prompt = "한".repeat(400);
    let end = record.ended_executions.last_mut().unwrap();
    end.result = Some(format!("head{}tail", "한".repeat(5000)));
    end.execution.as_mut().unwrap().source = daruda_store::tasks::TaskExecutionSource::ClaudeCli {
        transcript_path: Some("/full.jsonl".into()),
        process: daruda_store::tasks::CliProcessState::Discovering,
    };
    let snapshot = task_snapshot(&record, &exec).unwrap();
    assert_eq!(snapshot.why.unwrap().chars().count(), 300);
    let RunState::Ended {
        text: Some(text),
        pointers,
        ..
    } = snapshot.state
    else {
        panic!("ended text");
    };
    assert!(
        text.starts_with("head")
            && text.ends_with("tail")
            && text.contains("[...truncated by daruda...]")
    );
    assert_eq!(pointers.transcript, Some("/full.jsonl".into()));
}

#[test]
fn exact_execution_never_reads_another_result_and_legacy_terminal_falls_back() {
    let (mut record, exec) = record(SeedEnd::Done);
    record.ended_executions.clear();
    assert!(matches!(
        task_snapshot(&record, &exec).unwrap().state,
        RunState::Ended {
            ended_as: EndedAs::Completed,
            text: None,
            ..
        }
    ));
    assert!(
        task_snapshot(
            &record,
            &ExecutionRef {
                execution_id: "other".into(),
                ..exec
            }
        )
        .is_none()
    );
}

#[test]
fn unrecorded_flow_errors_are_frozen_without_disk() {
    let base = RunSnapshot {
        key: RunKey::Flow("run".into()),
        what: "flow".into(),
        why: Some("until: end".into()),
        state: RunState::Unknown,
    };
    let end = daruda_flow::event::RunEnd::NotResumed {
        message: "cannot resume".into(),
    };
    let result = flow_ended(
        base,
        &end,
        Pointers {
            run_dir: Some("/gone/run".into()),
            ..Default::default()
        },
    );
    assert!(
        matches!(result.state,RunState::Ended {ended_as:EndedAs::Error,text:Some(ref text),..} if text == "cannot resume")
    );
    assert_eq!(result.what, "flow");
    assert_eq!(result.why.as_deref(), Some("until: end"));
}

#[test]
fn review_regression_execution_description_is_frozen_at_dispatch() {
    use daruda_store::tasks::machine::TaskEdit;
    use daruda_store::tasks::{TaskEvent, TaskRunIn};
    let mut book = TasksBook::default();
    let mut spec = seed::spec(Default::default(), "original");
    spec.prompt = "original prompt".into();
    let id = spec.id.clone();
    TaskMachine::apply(&mut book, &TaskEvent::Created { spec });
    let run = SeedRun::chat("/work");
    let events = seed::run(&id, &run, SeedEnd::Done);
    for event in &events[..events.len() - 1] {
        TaskMachine::apply(&mut book, event);
    }
    let edit = |title: &str| TaskEvent::Edited {
        id: id.clone(),
        at: chrono::Utc::now(),
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
    };
    TaskMachine::apply(&mut book, &edit("during run"));
    TaskMachine::apply(&mut book, events.last().unwrap());
    TaskMachine::apply(
        &mut book,
        &TaskEvent::Reopened {
            id: id.clone(),
            run_in: TaskRunIn::default(),
            at: chrono::Utc::now(),
        },
    );
    TaskMachine::apply(&mut book, &edit("next"));
    for event in seed::run(&id, &SeedRun::chat("/work"), SeedEnd::Running) {
        TaskMachine::apply(&mut book, &event);
    }
    let snapshot = task_snapshot(book.get(&id).unwrap(), &run.exec(&id)).unwrap();
    assert_eq!(snapshot.what, "original");
    assert_eq!(snapshot.why.as_deref(), Some("original prompt"));
}
