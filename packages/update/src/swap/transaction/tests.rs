use super::*;

#[cfg(unix)]
#[test]
fn interrupted_swap_restores_a_dangling_live_symlink() {
    let root = tempfile::tempdir().unwrap();
    let bundle = tempfile::tempdir().unwrap();
    let live = root.path().join("app.exe");
    let missing = root.path().join("missing.exe");
    daruda_core::path::symlink(&missing, &live).unwrap();
    std::fs::write(bundle.path().join("app.exe"), "replacement").unwrap();

    install(bundle.path(), root.path()).unwrap();
    // A crash after the last rename but before Committed is published
    // leaves the same files with a Prepared journal.
    let mut journal: Journal =
        serde_json::from_slice(&std::fs::read(root.path().join(JOURNAL)).unwrap()).unwrap();
    journal.phase = Phase::Prepared;
    publish(root.path(), &journal).unwrap();
    recover_update(root.path()).unwrap();

    assert_eq!(std::fs::read_link(&live).unwrap(), missing);
    assert!(std::fs::symlink_metadata(root.path().join("app.exe.daruda-old")).is_err());
    assert!(!root.path().join(JOURNAL).exists());
    install(bundle.path(), root.path()).unwrap();
    recover_update(root.path()).unwrap();
    assert_eq!(std::fs::read_to_string(live).unwrap(), "replacement");
    assert!(std::fs::symlink_metadata(root.path().join("app.exe.daruda-old")).is_err());
}

#[test]
fn failed_commit_recovers_replacements_and_first_install_files() {
    let root = tempfile::tempdir().unwrap();
    let replaced = root.path().join("app.exe");
    let added = root.path().join("new.dll");
    let refused = root.path().join("runtime.dll");
    std::fs::write(&replaced, "old app").unwrap();
    std::fs::write(&refused, "old runtime").unwrap();
    std::fs::write(staged_path(&replaced), "new app").unwrap();
    std::fs::write(staged_path(&added), "new dll").unwrap();
    let plan = vec![
        PlannedFile {
            source: PathBuf::new(),
            staged: staged_path(&replaced),
            aside: Some(root.path().join("app.exe.daruda-old.1")),
            live: replaced.clone(),
        },
        PlannedFile {
            source: PathBuf::new(),
            staged: staged_path(&added),
            aside: None,
            live: added.clone(),
        },
        PlannedFile {
            source: PathBuf::new(),
            staged: staged_path(&refused),
            aside: Some(root.path().join("runtime.dll.daruda-old")),
            live: refused.clone(),
        },
    ];
    // The third replacement is missing: commit fails after publishing
    // a replacement and a previously absent file, and moving its own old file.
    publish(
        root.path(),
        &Journal {
            phase: Phase::Prepared,
            files: vec![
                Entry {
                    live: "app.exe".into(),
                    aside: Some("app.exe.daruda-old.1".into()),
                },
                Entry {
                    live: "new.dll".into(),
                    aside: None,
                },
                Entry {
                    live: "runtime.dll".into(),
                    aside: Some("runtime.dll.daruda-old".into()),
                },
            ],
        },
    )
    .unwrap();
    assert!(commit(&plan).is_err());
    assert_eq!(
        std::fs::read_to_string(root.path().join("app.exe.daruda-old.1")).unwrap(),
        "old app"
    );
    recover_update(root.path()).unwrap();

    assert_eq!(std::fs::read_to_string(replaced).unwrap(), "old app");
    assert_eq!(std::fs::read_to_string(refused).unwrap(), "old runtime");
    assert!(!added.exists());
    assert!(!root.path().join("app.exe.daruda-new").exists());
    assert!(!root.path().join("new.dll.daruda-new").exists());
    assert!(!root.path().join(JOURNAL).exists());
}

#[test]
fn legacy_journals_keep_their_recovery_phase() {
    for (flags, expected) in [
        (serde_json::json!({"committed": false}), Phase::Staging),
        (serde_json::json!({"committed": true}), Phase::Committed),
        (
            serde_json::json!({"committed": false, "prepared": false}),
            Phase::Staging,
        ),
        (
            serde_json::json!({"committed": false, "prepared": true}),
            Phase::Prepared,
        ),
        (
            serde_json::json!({"committed": true, "prepared": false}),
            Phase::Committed,
        ),
        (
            serde_json::json!({"committed": true, "prepared": true}),
            Phase::Committed,
        ),
    ] {
        let mut value = flags;
        value["files"] = serde_json::json!([]);
        let journal: Journal = serde_json::from_value(value).expect("legacy journal");
        assert_eq!(journal.phase, expected);
    }
}

#[test]
fn new_journals_remain_readable_by_the_legacy_format() {
    for phase in [Phase::Staging, Phase::Prepared, Phase::Committed] {
        let encoded = serde_json::to_value(Journal {
            phase,
            files: vec![],
        })
        .expect("serialize journal");
        assert!(encoded.get("phase").is_none());
        assert_eq!(encoded["committed"], phase == Phase::Committed);
        assert_eq!(encoded["prepared"], phase != Phase::Staging);
        let decoded: Journal = serde_json::from_value(encoded).expect("round trip");
        assert_eq!(decoded.phase, phase);
    }
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
    let result = install(&bundle, &root);
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
    let result = install(&bundle, &root);
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
    assert!(install(bundle.path(), root.path()).is_err());
    assert!(!root.path().join(JOURNAL).exists());
    std::fs::write(root.path().join("app.exe"), "preserve").unwrap();
    publish(
        root.path(),
        &Journal {
            phase: Phase::Prepared,
            files: vec![
                Entry {
                    live: "app.exe".into(),
                    aside: None,
                },
                Entry {
                    live: "APP.EXE".into(),
                    aside: None,
                },
            ],
        },
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
        &Journal {
            phase: Phase::Prepared,
            files: vec![
                Entry {
                    live: "app.exe".into(),
                    aside: Some("app.exe.daruda-old".into()),
                },
                Entry {
                    live: "runtime.dll".into(),
                    aside: Some("runtime.dll.daruda-old".into()),
                },
            ],
        },
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
        &Journal {
            phase: Phase::Prepared,
            files: vec![Entry {
                live: "../escape".into(),
                aside: None,
            }],
        },
    )
    .unwrap();
    assert!(recover_update(root.path()).is_err());
    assert!(root.path().join(JOURNAL).exists());
}
