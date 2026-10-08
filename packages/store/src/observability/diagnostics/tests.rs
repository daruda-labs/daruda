use super::super::error_report::ErrorReport;
use super::filter::public_event;
use super::selection::log_order;
use super::*;
use serde_json::json;
use std::fs::File;
use std::io::{Read as _, Seek as _, SeekFrom, Write as _};

fn recent_events(logs: &Path, now: DateTime<Utc>) -> std::io::Result<Vec<String>> {
    recent_events_from(&[logs.to_path_buf()], now)
}

fn recent_events_from(sources: &[PathBuf], now: DateTime<Utc>) -> std::io::Result<Vec<String>> {
    let sources: Vec<_> = sources.iter().cloned().map(LogSource::Current).collect();
    filter::recent_events(&selection::files(&sources)?, now)
}

#[test]
fn broken_compatibility_source_does_not_block_current_export() {
    let root = tempfile::tempdir().unwrap();
    let native = root.path().join("native");
    std::fs::create_dir(&native).unwrap();
    let legacy = root.path().join("legacy");
    std::fs::write(&legacy, "not a directory").unwrap();
    let now = Utc::now();
    let mut report = ErrorReport::new("current").message("ENOENT").build();
    report.timestamp = now;
    std::fs::write(
        native.join(format!("daruda-{}.log", now.date_naive())),
        report.to_ndjson_line(),
    )
    .unwrap();
    let repository = LogRepository {
        directories: vec![
            LogSource::Current(native),
            LogSource::Compatibility(legacy.clone()),
        ],
    };
    let output = root.path().join("result.zip");
    repository.export(&output, now).unwrap();
    let mut archive = zip::ZipArchive::new(File::open(output).unwrap()).unwrap();
    let mut events = String::new();
    archive
        .by_name("events.ndjson")
        .unwrap()
        .read_to_string(&mut events)
        .unwrap();
    assert!(events.contains("ENOENT"));
    assert!(export(&legacy, &root.path().join("invalid.zip"), now).is_err());
}

#[test]
fn compatibility_files_disappearing_after_discovery_are_optional() {
    for compatibility in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let now = Utc::now();
        let path = root.path().join(format!("daruda-{}.log", now.date_naive()));
        std::fs::write(&path, "temporary").unwrap();
        let source = if compatibility {
            LogSource::Compatibility(root.path().into())
        } else {
            LogSource::Current(root.path().into())
        };
        let selected = selection::files(&[source]).unwrap();
        std::fs::remove_file(path).unwrap();
        let result = filter::recent_events(&selected, now);
        if compatibility {
            assert!(result.unwrap().is_empty());
        } else {
            assert!(result.is_err());
        }
    }
}

#[test]
fn native_and_legacy_logs_share_one_global_file_budget() {
    let root = tempfile::tempdir().unwrap();
    let native = root.path().join("native");
    let legacy = root.path().join("legacy");
    std::fs::create_dir(&native).unwrap();
    std::fs::create_dir(&legacy).unwrap();
    let now = Utc::now();
    for (directory, code, start) in [(&native, "ENOENT", 0), (&legacy, "EACCES", FILE_COUNT)] {
        for ordinal in start..start + FILE_COUNT {
            let mut report = ErrorReport::new("event").message(code).build();
            report.timestamp = now;
            std::fs::write(
                directory.join(format!("daruda-{}.{ordinal:03}.log", now.date_naive())),
                serde_json::to_vec(&report).unwrap(),
            )
            .unwrap();
        }
    }
    let events = recent_events_from(&[native, legacy], now).unwrap();
    assert_eq!(events.len(), FILE_COUNT);
    assert!(events.iter().all(|event| event.contains("EACCES")));
}

#[test]
fn missing_source_is_normal_but_unreadable_source_is_an_error() {
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join("not-created-yet");
    assert!(
        recent_events_from(&[missing], Utc::now())
            .unwrap()
            .is_empty()
    );
    let file = root.path().join("not-a-directory");
    std::fs::write(&file, "data").unwrap();
    assert!(recent_events_from(&[file], Utc::now()).is_err());
}

#[test]
fn both_log_generations_are_projected_without_moving_originals() {
    let root = tempfile::tempdir().unwrap();
    let sources: Vec<_> = ["native", "legacy"]
        .map(|name| root.path().join(name))
        .into();
    let now = Utc::now();
    for (source, code) in sources.iter().zip(["ENOENT", "EACCES"]) {
        std::fs::create_dir(source).unwrap();
        let mut report = ErrorReport::new("private title").message(code).build();
        report.timestamp = now;
        std::fs::write(
            source.join(format!("daruda-{}.log", now.date_naive())),
            serde_json::to_vec(&report).unwrap(),
        )
        .unwrap();
    }
    let output = root.path().join("export.zip");
    LogRepository {
        directories: vec![
            LogSource::Current(sources[0].clone()),
            LogSource::Compatibility(sources[1].clone()),
        ],
    }
    .export(&output, now)
    .unwrap();
    let mut archive = zip::ZipArchive::new(File::open(output).unwrap()).unwrap();
    let mut events = String::new();
    archive
        .by_name("events.ndjson")
        .unwrap()
        .read_to_string(&mut events)
        .unwrap();
    assert_eq!(events.lines().count(), 2);
    assert!(events.contains("ENOENT") && events.contains("EACCES"));
    assert!(!events.contains("private"));
    for source in sources {
        assert!(
            source
                .join(format!("daruda-{}.log", now.date_naive()))
                .exists()
        );
    }
}

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
fn codes_require_token_boundaries_in_every_allowed_source() {
    for (text, expected) in [
        ("os error 50", json!([])),
        ("os error 320", json!([])),
        ("os error 330", json!([])),
        ("os error 2060", json!([])),
        ("xos error 5", json!([])),
        ("os error 5_suffix", json!([])),
        ("ENOENT_SUFFIX", json!([])),
        ("PREFIX_EACCES", json!([])),
        ("xEPERM ETIMEDOUTx", json!([])),
        ("오류ENOENT문자열", json!([])),
        ("os error 5", json!(["os error 5"])),
        ("failed (os error 32).", json!(["os error 32"])),
        ("ENOENT: no such file", json!(["ENOENT"])),
        ("[EACCES], EPERM", json!(["EACCES", "EPERM"])),
        ("os error 50; then (os error 5)", json!(["os error 5"])),
        ("ENOENT_SUFFIX; ENOENT", json!(["ENOENT"])),
    ] {
        for source in ["message", "source_chain", "detail"] {
            let mut report = ErrorReport::new("test").build();
            match source {
                "message" => report.message = text.into(),
                "source_chain" => report.source_chain.push(text.into()),
                "detail" => {
                    report.context.insert("detail".into(), text.into());
                }
                _ => unreachable!(),
            }
            assert_eq!(
                public_event(&report)["error_codes"],
                expected,
                "{source}: {text}"
            );
        }
    }
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
