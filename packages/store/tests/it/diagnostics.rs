//! Diagnostic log export.

use chrono::Utc;
use daruda_store::observability::diagnostics::*;
use daruda_store::observability::error_report::ErrorReport;
use std::fs::File;
use std::io::Read as _;

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
