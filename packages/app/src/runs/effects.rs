use daruda_store::tasks::ExecutionRef;
use gpui::App;

pub(crate) trait RunsEffects {
    fn stop_execution(&self, exec: &ExecutionRef, cx: &mut App);

    /// A flow run changed; the windows holding its worktree redraw it.
    fn flow_changed(&self, _change: &super::FlowChange, _cx: &mut App) {}

    fn cli_event(
        &self,
        _pane: Option<super::RunPane>,
        _event: daruda_agent::pty_tracker::PtyTrackerEvent,
        _cx: &mut App,
    ) {
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use daruda_store::tasks::{Lifecycle, TaskStore};
    use gpui::TestAppContext;

    use super::*;
    use crate::runs::seed::{self, SeedEnd, SeedRun};
    use crate::runs::{GlobalTasks, RunsHost, TaskCommitError, TaskIntent, no_hook};

    struct RecordingEffects {
        calls: Rc<RefCell<Vec<ExecutionRef>>>,
        store: TaskStore,
    }

    impl RunsEffects for RecordingEffects {
        fn stop_execution(&self, exec: &ExecutionRef, cx: &mut App) {
            let durable = self.store.current().unwrap().1;
            assert!(matches!(
                durable.get(&exec.task_id).unwrap().lifecycle,
                Lifecycle::Cancelled { .. }
            ));
            assert_eq!(cx.global::<GlobalTasks>().book(), &durable);
            self.calls.borrow_mut().push(exec.clone());
        }
    }

    #[gpui::test]
    async fn cancellation_effects_follow_commit_even_if_the_caller_leaves(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let store = TaskStore::in_directory(dir.path(), None, "test", no_hook()).unwrap();
        let calls = Rc::new(RefCell::new(Vec::new()));
        cx.update(|cx| {
            RunsHost::install(RunsHost::open(Ok(store.clone())), cx);
            RunsHost::install_effects(
                Rc::new(RecordingEffects {
                    calls: calls.clone(),
                    store,
                }),
                cx,
            );
        });
        let run = SeedRun::chat("/wt");
        let exec = cx.update(|cx| {
            let id = seed::task(
                seed::spec(Default::default(), "cancel"),
                Some((&run, SeedEnd::Running)),
                cx,
            );
            run.exec(&id)
        });
        drop(cx.update(|cx| RunsHost::task(TaskIntent::Cancel { exec: exec.clone() }, cx)));
        cx.run_until_parked();
        assert_eq!(*calls.borrow(), vec![exec.clone()]);
        let rejected = cx.update(|cx| RunsHost::task(TaskIntent::Cancel { exec }, cx));
        assert!(matches!(rejected.await, Err(TaskCommitError::Rejected(_))));
        assert_eq!(calls.borrow().len(), 1);
    }
}
