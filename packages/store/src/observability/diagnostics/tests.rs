use super::*;

#[test]
fn exported_events_exclude_secrets_and_absolute_paths() {
    let root = tempfile::tempdir().unwrap();
    let report = ErrorReport::new("secret title")
        .message("ENOENT Bearer secret-token")
        .with_context("api_key", "secret-key")
        .location("C:\\Users\\private\\source.rs:1")
        .build();
    std::fs::write(
        root.path()
            .join(format!("daruda-{}.log", report.timestamp.date_naive())),
        serde_json::to_vec(&report).unwrap(),
    )
    .unwrap();
    std::fs::write(root.path().join("acp-wire.log"), "private transcript").unwrap();
    let output = root.path().join("diagnostics.zip");
    export(root.path(), &output, Utc::now()).unwrap();
    let mut archive = zip::ZipArchive::new(File::open(&output).unwrap()).unwrap();
    assert_eq!(archive.len(), 2);
    let mut events = String::new();
    archive
        .by_name("events.ndjson")
        .unwrap()
        .read_to_string(&mut events)
        .unwrap();
    assert!(events.contains("ENOENT"));
    for private in ["secret", "private", "Bearer", "api_key", "Users"] {
        assert!(!events.contains(private));
    }
    assert!(export(root.path(), &output, Utc::now()).is_err());
}

#[test]
fn context_detail_preserves_only_allowlisted_codes() {
    let report = ErrorReport::new("ACP connection failed")
        .with_context("detail", "access denied (os error 5), secret-token")
        .with_context("api_key", "EACCES")
        .build();
    let event = public_event(&report);
    assert_eq!(event["error_codes"], json!(["os error 5"]));
    assert!(!event.to_string().contains("secret-token"));
    assert!(!event.to_string().contains("access denied"));
}

#[test]
fn rotated_and_same_file_recent_events_survive_the_event_cap() {
    for rotated in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let now = Utc::now();
        let mut old = ErrorReport::new("old").message("ENOENT").build();
        old.timestamp = now - chrono::Duration::seconds(1);
        let mut recent = ErrorReport::new("latest").message("ECONNRESET").build();
        recent.timestamp = now;
        let base = root.path().join(format!("daruda-{}.log", now.date_naive()));
        let mut contents = (serde_json::to_string(&old).unwrap() + "\n").repeat(EVENT_LIMIT);
        let latest = serde_json::to_string(&recent).unwrap() + "\n";
        if rotated {
            std::fs::write(
                root.path()
                    .join(format!("daruda-{}.001.log", now.date_naive())),
                latest,
            )
            .unwrap();
        } else {
            contents.push_str(&latest);
        }
        std::fs::write(base, contents).unwrap();
        let output = root.path().join("result.zip");
        export(root.path(), &output, now).unwrap();
        let mut archive = zip::ZipArchive::new(File::open(output).unwrap()).unwrap();
        let mut events = String::new();
        archive
            .by_name("events.ndjson")
            .unwrap()
            .read_to_string(&mut events)
            .unwrap();
        assert_eq!(events.lines().count(), EVENT_LIMIT);
        assert!(events.contains("ECONNRESET"));
    }
}

#[test]
fn byte_limit_reads_the_tail_and_rotation_order_is_numeric() {
    let root = tempfile::tempdir().unwrap();
    let now = Utc::now();
    let date = now.date_naive();
    for ordinal in 0..=FILE_COUNT {
        let path = root.path().join(format!("daruda-{date}.{ordinal:03}.log"));
        let mut report = ErrorReport::new("event").message("ENOENT").build();
        report.timestamp = now;
        if ordinal == FILE_COUNT {
            report.message = "ECONNRESET".into();
        }
        let mut file = File::create(path).unwrap();
        if ordinal == FILE_COUNT {
            // Seek makes a sparse oversized, malformed first record.
            file.seek(SeekFrom::Start(FILE_LIMIT + 128)).unwrap();
            file.write_all(b"\n").unwrap();
        }
        serde_json::to_writer(&mut file, &report).unwrap();
        file.write_all(b"\n").unwrap();
    }
    let events = recent_events(root.path(), now).unwrap();
    assert_eq!(events.len(), FILE_COUNT);
    assert!(events.iter().any(|event| event.contains("ECONNRESET")));
    assert!(log_order("daruda-test.log").is_none());
    assert!(log_order("acp-wire.log").is_none());
}
