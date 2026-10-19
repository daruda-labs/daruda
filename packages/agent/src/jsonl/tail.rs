//! Reverse-read the last N lines of a (potentially large) JSONL file.
//!
//! Ported from c9watch (`src-tauri/src/session/parser.rs:255-303`,
//! MIT — see `LICENSE-THIRD-PARTY.md`).
//!
//! Strategy:
//! - Empty file → `Ok(vec![])`.
//! - Files under 10 KB → read whole.
//! - Larger files → read only the last `chunk_size` bytes, where
//!   `chunk_size` defaults to `n * 2 KB`; the caller asks for `n=20`
//!   and the average JSONL line is well under 1 KB, so 40 KB lookback
//!   is plenty. The window's first line is torn unless the byte before
//!   it is a newline, so it is dropped up to the first `\n`.
//!
//! Each line is decoded on its own and a line that is not UTF-8 is
//! skipped, so a torn or corrupt line costs only itself.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const SMALL_FILE_BYTES: u64 = 10_000;
const BYTES_PER_LINE_ESTIMATE: usize = 1024;
const SEEK_BUFFER_FACTOR: usize = 2;

/// Read the last `n` non-blank lines from `path`. Missing or
/// unreadable files return an `io::Error`.
pub fn read_last_n_lines<P: AsRef<Path>>(path: P, n: usize) -> Result<Vec<String>, std::io::Error> {
    let mut file = File::open(path.as_ref())?;
    let file_size = file.metadata()?.len();

    if file_size == 0 {
        return Ok(Vec::new());
    }

    let mut bytes = Vec::new();
    if file_size < SMALL_FILE_BYTES {
        file.read_to_end(&mut bytes)?;
        return Ok(last_lines(&bytes, n));
    }

    let chunk_size = (n
        .saturating_mul(BYTES_PER_LINE_ESTIMATE)
        .saturating_mul(SEEK_BUFFER_FACTOR)) as u64;
    let chunk_size = chunk_size.min(file_size);
    let window_start = file_size - chunk_size;
    if window_start == 0 {
        file.read_to_end(&mut bytes)?;
        return Ok(last_lines(&bytes, n));
    }
    // One byte early, so a window that starts exactly on a line keeps it:
    // that byte is then the newline the skip below consumes.
    file.seek(SeekFrom::Start(window_start - 1))?;
    file.take(chunk_size + 1).read_to_end(&mut bytes)?;
    let whole = match bytes.iter().position(|&b| b == b'\n') {
        Some(newline) => &bytes[newline + 1..],
        None => &[],
    };
    Ok(last_lines(whole, n))
}

/// The last `n` non-blank lines of `bytes`, as `BufRead::lines` splits them
/// (`\n`, with a trailing `\r` removed), skipping lines that are not UTF-8.
fn last_lines(bytes: &[u8], n: usize) -> Vec<String> {
    let lines: Vec<String> = bytes
        .split(|&b| b == b'\n')
        .filter_map(|line| {
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            std::str::from_utf8(line).ok()
        })
        .filter(|line| !line.trim().is_empty())
        .map(str::to_owned)
        .collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

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
}
