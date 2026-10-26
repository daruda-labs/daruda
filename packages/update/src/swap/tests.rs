use super::*;

fn write(path: &Path, body: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, body).unwrap();
}

#[test]
fn the_aside_name_keeps_the_extension_it_had() {
    let named = aside(Path::new("/x/daruda.exe"), 0);

    assert_eq!(
        named,
        PathBuf::from(format!("/x/daruda.exe{ASIDE_SUFFIX}")),
        "replacing the extension would leave a second runnable .exe"
    );
}

/// An archive that is not the package refuses rather than swapping some
/// arbitrary part of itself over a working install.
#[test]
fn an_archive_without_a_single_bundle_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let staging = tmp.path().join("staging");
    std::fs::create_dir_all(staging.join("one")).unwrap();
    std::fs::create_dir_all(staging.join("two")).unwrap();

    assert!(sole_bundle(&staging).is_err());
    assert!(sole_bundle(&tmp.path().join("empty-missing")).is_err());
}

/// A second update before the app restarts: the first aside still holds
/// the running executable, so it cannot be reused or removed.
#[cfg(unix)]
#[test]
fn a_locked_leftover_gets_the_next_name_rather_than_failing() {
    use std::os::unix::fs::PermissionsExt as _;

    let tmp = tempfile::tempdir().unwrap();
    let guarded = tmp.path().join("guarded");
    let exe = guarded.join("daruda.exe");
    write(&exe, "second old");
    // Stands in for a Windows lock: the directory refuses unlink, so the
    // existing aside can be neither reused nor deleted.
    write(
        &guarded.join(format!("daruda.exe{ASIDE_SUFFIX}")),
        "first old",
    );
    std::fs::set_permissions(&guarded, std::fs::Permissions::from_mode(0o500)).unwrap();

    let taken = free_aside(&exe);

    std::fs::set_permissions(&guarded, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(
        taken.unwrap(),
        guarded.join(format!("daruda.exe{ASIDE_SUFFIX}.1")),
        "the occupied name must not be handed out again"
    );
}

thread_local! {
    static REFUSE_PUBLICATION: std::cell::RefCell<Option<PathBuf>> = const {
        std::cell::RefCell::new(None)
    };
}

/// Remove the selected staged file after its old live entry moved aside.
/// Thread-local arming keeps unrelated parallel installs unaffected.
pub(super) fn remove_staged_for_failure(file: &PlannedFile) -> Result<(), UpdateError> {
    let refuse = REFUSE_PUBLICATION.with(|target| {
        let mut target = target.borrow_mut();
        if target.as_ref() == Some(&file.live) {
            target.take();
            true
        } else {
            false
        }
    });
    if refuse {
        std::fs::remove_file(&file.staged).map_err(io)?;
    }
    Ok(())
}

#[test]
fn failed_swap_recovers_before_returning_its_commit_error() {
    let root = tempfile::tempdir().unwrap();
    let bundle = tempfile::tempdir().unwrap();
    // The plan sorts these names: replace, first install, then a failed
    // publication after its old runtime has already moved aside.
    for (name, bytes) in [
        ("app.exe", "new app"),
        ("new.dll", "new dll"),
        ("runtime.dll", "new runtime"),
    ] {
        write(&bundle.path().join(name), bytes);
    }
    write(&root.path().join("app.exe"), "old app");
    write(&root.path().join("runtime.dll"), "old runtime");
    REFUSE_PUBLICATION.with(|target| {
        *target.borrow_mut() = Some(root.path().join("runtime.dll"));
    });

    let result = swap_into(bundle.path(), root.path());
    let unconsumed = REFUSE_PUBLICATION.with(|target| target.borrow_mut().take());
    assert!(
        unconsumed.is_none(),
        "the commit must reach the selected file"
    );
    assert!(matches!(result, Err(UpdateError::Io(_))), "{result:?}");
    assert_eq!(
        std::fs::read_to_string(root.path().join("app.exe")).unwrap(),
        "old app"
    );
    assert_eq!(
        std::fs::read_to_string(root.path().join("runtime.dll")).unwrap(),
        "old runtime"
    );
    assert!(!root.path().join("new.dll").exists());
    for name in [
        "app.exe.daruda-old",
        "runtime.dll.daruda-old",
        "app.exe.daruda-new",
        "new.dll.daruda-new",
        "runtime.dll.daruda-new",
        ".daruda-update-journal.json",
    ] {
        assert!(!root.path().join(name).exists(), "leftover {name}");
    }
}
