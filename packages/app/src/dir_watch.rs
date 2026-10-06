//! GPUI-free directory watcher resilient to FSEvents rescan drops.
//!
//! On `event.need_rescan()`, the watched anchors are re-enumerated so existing
//! files are re-emitted after sleep/wake. Deletions during the gap are not
//! reconciled here; full "current set" reporting would couple this generic
//! helper to watcher-specific semantics.
//!
//! [`spawn_dir_watcher`] returns a receiver plus a [`DirWatcher`] RAII handle.
//! Watching continues only while that handle is alive; app-global watchers park
//! it, while respawned watchers drop it to stop the old subscription.

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;
use daruda_store::observability::system_info::redact_home;

/// Owns the live notify watcher; dropping it closes downstream receivers.
/// `Box<dyn Watcher>` hides notify's platform-specific concrete type.
pub struct DirWatcher {
    _watcher: Option<Box<dyn notify::Watcher + Send>>,
}

/// Route one notify event to either normal classification or rescan recovery.
fn route<T>(
    event: &notify::Event,
    classify: &impl Fn(&notify::Event) -> Vec<T>,
    rescan: &impl Fn() -> Vec<T>,
) -> Vec<T> {
    if event.need_rescan() {
        rescan()
    } else {
        classify(event)
    }
}

/// Watch `anchors` and deliver classified items while the returned
/// [`DirWatcher`] is kept alive.
///
/// - `classify`: maps a normal event to zero or more items.
/// - `rescan`: called on FSEvents drops; returns current relevant files.
///
/// One bad anchor is logged and skipped; backend init failure returns an inert
/// handle and an empty receiver.
pub fn spawn_dir_watcher<T: Send + 'static>(
    anchors: &[PathBuf],
    mode: notify::RecursiveMode,
    classify: impl Fn(&notify::Event) -> Vec<T> + Send + 'static,
    rescan: impl Fn() -> Vec<T> + Send + 'static,
) -> (mpsc::Receiver<T>, DirWatcher) {
    let (tx, rx) = mpsc::channel();

    let mut watcher: Box<dyn notify::Watcher + Send> =
        match notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
            let Ok(ev) = res else {
                return;
            };
            for item in route(&ev, &classify, &rescan) {
                // SILENT-OK: a dropped receiver means the consumer is gone and
                // this watcher is about to be dropped with it.
                let _ = tx.send(item);
            }
        }) {
            Ok(w) => Box::new(w),
            Err(e) => {
                LogWriter::log(
                    ErrorReport::new("dir watcher disabled — FS watcher init failed")
                        .severity(ErrorSeverity::Warning)
                        .from_error(&e)
                        .at(file!(), line!())
                        .dedup("dir_watch.init")
                        .build(),
                );
                // Inert: the sender was dropped with the failed closure, so the
                // receiver is already disconnected — callers' `try_recv` reads
                // it the same as empty.
                return (rx, DirWatcher { _watcher: None });
            }
        };

    for anchor in anchors {
        // A failed anchor is logged but does not kill the others: an anchor
        // may not exist yet (callers watch the nearest existing ancestor).
        if let Err(e) = watcher.watch(anchor, mode) {
            LogWriter::log(
                ErrorReport::new("dir watcher — could not attach to anchor")
                    .severity(ErrorSeverity::Warning)
                    .from_error(&e)
                    .at(file!(), line!())
                    .with_context("path", redact_home(anchor))
                    .dedup("dir_watch.attach")
                    .build(),
            );
        }
    }

    (
        rx,
        DirWatcher {
            _watcher: Some(watcher),
        },
    )
}

/// Collapse a burst of unit signals into one output per settling `window`.
/// The helper thread exits when either channel side disconnects.
pub fn debounce(raw_rx: mpsc::Receiver<()>, window: Duration) -> mpsc::Receiver<()> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        loop {
            if raw_rx.recv().is_err() {
                break;
            }
            std::thread::sleep(window);
            while raw_rx.try_recv().is_ok() {}
            if tx.send(()).is_err() {
                break;
            }
        }
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::EventKind;
    use notify::event::{CreateKind, Flag};

    #[test]
    fn route_uses_rescan_on_dropped_event() {
        let ev = notify::Event::new(EventKind::Other).set_flag(Flag::Rescan);
        let classify = |_: &notify::Event| vec!["C"];
        let rescan = || vec!["R"];
        assert_eq!(route(&ev, &classify, &rescan), vec!["R"]);
    }

    #[test]
    fn route_uses_classify_on_normal_event() {
        let ev = notify::Event::new(EventKind::Create(CreateKind::Any));
        let classify = |_: &notify::Event| vec!["C"];
        let rescan = || vec!["R"];
        assert_eq!(route(&ev, &classify, &rescan), vec!["C"]);
    }
}
