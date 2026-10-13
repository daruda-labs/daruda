//! Marks NDJSON sink and replay.

use std::fs::{self, OpenOptions};

use tempfile::TempDir;

use daruda_store::marks::ndjson_sink::{NdjsonFileSink, marks_path, replay_iter};

// ---- helpers ----

fn synthetic_line(i: usize) -> String {
    format!(r#"{{"op":"add","v":1,"seq":{i},"id":{i},"kind":"annotation"}}"#)
}

// ---- tests ----

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
