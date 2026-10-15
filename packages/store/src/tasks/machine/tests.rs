use chrono::{TimeZone, Utc};

use super::apply::apply;
use super::check::allowed;
use super::*;
use crate::tasks::SubTask;

#[test]
fn import_and_create_never_act_on_an_existing_record() {
    for from in enum_iterator::all::<LifecycleKind>() {
        assert!(!allowed(from, EventKind::Imported));
        assert!(!allowed(from, EventKind::Created));
    }
}

#[test]
fn a_rejected_delete_of_an_unknown_task_leaves_apply_a_no_op() {
    let mut book = TasksBook::default();
    let at = Utc.timestamp_opt(0, 0).unwrap();
    apply(
        &mut book,
        &TaskEvent::SubtaskAdded {
            id: "missing".into(),
            subtask: SubTask::default(),
            at,
        },
    );
    assert!(book.is_empty());
}
