//! Reattach native watchers after backend errors without blocking the UI thread.

use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::time::Duration;

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

pub(super) struct WatchBackend {
    _stop: mpsc::Sender<()>,
}

fn attach(
    root: &Path,
    output: mpsc::SyncSender<Result<Event, String>>,
    failed: Arc<AtomicBool>,
    overflow: Arc<AtomicBool>,
) -> Result<RecommendedWatcher, notify::Error> {
    let reroot = Reroot::new(root);
    let watched_root = root.to_path_buf();
    let mut watcher = notify::recommended_watcher(move |event: Result<Event, notify::Error>| {
        let event = event.map(|mut event| {
            reroot.apply(&mut event);
            event
        });
        if event.as_ref().is_err() || event.as_ref().is_ok_and(|e| moves_root(e, &watched_root)) {
            failed.store(true, Ordering::Release);
        }
        deliver(&output, event.map_err(|error| error.to_string()), &overflow);
    })?;
    watcher.watch(root, RecursiveMode::Recursive)?;
    Ok(watcher)
}

/// Rewrites event paths into the spelling the caller watched. FSEvents
/// reports resolved paths (`/private/var` for `/var`, a symlink's target),
/// so without this every consumer would have to resolve its root per event.
struct Reroot {
    resolved: PathBuf,
    given: PathBuf,
}

impl Reroot {
    fn new(given: &Path) -> Self {
        Self {
            resolved: daruda_core::path::canonicalize_or_self(given),
            given: given.to_path_buf(),
        }
    }

    fn apply(&self, event: &mut Event) {
        if self.resolved == self.given {
            return;
        }
        for path in &mut event.paths {
            // A root whose link points at its own ancestor already starts
            // with `resolved` in the given spelling.
            if path.starts_with(&self.given) {
                continue;
            }
            if let Ok(rel) = path.strip_prefix(&self.resolved) {
                *path = self.given.join(rel);
            }
        }
    }
}

/// Whether `event` says the watched root itself was removed or renamed. The
/// identity poll misses a deleted root whose inode its successor reuses.
fn moves_root(event: &Event, root: &Path) -> bool {
    matches!(
        event.kind,
        EventKind::Remove(_) | EventKind::Modify(notify::event::ModifyKind::Name(_))
    ) && event.paths.iter().any(|path| path == root)
}

fn deliver(
    output: &mpsc::SyncSender<Result<Event, String>>,
    event: Result<Event, String>,
    overflow: &AtomicBool,
) -> bool {
    match output.try_send(event) {
        Ok(()) => true,
        Err(mpsc::TrySendError::Full(_)) => {
            overflow.store(true, Ordering::Release);
            true
        }
        Err(mpsc::TrySendError::Disconnected(_)) => false,
    }
}

fn rescan() -> Event {
    let mut event = Event::new(EventKind::Any);
    event.attrs.set_flag(notify::event::Flag::Rescan);
    event
}

impl WatchBackend {
    pub(super) fn new(
        root: PathBuf,
        output: mpsc::SyncSender<Result<Event, String>>,
    ) -> Result<Self, notify::Error> {
        let failed = Arc::new(AtomicBool::new(false));
        let overflow = Arc::new(AtomicBool::new(false));
        // Held only to keep the native watch alive until replaced or stopped.
        let mut _watcher = attach(&root, output.clone(), failed.clone(), overflow.clone())?;
        let mut root_id = daruda_core::path::file_id(&root).ok();
        let (stop, stopped) = mpsc::channel();
        std::thread::spawn(move || {
            let mut delay = Duration::from_secs(1);
            loop {
                match stopped.recv_timeout(delay) {
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    _ => return,
                }
                // Windows keeps watching a renamed root through its old handle
                // and reports no error; only the name's new identity shows it.
                let current = daruda_core::path::file_id(&root).ok();
                if current.is_none() || root_id != current {
                    failed.store(true, Ordering::Release);
                }
                if overflow.swap(false, Ordering::AcqRel)
                    && !deliver(&output, Ok(rescan()), &overflow)
                {
                    return;
                }
                if !failed.swap(false, Ordering::AcqRel) {
                    delay = Duration::from_secs(1);
                    continue;
                }
                match attach(&root, output.clone(), failed.clone(), overflow.clone()) {
                    Ok(replacement) => {
                        _watcher = replacement;
                        root_id = current;
                        if !deliver(&output, Ok(rescan()), &overflow) {
                            return;
                        }
                        delay = Duration::from_secs(1);
                    }
                    Err(error) => {
                        failed.store(true, Ordering::Release);
                        if !deliver(&output, Err(error.to_string()), &overflow) {
                            return;
                        }
                        delay = (delay * 2).min(Duration::from_secs(30));
                    }
                }
            }
        });
        Ok(Self { _stop: stop })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overflow_is_bounded_and_requests_a_pathless_rescan() {
        let (tx, rx) = mpsc::sync_channel(1);
        let overflow = AtomicBool::new(false);
        assert!(deliver(&tx, Ok(Event::new(EventKind::Any)), &overflow));
        for _ in 0..1000 {
            assert!(deliver(&tx, Ok(Event::new(EventKind::Any)), &overflow));
        }
        assert!(overflow.swap(false, Ordering::AcqRel));
        assert!(rx.try_recv().is_ok());
        assert!(rx.try_recv().is_err());
        assert!(deliver(&tx, Ok(rescan()), &overflow));
        let event = rx.try_recv().unwrap().unwrap();
        assert!(event.need_rescan());
        assert!(event.paths.is_empty());
        drop(rx);
        assert!(!deliver(&tx, Ok(rescan()), &overflow));
    }

    fn event(kind: EventKind, path: &str) -> Event {
        Event::new(kind).add_path(PathBuf::from(path))
    }

    #[test]
    fn resolved_paths_return_in_the_spelling_the_caller_watched() {
        let reroot = Reroot {
            resolved: PathBuf::from("/private/var/repo"),
            given: PathBuf::from("/var/repo"),
        };
        let mut moved = event(EventKind::Any, "/private/var/repo/src/a.rs");
        moved.paths.push(PathBuf::from("/private/var/repo"));
        moved.paths.push(PathBuf::from("/elsewhere/b.rs"));
        reroot.apply(&mut moved);
        assert_eq!(
            moved.paths,
            [
                PathBuf::from("/var/repo/src/a.rs"),
                PathBuf::from("/var/repo"),
                PathBuf::from("/elsewhere/b.rs"),
            ]
        );
    }

    #[test]
    fn a_path_already_in_the_given_spelling_is_left_alone() {
        let reroot = Reroot {
            resolved: PathBuf::from("/a"),
            given: PathBuf::from("/a/b/link"),
        };
        let mut inside = event(EventKind::Any, "/a/b/link/x");
        reroot.apply(&mut inside);
        assert_eq!(inside.paths, [PathBuf::from("/a/b/link/x")]);
    }

    /// The tempdir test re-spells the root only on macOS; a symlink does it
    /// on every Unix, so Linux CI also covers `Reroot::new`.
    #[cfg(unix)]
    #[test]
    fn events_under_a_symlinked_root_keep_the_link_spelling() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        std::fs::create_dir(&target).unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let (tx, rx) = mpsc::sync_channel(256);
        let _backend = WatchBackend::new(link.clone(), tx).unwrap();
        let file = link.join("created.txt");
        std::fs::write(&file, "x").unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            assert!(
                std::time::Instant::now() < deadline,
                "No event arrived in the link's spelling"
            );
            if rx
                .recv_timeout(Duration::from_millis(100))
                .is_ok_and(|event| event.is_ok_and(|event| event.paths.contains(&file)))
            {
                break;
            }
        }
    }

    #[test]
    fn only_a_removal_or_rename_of_the_root_itself_moves_it() {
        use notify::event::{CreateKind, ModifyKind, RemoveKind, RenameMode};
        let root = Path::new("/repo");
        assert!(moves_root(
            &event(EventKind::Remove(RemoveKind::Folder), "/repo"),
            root
        ));
        assert!(moves_root(
            &event(
                EventKind::Modify(ModifyKind::Name(RenameMode::From)),
                "/repo"
            ),
            root
        ));
        assert!(!moves_root(
            &event(EventKind::Remove(RemoveKind::File), "/repo/a.rs"),
            root
        ));
        assert!(!moves_root(
            &event(EventKind::Create(CreateKind::Folder), "/repo"),
            root
        ));
    }

    #[test]
    fn replacing_the_watched_directory_reattaches_to_the_new_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("watched");
        std::fs::create_dir(&root).unwrap();
        let (tx, rx) = mpsc::sync_channel(256);
        let _backend = WatchBackend::new(root.clone(), tx).unwrap();
        std::fs::rename(&root, dir.path().join("old")).unwrap();
        std::fs::create_dir(&root).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            assert!(
                std::time::Instant::now() < deadline,
                "Watcher never reattached"
            );
            if rx
                .recv_timeout(Duration::from_millis(100))
                .is_ok_and(|event| event.is_ok_and(|event| event.need_rescan()))
            {
                break;
            }
        }
        let file = root.join("after.txt");
        std::fs::write(&file, "after reattach").unwrap();
        loop {
            assert!(
                std::time::Instant::now() < deadline,
                "Replacement watcher missed the new file"
            );
            if rx
                .recv_timeout(Duration::from_millis(100))
                .is_ok_and(|event| event.is_ok_and(|event| event.paths.contains(&file)))
            {
                break;
            }
        }
    }
}
