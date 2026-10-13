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
