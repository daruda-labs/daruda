//! The last main-thread assistant answer in a Claude Code transcript.
//!
//! Scans backward from the end under a caller-supplied byte budget and stops
//! at the newest qualifying line, so a long transcript costs only its tail.
//! A line the reader cannot parse (torn by a concurrent append, invalid UTF-8,
//! an unfamiliar shape) is skipped rather than discarding an earlier answer.

use std::borrow::Cow;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use serde::Deserialize;

const READ_CHUNK_BYTES: u64 = 64 * 1024;

/// Only the fields the answer needs. Every other field, including multi-MB
/// tool results and images, is skipped by serde's `IgnoredAny` without being
/// copied.
#[derive(Deserialize)]
struct Line {
    #[serde(rename = "type")]
    kind: String,
    #[serde(rename = "isSidechain", default)]
    is_sidechain: Option<bool>,
    #[serde(default)]
    message: Option<Message>,
}

#[derive(Deserialize)]
struct Message {
    #[serde(default)]
    content: Vec<Block>,
}

#[derive(Deserialize)]
struct Block {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: Option<String>,
}

/// The text blocks of the newest non-sidechain assistant message with
/// non-blank text, joined by newlines. Only the last `max_bytes` of the file
/// are read; a line starting before that window is never considered.
/// `Ok(None)` when no qualifying line is in the window; I/O failures are `Err`.
pub fn last_assistant_text(path: &Path, max_bytes: u64) -> std::io::Result<Option<String>> {
    scan(path, max_bytes, READ_CHUNK_BYTES)
}

fn scan(path: &Path, max_bytes: u64, chunk_bytes: u64) -> std::io::Result<Option<String>> {
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();
    let floor = size.saturating_sub(max_bytes);
    // One byte before the window shows whether its first line starts exactly
    // at the window edge.
    let read_floor = floor.saturating_sub(1);
    let mut pos = size;
    // Pieces of the line that continues past the current chunk, newest first.
    let mut carried: Vec<Vec<u8>> = Vec::new();
    while pos > read_floor {
        let start = pos.saturating_sub(chunk_bytes).max(read_floor);
        let mut chunk = vec![0; (pos - start) as usize];
        file.seek(SeekFrom::Start(start))?;
        file.read_exact(&mut chunk)?;
        pos = start;
        let mut end = chunk.len();
        while let Some(newline) = chunk[..end].iter().rposition(|byte| *byte == b'\n') {
            if let Some(text) = answer(&assemble(&chunk[newline + 1..end], &mut carried)) {
                return Ok(Some(text));
            }
            end = newline;
        }
        carried.push(chunk[..end].to_vec());
    }
    // At the file start the remainder is the first line; otherwise it began
    // before the budget window and is incomplete.
    Ok((floor == 0)
        .then(|| answer(&assemble(&[], &mut carried)))
        .flatten())
}

/// One line in file order: `head` followed by the carried pieces.
fn assemble<'a>(head: &'a [u8], carried: &mut Vec<Vec<u8>>) -> Cow<'a, [u8]> {
    if carried.is_empty() {
        return Cow::Borrowed(head);
    }
    let mut line = head.to_vec();
    for piece in carried.drain(..).rev() {
        line.extend_from_slice(&piece);
    }
    Cow::Owned(line)
}

fn answer(line: &[u8]) -> Option<String> {
    if line.iter().all(u8::is_ascii_whitespace) {
        return None;
    }
    let line: Line = serde_json::from_slice(line).ok()?;
    if line.kind != "assistant" || line.is_sidechain == Some(true) {
        return None;
    }
    let text = line
        .message?
        .content
        .into_iter()
        .filter(|block| block.kind == "text")
        .filter_map(|block| block.text)
        .collect::<Vec<_>>()
        .join("\n");
    (!text.trim().is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assistant(text: &str) -> String {
        serde_json::json!({
            "type": "assistant",
            "message": { "content": [{"type": "text", "text": text}] }
        })
        .to_string()
    }

    /// Lines spanning many small chunks are reassembled in file order.
    #[test]
    fn lines_longer_than_a_chunk_are_reassembled() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("chunks.jsonl");
        let long = assistant(&"0123456789".repeat(20));
        let later = r#"{"type":"user","message":{"content":[{"type":"text","text":"q"}]}}"#;
        std::fs::write(&path, format!("{long}\n{later}\n")).unwrap();
        for chunk in [1, 3, 7, 64, 4096] {
            assert_eq!(
                scan(&path, u64::MAX, chunk).unwrap().as_deref(),
                Some("0123456789".repeat(20).as_str()),
                "chunk {chunk}"
            );
        }
    }

    /// The budget is exact: one byte short of the answer line excludes it.
    #[test]
    fn the_budget_boundary_is_exact() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("budget.jsonl");
        let line = assistant("edge");
        std::fs::write(&path, format!("x\n{line}\n")).unwrap();
        let needed = line.len() as u64 + 1;
        for chunk in [1, 5, 4096] {
            assert_eq!(scan(&path, needed, chunk).unwrap().as_deref(), Some("edge"));
            assert_eq!(scan(&path, needed - 1, chunk).unwrap(), None);
        }
    }
}
