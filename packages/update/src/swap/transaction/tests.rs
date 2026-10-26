use super::*;

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
