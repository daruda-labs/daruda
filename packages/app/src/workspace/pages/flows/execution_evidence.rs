//! Bounded evidence reads; command output remains literal when shown in Markdown.

use std::{io::Read as _, path::PathBuf};
const MAX_EVIDENCE_BYTES: usize = 512 * 1024;

pub(super) enum ReadEvidence {
    Missing,
    Ready { text: String, truncated: bool },
    Failed(String),
}

pub(super) fn read(path: Option<PathBuf>) -> ReadEvidence {
    let Some(path) = path else {
        return ReadEvidence::Missing;
    };
    let result = (|| {
        let mut bytes = Vec::new();
        std::fs::File::open(&path)?
            .take((MAX_EVIDENCE_BYTES + 1) as u64)
            .read_to_end(&mut bytes)?;
        let truncated = bytes.len() > MAX_EVIDENCE_BYTES;
        bytes.truncate(MAX_EVIDENCE_BYTES);
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let text = if path.extension().is_some_and(|ext| ext == "log") {
            literal_log(&text)
        } else {
            text
        };
        Ok::<_, std::io::Error>((text, truncated))
    })();
    match result {
        Ok((text, truncated)) => ReadEvidence::Ready { text, truncated },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => ReadEvidence::Missing,
        Err(error) => ReadEvidence::Failed(format!("{}: {error}", path.display())),
    }
}

fn literal_log(text: &str) -> String {
    let longest = text
        .split(|ch| ch != '`')
        .map(str::len)
        .max()
        .unwrap_or_default();
    let fence = "`".repeat(longest.saturating_add(1).max(3));
    format!("{fence}text\n{text}\n{fence}\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn logs_cannot_break_out_of_the_literal_block() {
        let text = "```\n# pretend heading\n````";
        let literal = literal_log(text);
        assert!(literal.starts_with("`````text\n"));
        assert!(literal.ends_with("\n`````\n"));
    }
    #[test]
    fn large_evidence_is_explicitly_truncated() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("transcript.md");
        std::fs::write(&path, vec![b'a'; MAX_EVIDENCE_BYTES + 20]).unwrap();
        let ReadEvidence::Ready { text, truncated } = read(Some(path)) else {
            panic!("evidence");
        };
        assert!(truncated);
        assert_eq!(text.len(), MAX_EVIDENCE_BYTES);
    }
}
