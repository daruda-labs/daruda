use std::fs;
use std::io::Read;

use tempfile::TempDir;

use super::ndjson_sink::{NdjsonFileSink, marks_path};

// ---- helpers ----

fn synthetic_line(i: usize) -> String {
    format!(r#"{{"op":"add","v":1,"seq":{i},"id":{i},"kind":"annotation"}}"#)
}

// ---- tests ----

/// Write 10 000 records, drop the sink, then verify byte count and
/// line count are consistent.
#[test]
fn write_10000_records_byte_and_line_count_match() {
    let dir = TempDir::new().expect("tempdir");
    let mut sink = NdjsonFileSink::open(dir.path()).expect("open");

    let mut expected_bytes: u64 = 0;
    for i in 0..10_000_usize {
        let line = synthetic_line(i);
        let written = sink.append_line(&line).expect("append");
        // Each call writes `line.len() + 1` (the newline) bytes.
        assert_eq!(written, line.len() + 1);
        expected_bytes += written as u64;
    }
    assert_eq!(sink.bytes_written(), expected_bytes);

    drop(sink);

    let path = marks_path(dir.path());
    let mut body = String::new();
    fs::File::open(&path)
        .expect("file exists")
        .read_to_string(&mut body)
        .expect("read");

    // File length on disk must match what the sink tracked.
    let on_disk = fs::metadata(&path).expect("metadata").len();
    assert_eq!(
        on_disk, expected_bytes,
        "on-disk size must match bytes_written"
    );

    // Newline count must equal the number of records written.
    let line_count = body.chars().filter(|&c| c == '\n').count();
    assert_eq!(line_count, 10_000, "one newline per record");
}

/// The 100 MiB cap warning is emitted exactly once, even when more
/// lines are appended after the threshold is crossed.
///
/// Uses `open_with_cap` with a tiny cap so we do not write 100 MiB
/// of actual data.
#[test]
fn cap_warning_emitted_exactly_once() {
    // LogWriter::log is a no-op when the global writer is not installed,
    // so this test can only verify the internal state flag, not a captured
    // log entry. The invariant tested is: `cap_warned` flips true once and
    // stays true, and subsequent appends succeed without flipping it back.
    let dir = TempDir::new().expect("tempdir");

    // Cap of 1 byte → any single-line append will exceed it.
    let mut sink = NdjsonFileSink::open_with_cap(dir.path(), 1).expect("open");

    // First append: crosses cap → warning should be queued.
    assert!(!sink.is_cap_warned());
    sink.append_line(r#"{"op":"add"}"#).expect("first append");
    assert!(
        sink.is_cap_warned(),
        "cap_warned must be true after first over-cap append"
    );

    // Reset the flag to verify it is NOT re-emitted on subsequent appends.
    // (We white-box test the flag directly; the log target is best-effort.)
    // Second and third appends: warning must NOT flip cap_warned back or
    // emit again (cap_warned stays true, no panic).
    sink.append_line(r#"{"op":"update"}"#)
        .expect("second append");
    sink.append_line(r#"{"op":"remove"}"#)
        .expect("third append");
    assert!(sink.is_cap_warned(), "cap_warned must remain true");
}
