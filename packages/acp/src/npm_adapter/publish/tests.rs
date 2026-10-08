use super::*;
use crate::preparation::{PreparationError, PreparationKind};

#[test]
fn canceled_publication_does_not_move_source() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let target = root.path().join("target");
    std::fs::create_dir(&source).unwrap();
    let context = PreparationContext::new(&|| true, &|_| {});
    let error = rename(&source, &target, &context).unwrap_err();
    assert_eq!(
        error.downcast_ref::<PreparationError>().unwrap().kind,
        PreparationKind::Canceled
    );
    assert!(source.is_dir());
    assert!(!target.exists());
}

#[test]
fn missing_source_reports_paths_and_does_not_retry() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("missing");
    let target = root.path().join("target");
    let error = rename(&source, &target, &PreparationContext::default()).unwrap_err();
    let detail = format!("{error:#}");
    assert!(detail.contains(&source.display().to_string()));
    assert!(detail.contains(&target.display().to_string()));
    assert!(detail.contains("after 1 attempt(s)"));
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::NotFound
    );
}

#[cfg(windows)]
fn locked_tree(root: &Path) -> (std::path::PathBuf, std::path::PathBuf, std::fs::File) {
    use std::os::windows::fs::OpenOptionsExt as _;

    let source = root.join("source");
    let target = root.join("target");
    std::fs::create_dir(&source).unwrap();
    let entry = source.join("index.js");
    std::fs::write(&entry, "ready").unwrap();
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(entry)
        .unwrap();
    (source, target, lock)
}

#[cfg(windows)]
#[test]
fn publishes_after_real_windows_file_lock_is_released() {
    let root = tempfile::tempdir().unwrap();
    let (source, target, lock) = locked_tree(root.path());
    // Prove the fixture blocks a directory rename before testing recovery.
    assert!(std::fs::rename(&source, &target).is_err());
    let release = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(100));
        drop(lock);
    });
    let result = rename(&source, &target, &PreparationContext::default());
    release.join().unwrap();
    result.unwrap();
    assert_eq!(
        std::fs::read_to_string(target.join("index.js")).unwrap(),
        "ready"
    );
    assert!(!source.exists());
}

#[cfg(windows)]
#[test]
fn persistent_windows_lock_exhausts_retries_without_publishing() {
    let root = tempfile::tempdir().unwrap();
    let (source, target, lock) = locked_tree(root.path());
    let error = rename(&source, &target, &PreparationContext::default()).unwrap_err();
    assert!(error.to_string().contains("after 6 attempt(s)"));
    assert!(error.downcast_ref::<std::io::Error>().is_some());
    assert!(source.is_dir());
    assert!(!target.exists());
    drop(lock);
    assert_eq!(
        std::fs::read_to_string(source.join("index.js")).unwrap(),
        "ready"
    );
}

#[cfg(windows)]
#[test]
fn cancellation_during_windows_contention_stops_retries() {
    let root = tempfile::tempdir().unwrap();
    let (source, target, _lock) = locked_tree(root.path());
    let checks = std::cell::Cell::new(0);
    let canceled = || {
        checks.set(checks.get() + 1);
        checks.get() > 1
    };
    let context = PreparationContext::new(&canceled, &|_| {});
    let error = rename(&source, &target, &context).unwrap_err();
    assert_eq!(
        error.downcast_ref::<PreparationError>().unwrap().kind,
        PreparationKind::Canceled
    );
    assert!(source.is_dir());
    assert!(!target.exists());
}
