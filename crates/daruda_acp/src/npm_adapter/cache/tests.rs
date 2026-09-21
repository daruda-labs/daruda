use super::*;

#[test]
fn the_last_owner_unlocks_even_while_a_spawn_has_duplicated_the_descriptor() {
    let root = tempfile::tempdir().unwrap();
    let lease = InstallationLease::acquire(root.path()).unwrap();
    let inherited = lease._file.0.try_clone().unwrap();
    drop(lease);
    assert!(exclusive_lease(root.path()).unwrap().is_some());
    drop(inherited);
}

#[test]
fn canceled_lock_wait_does_not_block_other_packages() {
    let root = tempfile::tempdir().unwrap();
    let first = root.path().join("first");
    let _held = lock_package(&first, &PreparationContext::default()).unwrap();
    let _other = lock_package(&root.path().join("second"), &PreparationContext::default()).unwrap();
    let started = Instant::now();
    let canceled = || started.elapsed() > Duration::from_millis(30);
    let error = lock_package(&first, &PreparationContext::new(&canceled, &|_| {})).unwrap_err();
    assert_eq!(error.kind, PreparationKind::Canceled);
}

#[test]
fn sweep_preserves_leased_versions_and_unrelated_directories() {
    let root = tempfile::tempdir().unwrap();
    let active = root.path().join("version-active");
    fs::create_dir(&active).unwrap();
    let lease = InstallationLease::acquire(&active).unwrap();
    File::options()
        .write(true)
        .open(active.join(LAST_USED))
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(SystemTime::UNIX_EPOCH))
        .unwrap();
    let clone = lease.clone();
    for i in 0..KEEP_INSTALLATIONS + 2 {
        let path = root.path().join(format!("version-{i}"));
        fs::create_dir(path).unwrap();
    }
    fs::create_dir(root.path().join("unrelated")).unwrap();
    sweep(root.path(), &PreparationContext::default()).unwrap();
    assert!(active.exists());
    drop(lease);
    assert!(exclusive_lease(&active).unwrap().is_none());
    drop(clone);
    assert!(exclusive_lease(&active).unwrap().is_some());
    sweep(root.path(), &PreparationContext::default()).unwrap();
    assert!(!active.exists());
    assert!(root.path().join("unrelated").exists());
}

#[test]
fn quarantine_has_a_bounded_retention_count() {
    let root = tempfile::tempdir().unwrap();
    for i in 0..5 {
        fs::create_dir(root.path().join(format!(".invalid-{i}"))).unwrap();
    }
    sweep(root.path(), &PreparationContext::default()).unwrap();
    assert_eq!(
        fs::read_dir(root.path())
            .unwrap()
            .filter(|entry| {
                entry
                    .as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(INVALID_PREFIX)
            })
            .count(),
        KEEP_INVALID
    );
}

#[cfg(unix)]
#[test]
fn sweep_never_follows_directory_symlinks() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("keep"), "data").unwrap();
    std::os::unix::fs::symlink(outside.path(), root.path().join("version-link")).unwrap();
    sweep_prefix(
        root.path(),
        VERSION_PREFIX,
        0,
        &PreparationContext::default(),
    )
    .unwrap();
    assert!(outside.path().join("keep").exists());
}
