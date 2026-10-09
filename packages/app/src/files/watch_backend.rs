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
    let watched_root = root.to_path_buf();
    let mut watcher = notify::recommended_watcher(move |event: Result<Event, notify::Error>| {
        let root_moved = event.as_ref().is_ok_and(|event| {
            matches!(
                event.kind,
                EventKind::Remove(_) | EventKind::Modify(notify::event::ModifyKind::Name(_))
            ) && event.paths.contains(&watched_root)
        });
        if event.is_err() || root_moved {
            failed.store(true, Ordering::Release);
        }
        deliver(&output, event.map_err(|error| error.to_string()), &overflow);
    })?;
    watcher.watch(root, RecursiveMode::Recursive)?;
    Ok(watcher)
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
        let mut watcher = attach(&root, output.clone(), failed.clone(), overflow.clone())?;
        let mut root_created = std::fs::metadata(&root)
            .and_then(|metadata| metadata.created())
            .ok();
        let (stop, stopped) = mpsc::channel();
        std::thread::spawn(move || {
            let mut delay = Duration::from_secs(1);
            loop {
                match stopped.recv_timeout(delay) {
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    _ => return,
                }
                // Windows can keep watching a renamed root's old file handle.
                // Its creation stamp detects replacement even without a backend error.
                let metadata = std::fs::metadata(&root).ok();
                let created = metadata
                    .as_ref()
                    .and_then(|metadata| metadata.created().ok());
                if metadata.is_none() || root_created != created {
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
                        watcher = replacement;
                        root_created = created;
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
                // Keep ownership until replacement or shutdown.
                let _ = &watcher;
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
