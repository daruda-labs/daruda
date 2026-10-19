//! Task end history projected into the ledger, one change at a time.
use super::state::RunWatch;
use crate::runs::result::{self, RunKey};
use daruda_store::tasks::TasksBook;

impl RunWatch {
    /// Retain only the ends `next` adds or changes relative to `previous`.
    /// Commits that leave a task's history alone are no lifecycle change.
    pub(crate) fn sync_task_history(&mut self, previous: Option<&TasksBook>, next: &TasksBook) {
        for record in next.records() {
            let before = previous.and_then(|book| book.get(&record.spec.id));
            for ended in &record.ended_executions {
                if before.is_some_and(|old| old.ended_executions.contains(ended)) {
                    continue;
                }
                if let Some(snapshot) = result::task_snapshot(record, &ended.exec) {
                    self.retain(snapshot);
                }
            }
        }
    }

    /// Join a newly opened log: its carried starts enter the scope before its
    /// history is compared with what the process already retained.
    pub(crate) fn adopt_log(
        &mut self,
        carried: Vec<RunKey>,
        previous: Option<&TasksBook>,
        next: &TasksBook,
    ) {
        for key in carried {
            self.register(key);
        }
        self.sync_task_history(previous, next);
    }
}

#[cfg(test)]
mod tests;
