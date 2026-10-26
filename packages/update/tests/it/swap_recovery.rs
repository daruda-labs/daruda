use daruda_update::swap::{recover_update, swap_into};
use std::path::Path;

const JOURNAL: &str = ".daruda-update-journal.json";

/// Plant the durable input an interrupted installer left behind.
fn publish(root: &Path, journal: &serde_json::Value) -> std::io::Result<()> {
    std::fs::write(root.join(JOURNAL), serde_json::to_vec(journal)?)
}

#[cfg(unix)]
#[test]
fn interrupted_swap_restores_a_dangling_live_symlink() {
    let root = tempfile::tempdir().unwrap();
    let bundle = tempfile::tempdir().unwrap();
    let live = root.path().join("app.exe");
    let missing = root.path().join("missing.exe");
    daruda_core::path::symlink(&missing, &live).unwrap();
    std::fs::write(bundle.path().join("app.exe"), "replacement").unwrap();

    swap_into(bundle.path(), root.path()).unwrap();
    // A crash after the last rename but before Committed is published
    // leaves the same files with a Prepared journal.
    let mut journal: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.path().join(JOURNAL)).unwrap()).unwrap();
    journal["committed"] = false.into();
    journal["prepared"] = true.into();
    publish(root.path(), &journal).unwrap();
    recover_update(root.path()).unwrap();

    assert_eq!(std::fs::read_link(&live).unwrap(), missing);
    assert!(std::fs::symlink_metadata(root.path().join("app.exe.daruda-old")).is_err());
    assert!(!root.path().join(JOURNAL).exists());
    swap_into(bundle.path(), root.path()).unwrap();
    recover_update(root.path()).unwrap();
    assert_eq!(std::fs::read_to_string(live).unwrap(), "replacement");
    assert!(std::fs::symlink_metadata(root.path().join("app.exe.daruda-old")).is_err());
}

#[test]
fn install_refuses_symlinked_parents_before_staging() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("install");
    let bundle = dir.path().join("bundle");
    let outside = dir.path().join("outside");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(bundle.join("licenses")).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("notice.txt"), "preserve").unwrap();
    std::fs::write(bundle.join("licenses/notice.txt"), "replacement").unwrap();
    let link = root.join("licenses");
    daruda_core::path::symlink(&outside, &link).unwrap();
    let result = swap_into(&bundle, &root);
    daruda_core::path::remove_symlink(&link).unwrap();
    assert!(result.is_err());
    assert_eq!(
        std::fs::read_to_string(outside.join("notice.txt")).unwrap(),
        "preserve"
    );
    assert!(!outside.join("notice.txt.daruda-new").exists());
    assert!(!root.join(JOURNAL).exists());
}

#[test]
fn staging_does_not_follow_an_existing_destination_symlink() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("install");
    let bundle = dir.path().join("bundle");
    let outside = dir.path().join("outside.txt");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&bundle).unwrap();
    std::fs::write(&outside, "preserve").unwrap();
    std::fs::write(bundle.join("app.exe"), "replacement").unwrap();
    let link = root.join("app.exe.daruda-new");
    daruda_core::path::symlink(&outside, &link).unwrap();
    let result = swap_into(&bundle, &root);
    assert!(result.is_err());
    assert_eq!(std::fs::read_to_string(&outside).unwrap(), "preserve");
    assert!(!root.join("app.exe").exists());
}

#[cfg(windows)]
#[test]
fn windows_reserved_paths_are_case_insensitive() {
    let root = tempfile::tempdir().unwrap();
    let bundle = tempfile::tempdir().unwrap();
    std::fs::write(bundle.path().join(".DARUDA-UPDATE-JOURNAL.JSON"), "package").unwrap();
    assert!(swap_into(bundle.path(), root.path()).is_err());
    assert!(!root.path().join(JOURNAL).exists());
    std::fs::write(root.path().join("app.exe"), "preserve").unwrap();
    publish(
        root.path(),
        &serde_json::json!({
            "committed": false,
            "prepared": true,
            "files": [
                { "live": "app.exe", "aside": null },
                { "live": "APP.EXE", "aside": null },
            ],
        }),
    )
    .unwrap();
    assert!(recover_update(root.path()).is_err());
    assert_eq!(
        std::fs::read_to_string(root.path().join("app.exe")).unwrap(),
        "preserve"
    );
    assert!(root.path().join(JOURNAL).exists());
}

#[test]
fn interrupted_mixed_install_restores_every_old_file() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("app.exe"), "new").unwrap();
    std::fs::write(root.path().join("app.exe.daruda-old"), "old").unwrap();
    std::fs::write(root.path().join("runtime.dll.daruda-new"), "new runtime").unwrap();
    std::fs::write(root.path().join("runtime.dll"), "old runtime").unwrap();
    publish(
        root.path(),
        &serde_json::json!({
            "committed": false,
            "prepared": true,
            "files": [
                { "live": "app.exe", "aside": "app.exe.daruda-old" },
                { "live": "runtime.dll", "aside": "runtime.dll.daruda-old" },
            ],
        }),
    )
    .unwrap();
    recover_update(root.path()).unwrap();
    assert_eq!(
        std::fs::read_to_string(root.path().join("app.exe")).unwrap(),
        "old"
    );
    assert_eq!(
        std::fs::read_to_string(root.path().join("runtime.dll")).unwrap(),
        "old runtime"
    );
    assert!(!root.path().join(JOURNAL).exists());
}

#[test]
fn journal_cannot_modify_files_outside_the_install() {
    let root = tempfile::tempdir().unwrap();
    publish(
        root.path(),
        &serde_json::json!({
            "committed": false,
            "prepared": true,
            "files": [{ "live": "../escape", "aside": null }],
        }),
    )
    .unwrap();
    assert!(recover_update(root.path()).is_err());
    assert!(root.path().join(JOURNAL).exists());
}
