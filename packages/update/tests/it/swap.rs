use daruda_update::swap::{install_zip, swap_into, sweep_aside};
use std::path::{Path, PathBuf};

const ASIDE_SUFFIX: &str = ".daruda-old";
#[cfg(unix)]
const STAGED_SUFFIX: &str = ".daruda-new";

fn write(path: &Path, body: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, body).unwrap();
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap()
}

/// A bundle shaped like the published one: the executable, a runtime
/// library beside it, and a licenses directory.
fn bundle(root: &Path) -> PathBuf {
    let bundle = root.join("bundle");
    write(&bundle.join("daruda.exe"), "new exe");
    write(&bundle.join("vcruntime140.dll"), "new dll");
    write(
        &bundle.join("licenses").join("third-party.md"),
        "new notice",
    );
    bundle
}

#[test]
fn every_file_lands_and_the_old_one_is_kept_aside() {
    let tmp = tempfile::tempdir().unwrap();
    let bundle = bundle(tmp.path());
    let install = tmp.path().join("install");
    write(&install.join("daruda.exe"), "old exe");
    write(&install.join("vcruntime140.dll"), "old dll");

    swap_into(&bundle, &install).unwrap();

    assert_eq!(read(&install.join("daruda.exe")), "new exe");
    assert_eq!(read(&install.join("vcruntime140.dll")), "new dll");
    assert_eq!(
        read(&install.join("licenses").join("third-party.md")),
        "new notice"
    );
    assert_eq!(
        read(&install.join(format!("daruda.exe{ASIDE_SUFFIX}"))),
        "old exe",
        "the running executable has to survive under another name"
    );
}

/// The point of renaming rather than writing: on Windows the destination
/// may be mapped into this process, and only the *name* is free.
#[test]
fn nothing_is_written_over_in_place() {
    let tmp = tempfile::tempdir().unwrap();
    let bundle = bundle(tmp.path());
    let install = tmp.path().join("install");
    let exe = install.join("daruda.exe");
    write(&exe, "old exe");
    let before = std::fs::metadata(&exe).unwrap().len();

    swap_into(&bundle, &install).unwrap();

    // The old bytes are intact at the aside name — proof the file was
    // moved, not truncated and rewritten.
    let aside = install.join(format!("daruda.exe{ASIDE_SUFFIX}"));
    assert_eq!(std::fs::metadata(&aside).unwrap().len(), before);
    assert_eq!(read(&aside), "old exe");
}

#[test]
fn a_first_install_needs_nothing_to_move_aside() {
    let tmp = tempfile::tempdir().unwrap();
    let bundle = bundle(tmp.path());
    let install = tmp.path().join("install");
    std::fs::create_dir_all(&install).unwrap();

    swap_into(&bundle, &install).unwrap();

    assert_eq!(read(&install.join("daruda.exe")), "new exe");
    assert!(!install.join(format!("daruda.exe{ASIDE_SUFFIX}")).exists());
}

/// Where a swap actually fails — a short disk, a permission, a truncated
/// archive — is the copy, and none of it may reach the install.
///
/// The fixture fails on the *second* file, which is the case that used to
/// corrupt: the first had already gone live, so the install was left part
/// new and part old with no way back.
#[cfg(unix)]
#[test]
fn a_copy_that_fails_partway_leaves_the_install_exactly_as_it_was() {
    use std::os::unix::fs::PermissionsExt as _;

    let tmp = tempfile::tempdir().unwrap();
    let bundle = bundle(tmp.path());
    let install = tmp.path().join("install");
    write(&install.join("daruda.exe"), "old exe");
    let licenses = install.join("licenses");
    write(&licenses.join("third-party.md"), "old notice");
    std::fs::set_permissions(&licenses, std::fs::Permissions::from_mode(0o500)).unwrap();

    let refused = swap_into(&bundle, &install);

    std::fs::set_permissions(&licenses, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(refused.is_err(), "a copy that cannot land must refuse");
    assert_eq!(
        read(&install.join("daruda.exe")),
        "old exe",
        "the file that would have gone first must not be live"
    );
    assert_eq!(read(&licenses.join("third-party.md")), "old notice");
    assert!(
        !install.join(format!("daruda.exe{ASIDE_SUFFIX}")).exists(),
        "nothing may have been moved aside"
    );
    assert!(
        !install.join(format!("daruda.exe{STAGED_SUFFIX}")).exists(),
        "the copies made before the failure must be cleared"
    );
}

/// A read-only install is the common Windows case (`C:\\Program Files`
/// without elevation). It has to fail before the first rename, or the
/// user is left with an install missing its executable.
#[cfg(unix)]
#[test]
fn a_read_only_install_is_refused_with_nothing_moved() {
    use std::os::unix::fs::PermissionsExt as _;

    let tmp = tempfile::tempdir().unwrap();
    let bundle = bundle(tmp.path());
    let install = tmp.path().join("install");
    write(&install.join("daruda.exe"), "old exe");
    std::fs::set_permissions(&install, std::fs::Permissions::from_mode(0o500)).unwrap();

    let refused = swap_into(&bundle, &install);

    std::fs::set_permissions(&install, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(refused.is_err());
    assert_eq!(
        read(&install.join("daruda.exe")),
        "old exe",
        "a refused swap must leave the install exactly as it was"
    );
}

/// The published shape, end to end: a real archive unpacked and swapped.
/// bsdtar reads zip on every host daruda builds for, so this exercises
/// the Windows path from a macOS run.
#[test]
fn a_packaged_archive_replaces_the_install_it_is_pointed_at() {
    let tmp = tempfile::tempdir().unwrap();
    bundle(tmp.path());
    let zip = tmp.path().join("daruda-9.9.9-windows-x86_64.zip");
    let packed = daruda_core::process::command("tar")
        .arg("-a")
        .arg("-cf")
        .arg(&zip)
        .arg("-C")
        .arg(tmp.path())
        .arg("bundle")
        .status()
        .unwrap();
    assert!(packed.success(), "the fixture archive must be written");

    let install = tmp.path().join("install");
    write(&install.join("daruda.exe"), "old exe");

    install_zip(&zip, &install).unwrap();

    assert_eq!(read(&install.join("daruda.exe")), "new exe");
    assert_eq!(
        read(&install.join(format!("daruda.exe{ASIDE_SUFFIX}"))),
        "old exe"
    );
}

/// The marker is read off each file's own name. An install directory
/// whose *path* carries it keeps every file; a numbered aside still goes.
#[test]
fn a_sweep_judges_the_file_name_not_the_path() {
    let tmp = tempfile::tempdir().unwrap();
    let install = tmp.path().join(format!("tools{ASIDE_SUFFIX}"));
    write(&install.join("daruda.exe"), "live");
    write(
        &install.join(format!("daruda.exe{ASIDE_SUFFIX}.2")),
        "stale",
    );
    write(
        &install.join(format!("notes{ASIDE_SUFFIX}.txt")),
        "user file",
    );

    sweep_aside(&install);

    assert_eq!(read(&install.join("daruda.exe")), "live");
    assert!(!install.join(format!("daruda.exe{ASIDE_SUFFIX}.2")).exists());
    assert_eq!(
        read(&install.join(format!("notes{ASIDE_SUFFIX}.txt"))),
        "user file"
    );
}

/// An aside file under a subdirectory is one the swap itself created.
#[test]
fn a_sweep_reaches_the_nested_ones_too() {
    let tmp = tempfile::tempdir().unwrap();
    let nested = tmp.path().join("licenses");
    write(
        &nested.join(format!("third-party.md{ASIDE_SUFFIX}")),
        "stale",
    );
    write(&nested.join("third-party.md"), "live");

    sweep_aside(tmp.path());

    assert!(
        !nested
            .join(format!("third-party.md{ASIDE_SUFFIX}"))
            .exists()
    );
    assert_eq!(read(&nested.join("third-party.md")), "live");
}

#[test]
fn a_sweep_removes_what_an_earlier_swap_left() {
    let tmp = tempfile::tempdir().unwrap();
    let install = tmp.path();
    write(&install.join(format!("daruda.exe{ASIDE_SUFFIX}")), "stale");
    write(&install.join("daruda.exe"), "live");

    sweep_aside(install);

    assert!(!install.join(format!("daruda.exe{ASIDE_SUFFIX}")).exists());
    assert_eq!(
        read(&install.join("daruda.exe")),
        "live",
        "the live install is untouched"
    );
}
