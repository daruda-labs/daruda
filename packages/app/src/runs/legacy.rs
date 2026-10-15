//! The legacy `tasks.json`, rewritten from the projection after commits so
//! a downgraded build still finds the user's tasks. One write runs at a
//! time; commits that land during it fold into a single follow-up write of
//! the newest projection, so superseded snapshots are never written.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;
use daruda_store::observability::system_info::redact_home;
use daruda_store::tasks::TasksBook;
use gpui::{App, BorrowAppContext as _};

use super::{GlobalTasks, RunsHost};

pub(super) struct LegacyMirror {
    /// `None` when nothing may be written: no profile, or a suspended one.
    dir: Option<PathBuf>,
    writer: Writer,
    written: Arc<Mutex<Option<u64>>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Writer {
    Idle,
    /// A write is running; `again` asks for one more once it finishes.
    Writing {
        again: bool,
    },
}

impl LegacyMirror {
    pub(super) fn new(dir: Option<PathBuf>) -> Self {
        Self {
            dir,
            writer: Writer::Idle,
            written: Arc::new(Mutex::new(None)),
        }
    }

    #[cfg(any(test, feature = "screenshot"))]
    pub(super) fn disable(&mut self) {
        self.dir = None;
    }

    /// Write the projection now, on this thread: the app is quitting and a
    /// background write would not finish.
    pub(super) fn write_now(&self, seq: u64, book: &TasksBook) {
        if let Some(dir) = &self.dir {
            save(dir, seq, book, &self.written);
        }
    }
}

/// Queue a write of the current projection.
pub(super) fn schedule(cx: &mut App) {
    let start = cx.update_global::<RunsHost, _>(|host, _| {
        let mirror = &mut host.legacy;
        let dir = mirror.dir.clone()?;
        match &mut mirror.writer {
            Writer::Writing { again } => {
                *again = true;
                None
            }
            Writer::Idle => {
                mirror.writer = Writer::Writing { again: false };
                Some((dir, mirror.written.clone()))
            }
        }
    });
    let Some((dir, written)) = start else { return };
    let book = cx.global::<GlobalTasks>().book().clone();
    let seq = cx.global::<GlobalTasks>().seq();
    let pending = written.clone();
    let write = cx
        .background_executor()
        .spawn(async move { save(&dir, seq, &book, &pending) });
    cx.spawn(async move |cx| {
        write.await;
        cx.update(|cx| {
            let again = cx.update_global::<RunsHost, _>(|host, _| {
                if !Arc::ptr_eq(&written, &host.legacy.written) {
                    return false;
                }
                let again = host.legacy.writer == Writer::Writing { again: true };
                host.legacy.writer = Writer::Idle;
                again
            });
            if again {
                schedule(cx);
            }
        });
    })
    .detach();
}

fn save(dir: &Path, seq: u64, book: &TasksBook, written: &Mutex<Option<u64>>) {
    let mut written = written.lock().unwrap_or_else(|error| error.into_inner());
    if written.is_some_and(|previous| previous >= seq) {
        return;
    }
    let rows = daruda_store::tasks::legacy_rows(book);
    if let Err(e) = daruda_store::tasks::save_tasks_in(dir, &rows) {
        LogWriter::log(
            ErrorReport::new(crate::surface::strings::error::tasks_save_failed())
                .severity(ErrorSeverity::Warning)
                .from_error(&e)
                .at(file!(), line!())
                .with_context("dir", redact_home(dir))
                .dedup("tasks.save")
                .build(),
        );
    } else {
        *written = Some(seq);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_delayed_snapshot_never_overwrites_the_quit_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let newer = crate::runs::seed::book(vec![crate::runs::seed::record(
            Default::default(),
            "latest",
            daruda_store::tasks::Lifecycle::Backlog,
        )]);
        let written = Mutex::new(None);
        save(dir.path(), 2, &newer, &written);
        save(dir.path(), 1, &TasksBook::default(), &written);
        let restored = daruda_store::tasks::load_tasks_in(dir.path()).unwrap();
        assert_eq!(restored.tasks.len(), 1);
        assert_eq!(restored.tasks[0].title, "latest");
    }
}
