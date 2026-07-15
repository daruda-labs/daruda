//! GPUI-free directory watcher resilient to FSEvents drops (sleep/wake).
//!
//! On `event.need_rescan()` (FSEvents KernelDropped/UserDropped, surfaced by
//! notify as EventKind::Other + Flag::Rescan) the whole watched directory is
//! re-enumerated and re-emitted, so events lost across sleep/wake are not
//! permanently dropped (the same recovery zed performs).
//!
//! LIMITATION: rescan recovery re-emits a `Changed`-style item only for files
//! that still EXIST. A file *removed* during the gap is not reconciled — its
//! consumer-side state lingers until the next real event (for the status
//! watcher this is a ghost session indicator that clears on the next write).
//! Full reconciliation would need a "current set" signal, deferred to avoid
//! coupling this generic to per-watcher semantics.
//!
//! ## Lifetime
//!
//! [`spawn_dir_watcher`] returns the event [`Receiver`] plus a [`DirWatcher`]
//! handle that OWNS the live notify watcher. Watching continues only while the
//! handle is alive — drop it to stop (the OS subscription ends and the
//! callback's sender closes). This RAII shape lets callers that re-spawn on
//! config / project / lane changes (mcp, skills, …) avoid leaking a watcher
//! per re-spawn, and lets app-global watchers (status, panels, …) opt into
//! "never stop" by parking the handle somewhere long-lived. No background
//! thread is spawned here — notify runs its own internal thread.
//!
//! NOTE (long-term): if per-project/per-lane watching multiplies and path
//! dedup becomes a real cost, these watcher instances can be wrapped in a
//! central registry that owns and dedups them (this type is the producer-side
//! subset of that). At today's ~8 fixed watchers dedup buys nothing, so no
//! registry.

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;
use daruda_store::observability::system_info::redact_home;

/// Owns the live notify watcher. Dropping it stops watching: the OS
/// subscription ends and the FS callback's `Sender` closes (so a caller's
/// downstream `Receiver` / debounce thread sees a disconnect and can exit).
///
/// `Option` + `Box<dyn>`: `None` is the inert state when the backend failed to
/// initialize; `Box<dyn Watcher + Send>` hides notify's platform-specific
/// concrete type from the public signature (mirrors `ConfigWatcherHandle`).
pub struct DirWatcher {
    _watcher: Option<Box<dyn notify::Watcher + Send>>,
}

/// Routes a single notify event to either the rescan path or the classify path.
///
/// When `event.need_rescan()` is true (FSEvents KernelDropped/UserDropped),
/// the whole directory is re-enumerated via `rescan()` so events lost during
/// sleep/wake are recovered. Otherwise `classify(event)` handles it normally.
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

/// Watch `anchors` with `notify` and deliver classified items on the returned
/// receiver. The returned [`DirWatcher`] keeps the watch alive — see the module
/// "Lifetime" section.
///
/// - `classify`: maps a normal event to zero or more items.
/// - `rescan`: called instead of classify on an FSEvents drop (sleep/wake,
///   kernel buffer overflow). Should enumerate the directory and return a
///   `Changed`-style item for every relevant file found.
///
/// Per-anchor `watch` failures are tolerated (an anchor may not exist yet —
/// callers watch the nearest existing ancestor — and one bad anchor must not
/// kill the others). Anchor existence, when required, is the caller's
/// responsibility (e.g. the status watcher's `create_dir_all`). If the backend
/// itself fails to initialize, an inert handle (`None`) is returned and the
/// receiver stays empty.
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

/// Collapse a burst of unit signals into one per settling `window`. The first
/// signal starts the window; everything arriving during it coalesces into a
/// single output `()`. Spawns one thread that exits cleanly when `raw_rx`
/// disconnects (the source `DirWatcher` was dropped) or the returned receiver
/// is dropped. Shared by the file-reload watchers (config, panels) whose
/// `classify` emits `()` per relevant change.
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
