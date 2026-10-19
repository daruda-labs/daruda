//! Public API tests for jsonl tail.

use daruda_agent::jsonl::tail::*;
use std::io::Write;

#[test]
fn empty_file_yields_empty() {
    let dir = tempfile::TempDir::new().unwrap();
    let p = dir.path().join("empty.jsonl");
    std::fs::write(&p, b"").unwrap();
    let lines = read_last_n_lines(&p, 10).unwrap();
    assert!(lines.is_empty());
}

#[test]
fn small_file_returns_last_n() {
    let dir = tempfile::TempDir::new().unwrap();
    let p = dir.path().join("small.jsonl");
    let mut f = std::fs::File::create(&p).unwrap();
    for i in 0..5 {
        writeln!(f, r#"{{"line":{i}}}"#).unwrap();
    }
    drop(f);
    let lines = read_last_n_lines(&p, 3).unwrap();
    assert_eq!(lines.len(), 3);
    assert!(lines[0].contains("\"line\":2"));
    assert!(lines[2].contains("\"line\":4"));
}

#[test]
fn small_file_n_larger_than_lines_returns_all() {
    let dir = tempfile::TempDir::new().unwrap();
    let p = dir.path().join("few.jsonl");
    let mut f = std::fs::File::create(&p).unwrap();
    for i in 0..3 {
        writeln!(f, r#"{{"line":{i}}}"#).unwrap();
    }
    drop(f);
    let lines = read_last_n_lines(&p, 100).unwrap();
    assert_eq!(lines.len(), 3);
}

#[test]
fn blank_lines_filtered_out() {
    let dir = tempfile::TempDir::new().unwrap();
    let p = dir.path().join("blanks.jsonl");
    std::fs::write(&p, "a\n\n\n  \nb\n\nc\n").unwrap();
    let lines = read_last_n_lines(&p, 10).unwrap();
    assert_eq!(
        lines,
        vec!["a".to_string(), "b".to_string(), "c".to_string()]
    );
}

#[test]
fn missing_file_returns_error() {
    let dir = tempfile::TempDir::new().unwrap();
    let p = dir.path().join("nope.jsonl");
    let err = read_last_n_lines(&p, 10).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
}

/// The lookback window `read_last_n_lines` seeks back for `n` lines on a
/// large file. Mirrors the reader's private estimate; a fixture asserts where
/// its seek lands, so a change to the estimate fails loudly here.
fn window_start(size: usize, n: usize) -> usize {
    size - (n * 2048).min(size)
}

#[test]
fn seek_landing_mid_line_drops_the_partial_line() {
    let dir = tempfile::TempDir::new().unwrap();
    let p = dir.path().join("long-lines.jsonl");
    let pad = "x".repeat(3000);
    let content: String = (0..10)
        .map(|i| format!("{{\"line\":{i},\"pad\":\"{pad}\"}}\n"))
        .collect();
    std::fs::write(&p, &content).unwrap();
    let start = window_start(content.len(), 2);
    assert!(
        start > 0 && content.as_bytes()[start - 1] != b'\n',
        "fixture must seek into the middle of a line"
    );

    let lines = read_last_n_lines(&p, 2).unwrap();

    assert_eq!(lines.len(), 1, "only one whole line fits the window");
    assert!(lines[0].starts_with("{\"line\":9,"), "{}", &lines[0][..20]);
}

#[test]
fn seek_landing_on_a_line_start_keeps_that_line() {
    let dir = tempfile::TempDir::new().unwrap();
    let p = dir.path().join("aligned.jsonl");
    // Every line is exactly 2048 bytes, so the window starts on a line.
    let line = |i: usize| {
        let head = format!("{{\"line\":{i},\"pad\":\"");
        let pad = "x".repeat(2048 - head.len() - 3);
        format!("{head}{pad}\"}}\n")
    };
    let content: String = (0..10).map(line).collect();
    std::fs::write(&p, &content).unwrap();
    let start = window_start(content.len(), 3);
    assert_eq!(content.as_bytes()[start - 1], b'\n', "fixture is aligned");

    let lines = read_last_n_lines(&p, 3).unwrap();

    assert_eq!(lines.len(), 3);
    assert!(lines[0].starts_with("{\"line\":7,"));
    assert!(lines[2].starts_with("{\"line\":9,"));
}

#[test]
fn seek_inside_a_multibyte_char_still_returns_the_following_lines() {
    let dir = tempfile::TempDir::new().unwrap();
    let p = dir.path().join("korean.jsonl");
    let n = 4;
    let text = "가".repeat(600);
    // The seek is measured from the end, so lengthen the last line one ASCII
    // byte at a time until the seek splits a three-byte character.
    let content = (0..3)
        .map(|shift| {
            let body: String = (0..11)
                .map(|i| format!("{{\"line\":{i},\"text\":\"{text}\"}}\n"))
                .collect();
            let tail = "y".repeat(shift);
            body + &format!("{{\"line\":11,\"text\":\"{text}\",\"z\":\"{tail}\"}}\n")
        })
        .find(|content| !content.is_char_boundary(window_start(content.len(), n)))
        .expect("one shift lands inside a character");
    std::fs::write(&p, &content).unwrap();

    let lines = read_last_n_lines(&p, n).unwrap();

    assert!(!lines.is_empty(), "a torn character must not end the read");
    assert!(lines.iter().all(|line| line.starts_with("{\"line\":")));
    assert!(lines.last().unwrap().starts_with("{\"line\":11,"));
    assert!(lines.last().unwrap().contains("가가가"));
}
