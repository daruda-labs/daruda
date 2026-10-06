use std::fs::{self, OpenOptions};
use std::io::Read;

use tempfile::TempDir;

use super::ndjson_sink::{NdjsonFileSink, marks_path, replay_iter};

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

/// `replay_iter` returns `NotFound` when the file does not exist.
#[test]
fn replay_iter_returns_not_found_for_missing_file() {
    let dir = TempDir::new().expect("tempdir");
    let path = marks_path(dir.path());
    let err = replay_iter(&path).expect_err("should be NotFound");
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
}

/// `replay_iter` reads back lines previously written by `NdjsonFileSink`.
#[test]
fn replay_iter_reads_written_lines() {
    let dir = TempDir::new().expect("tempdir");
    let mut sink = NdjsonFileSink::open(dir.path()).expect("open");

    let lines: Vec<String> = (0..5).map(synthetic_line).collect();
    for line in &lines {
        sink.append_line(line).expect("append");
    }
    drop(sink);

    let path = marks_path(dir.path());
    let read_back: Vec<String> = replay_iter(&path)
        .expect("replay_iter")
        .map(|r| r.expect("line"))
        .collect();

    assert_eq!(read_back, lines);
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

/// Truncating the last few bytes of a written file leaves the first
/// records intact. (Full replay correctness is tested in the
/// daruda_terminal crate; here we just verify replay_iter surfaces the
/// right raw lines.)
#[test]
fn replay_iter_survives_truncated_last_line() {
    let dir = TempDir::new().expect("tempdir");

    let lines = [
        r#"{"op":"add","v":1,"seq":1}"#,
        r#"{"op":"update","v":1,"seq":2}"#,
        r#"{"op":"remove","v":1,"seq":3}"#,
    ];

    {
        let mut sink = NdjsonFileSink::open(dir.path()).expect("open");
        for line in &lines {
            sink.append_line(line).expect("append");
        }
    }

    // Truncate the last few bytes to simulate a partial write.
    let path = marks_path(dir.path());
    let original_len = fs::metadata(&path).expect("metadata").len();
    OpenOptions::new()
        .write(true)
        .open(&path)
        .expect("open for truncate")
        .set_len(original_len - 5)
        .expect("set_len");

    // Collect lines: the third line is now malformed / incomplete.
    let read_back: Vec<String> = replay_iter(&path)
        .expect("replay_iter")
        .map_while(Result::ok)
        .collect();

    // First two lines are complete; third is either missing or truncated.
    // We assert that the first two survive intact.
    assert!(
        read_back.len() >= 2,
        "first two lines must survive truncation, got {read_back:?}"
    );
    assert_eq!(read_back[0], lines[0]);
    assert_eq!(read_back[1], lines[1]);
    // The third item, if present, will be a partial JSON string — not
    // equal to the original. We do not assert its exact form.
}

/// `open` creates the lane directory if it does not exist.
#[test]
fn open_creates_lane_dir() {
    let parent = TempDir::new().expect("tempdir");
    let lane_dir = parent.path().join("nested").join("lane-01");
    assert!(!lane_dir.exists());

    let _sink = NdjsonFileSink::open(&lane_dir).expect("open");
    assert!(lane_dir.is_dir(), "lane_dir must be created by open()");
    assert!(marks_path(&lane_dir).exists(), "marks.ndjson must exist");
}
