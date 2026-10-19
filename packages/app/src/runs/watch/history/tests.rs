use super::super::tests::ended;
use super::*;
use crate::runs::seed::{self, SeedEnd, SeedRun};
use daruda_store::tasks::{ExecutionRef, Machine as _, TaskEvent, TaskMachine};

fn extend(book: &TasksBook, events: Vec<TaskEvent>) -> TasksBook {
    let mut book = book.clone();
    for event in events {
        TaskMachine::check(&book, &event).unwrap_or_else(|e| panic!("{event:?}: {e}"));
        TaskMachine::apply(&mut book, &event);
    }
    book
}

fn ended_task(book: &TasksBook, title: &str) -> (TasksBook, ExecutionRef) {
    let spec = seed::spec(Default::default(), title);
    let id = spec.id.clone();
    let run = SeedRun::chat("/wt");
    let mut events = seed::created(spec);
    events.extend(seed::run(&id, &run, SeedEnd::Done));
    (extend(book, events), run.exec(&id))
}

#[test]
fn an_unrelated_commit_changes_no_revision_and_wakes_no_waiter() {
    let mut watch = RunWatch::default();
    let (book, exec) = ended_task(&TasksBook::default(), "done");
    let key = RunKey::Task(exec);
    watch.register(key.clone());
    watch.sync_task_history(None, &book);
    assert_eq!(watch.unreported().len(), 1);
    let revision = watch.revision(&key);
    let (_wait, wake) = watch
        .open_wait(Some(vec![RunKey::Flow("other".into())]))
        .unwrap();
    let next = extend(
        &book,
        seed::created(seed::spec(Default::default(), "unrelated")),
    );
    watch.sync_task_history(Some(&book), &next);
    assert_eq!(watch.revision(&key), revision);
    assert!(wake.try_recv().is_err());
    assert_eq!(watch.unreported().len(), 1);
}

#[test]
fn an_evicted_reported_task_end_is_not_resurrected_by_later_commits() {
    let mut watch = RunWatch::default();
    let (book, exec) = ended_task(&TasksBook::default(), "evicted");
    let key = RunKey::Task(exec);
    watch.register(key.clone());
    watch.sync_task_history(None, &book);
    for n in 0..=200 {
        let key = (n > 0).then(|| RunKey::Flow(format!("flow-{n}")));
        if let Some(key) = &key {
            watch.register(key.clone());
            watch.retain(ended(key.clone()));
        }
        let (id, _) = watch.open_wait(None).unwrap();
        watch.poll_wait(id, false).unwrap();
        watch.finish_wait(id, true);
    }
    assert!(
        watch.get(&key).is_none(),
        "the oldest reported end was evicted"
    );
    let next = extend(
        &book,
        seed::created(seed::spec(Default::default(), "later")),
    );
    watch.sync_task_history(Some(&book), &next);
    assert!(watch.get(&key).is_none());
    assert!(watch.unreported().is_empty());
}

#[test]
fn a_new_task_end_is_retained_once() {
    let mut watch = RunWatch::default();
    let (book, _) = ended_task(&TasksBook::default(), "first");
    watch.sync_task_history(None, &book);
    let (next, exec) = ended_task(&book, "second");
    let key = RunKey::Task(exec);
    watch.register(key.clone());
    watch.sync_task_history(Some(&book), &next);
    assert!(watch.get(&key).is_some());
    let revision = watch.revision(&key);
    let after = extend(
        &next,
        seed::created(seed::spec(Default::default(), "third")),
    );
    watch.sync_task_history(Some(&next), &after);
    assert_eq!(watch.revision(&key), revision);
    assert_eq!(watch.unreported().len(), 1);
}
