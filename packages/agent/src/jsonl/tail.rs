//! Reverse-read the last N lines of a (potentially large) JSONL file.
//!
//! Ported from c9watch (`src-tauri/src/session/parser.rs:255-303`,
//! MIT — see `LICENSE-THIRD-PARTY.md`).
//!
//! Strategy:
//! - Empty file → `Ok(vec![])`.
//! - Files under 10 KB → read normally (the linewise iterator is
//!   already efficient at that size).
//! - Larger files → seek to `file_size - chunk_size` and read forward.
//!   `chunk_size` defaults to `n * 2 KB`; the caller asks for `n=20`
//!   and the average JSONL line is well under 1 KB, so 40 KB lookback
//!   is plenty.

use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::Path;

const SMALL_FILE_BYTES: u64 = 10_000;
const BYTES_PER_LINE_ESTIMATE: usize = 1024;
const SEEK_BUFFER_FACTOR: usize = 2;

/// Read the last `n` non-blank lines from `path`. Missing or
/// unreadable files return an `io::Error`.
pub fn read_last_n_lines<P: AsRef<Path>>(path: P, n: usize) -> Result<Vec<String>, std::io::Error> {
    let file = File::open(path.as_ref())?;
    let metadata = file.metadata()?;
    let file_size = metadata.len();

    if file_size == 0 {
        return Ok(Vec::new());
    }

    if file_size < SMALL_FILE_BYTES {
        let reader = BufReader::new(file);
        let lines: Vec<String> = reader
            .lines()
            .map_while(Result::ok)
            .filter(|line| !line.trim().is_empty())
            .collect();
        let start = lines.len().saturating_sub(n);
        return Ok(lines[start..].to_vec());
    }

    let chunk_size = (n
        .saturating_mul(BYTES_PER_LINE_ESTIMATE)
        .saturating_mul(SEEK_BUFFER_FACTOR)) as u64;
    let chunk_size = chunk_size.min(file_size);
    let mut file = file;
    file.seek(SeekFrom::End(-(chunk_size as i64)))?;

    let reader = BufReader::new(file);
    let lines: Vec<String> = reader
        .lines()
        .map_while(Result::ok)
        .filter(|line| !line.trim().is_empty())
        .collect();
    let start = lines.len().saturating_sub(n);
    Ok(lines[start..].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn large_file_reverse_seek_returns_last_n() {
        let dir = tempfile::TempDir::new().unwrap();
        let p = dir.path().join("big.jsonl");
        let mut f = std::fs::File::create(&p).unwrap();
        // Force the file over the 10 KB threshold.
        for i in 0..200 {
            // Pad each line so the total is comfortably above 10 KB.
            let pad = "x".repeat(80);
            writeln!(f, r#"{{"line":{i},"pad":"{pad}"}}"#).unwrap();
        }
        drop(f);

        let size = std::fs::metadata(&p).unwrap().len();
        assert!(size >= SMALL_FILE_BYTES, "test fixture should be large");

        let lines = read_last_n_lines(&p, 5).unwrap();
        assert_eq!(lines.len(), 5);
        // Must end on the very last line.
        assert!(lines[4].contains("\"line\":199"));
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
}
