//! Reverse-read the last N lines of a (potentially large) JSONL file.
//!
//! Ported from c9watch (`src-tauri/src/session/parser.rs:255-303`,
//! MIT — see `LICENSE-THIRD-PARTY.md`).
//!
//! Seek backward until enough complete lines are available; tool results can
//! occupy megabytes on one line, so byte estimates cannot bound the lookback.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const READ_CHUNK_BYTES: u64 = 64 * 1024;

/// Read the last `n` non-blank lines. I/O and invalid UTF-8 fail the whole read.
pub fn read_last_n_lines<P: AsRef<Path>>(path: P, n: usize) -> Result<Vec<String>, std::io::Error> {
    if n == 0 {
        return Ok(Vec::new());
    }
    let mut file = File::open(path.as_ref())?;
    let size = file.metadata()?.len();
    let mut lookback = READ_CHUNK_BYTES.min(size);
    loop {
        let start = size - lookback;
        file.seek(SeekFrom::Start(start))?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        // A seek can split UTF-8 or JSON; discard only that incomplete line.
        let complete = if start == 0 {
            bytes.as_slice()
        } else if let Some(newline) = bytes.iter().position(|byte| *byte == b'\n') {
            &bytes[newline + 1..]
        } else {
            &[]
        };
        let text = std::str::from_utf8(complete)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        let lines: Vec<_> = text
            .lines()
            .filter(|line| !line.trim().is_empty())
            .collect();
        if start == 0 || lines.len() >= n {
            return Ok(lines[lines.len().saturating_sub(n)..]
                .iter()
                .map(|line| (*line).to_owned())
                .collect());
        }
        lookback = lookback.saturating_mul(2).min(size);
    }
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
        assert!(size >= 10_000, "test fixture should be large");

        let lines = read_last_n_lines(&p, 5).unwrap();
        assert_eq!(lines.len(), 5);
        // Must end on the very last line.
        assert!(lines[4].contains("\"line\":199"));
    }
}
