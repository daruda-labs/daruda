//! What an agent node's turn said, on disk.
//!
//! A command node has left a log since P2c; an agent node left nothing, so
//! a repair's `{{attempts}}` pointed at the failed node's *output* and
//! nothing else — what the agent was doing when it went wrong was gone. A
//! cancelled or killed turn left no trace at all.
//!
//! Written as the turn streams, not assembled at the end: the turns worth
//! reading afterwards are exactly the ones that did not finish, and a
//! buffer would lose those. Every write is best-effort — a transcript that
//! could not be opened must not fail a node that is otherwise fine, so a
//! failed write closes the sink rather than propagating.

use daruda_acp::{ContentBlock, SessionUpdate};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Cap on one recorded content chunk. A tool that echoes a large file
/// would otherwise put it in the run directory twice — once as the node's
/// output and once here.
const MAX_CHUNK: usize = 4_000;

/// Cap on the whole transcript. A per-chunk cap bounds nothing on its own:
/// a turn streams as many chunks as it likes, and an agent looping until
/// its node timeout is exactly what design §6's ceilings exist for. Past
/// this the file says it stopped rather than trailing off.
const MAX_TRANSCRIPT: u64 = 512 * 1024;

/// Where the transcript is going, if anywhere.
///
/// Three states and no fourth: a handle plus a "was it opened" flag would
/// admit `(open, never opened)`, which cannot happen and would have to be
/// reasoned about at every use.
enum Sink {
    /// Never opened. There is no file, so there is nothing to report.
    Unavailable,
    Open(std::fs::File),
    /// Opened, then a write failed or the cap was reached. Whatever landed
    /// is still worth reading, so the path is still reported.
    Closed,
}

pub(super) struct Transcript {
    sink: Sink,
    path: PathBuf,
    written: u64,
}

impl Transcript {
    /// Named the way `ProcessRunner` names its log, so the archive treats
    /// both the same and a reader finds them side by side.
    pub(super) fn create(log_dir: &Path, node_id: &str, attempt: u32, evidence_seq: u32) -> Self {
        let path = log_dir.join(format!(
            "{node_id}.attempt-{attempt}.evidence-{evidence_seq}.md"
        ));
        let sink = std::fs::create_dir_all(log_dir)
            .and_then(|()| std::fs::File::create(&path))
            .map_or(Sink::Unavailable, Sink::Open);
        Self {
            sink,
            path,
            written: 0,
        }
    }

    /// The prompt the node was given. First, because a transcript that
    /// does not say what was asked cannot be read on its own.
    pub(super) fn prompt(&mut self, text: &str) {
        self.write(&format!("# Prompt\n\n{text}\n\n# Turn\n\n"));
    }

    pub(super) fn update(&mut self, update: &SessionUpdate) {
        let line = match update {
            SessionUpdate::AgentMessageChunk(chunk) => text_of(&chunk.content),
            // Kept: a turn that thought its way into the wrong answer is
            // exactly what a repair needs to see.
            SessionUpdate::AgentThoughtChunk(chunk) => {
                let text = text_of(&chunk.content);
                if text.is_empty() {
                    String::new()
                } else {
                    format!("_{text}_")
                }
            }
            // The title only — a tool call's content is often the file the
            // node is already writing, and this is not the place for a
            // second copy of it.
            SessionUpdate::ToolCall(call) => format!("\n`{}`\n", call.title),
            _ => String::new(),
        };
        if !line.is_empty() {
            self.write(&truncated(line));
        }
    }

    /// How the turn ended, in the runner's own words — the stop reason, a
    /// failure, or the fact that it was cancelled.
    pub(super) fn ended(&mut self, how: &str) {
        self.write(&format!("\n\n# Ended\n\n{how}\n"));
    }

    /// The path, when anything was written there. An unopened transcript
    /// contributes nothing rather than a path that is not on disk — the
    /// scheduler archives what it is given.
    pub(super) fn artifacts(self) -> Vec<PathBuf> {
        match self.sink {
            Sink::Unavailable => Vec::new(),
            Sink::Open(_) | Sink::Closed => vec![self.path],
        }
    }

    fn write(&mut self, text: &str) {
        let Sink::Open(file) = &mut self.sink else {
            return;
        };
        // Flushed per write: the turns worth reading are the ones that were
        // killed, and a buffer dies with the process.
        let wrote = file
            .write_all(text.as_bytes())
            .and_then(|()| file.flush())
            .is_ok();
        self.written += text.len() as u64;
        if !wrote {
            self.sink = Sink::Closed;
            return;
        }
        if self.written >= MAX_TRANSCRIPT {
            // Said in the file, not just implied by where it stops: a
            // transcript that trails off reads like a crash.
            let _ = file.write_all(b"\n\n_(transcript truncated)_\n");
            self.sink = Sink::Closed;
        }
    }
}

fn truncated(mut text: String) -> String {
    if text.len() > MAX_CHUNK {
        text.truncate(
            (0..=MAX_CHUNK)
                .rev()
                .find(|i| text.is_char_boundary(*i))
                .unwrap_or(0),
        );
        text.push('…');
    }
    text
}

/// Only the text blocks. An image or a resource link is a pointer to
/// something this file is not the place to copy.
fn text_of(block: &ContentBlock) -> String {
    match block {
        ContentBlock::Text(t) => t.text.clone(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transcript(dir: &Path) -> Transcript {
        Transcript::create(dir, "design", 1, 7)
    }

    /// The name `ProcessRunner` uses, so the archive handles both alike and
    /// a reader finds a node's log and transcript next to each other.
    #[test]
    fn a_transcript_is_named_like_the_command_log_beside_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let t = transcript(dir.path());
        let name = t.path.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(name, "design.attempt-1.evidence-7.md");
        assert_eq!(t.artifacts().len(), 1);
    }

    /// The reason this is written as it streams: the turns worth reading
    /// afterwards are the ones that never reached an ending.
    #[test]
    fn what_was_written_before_a_turn_died_is_on_disk() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut t = transcript(dir.path());
        t.prompt("write the file");
        t.write("half a thou");
        // No `ended` — the process is gone.
        let text = std::fs::read_to_string(&t.path).expect("readable");
        assert!(text.contains("write the file"), "{text}");
        assert!(text.contains("half a thou"), "{text}");
    }

    /// A tool that echoes a large file would otherwise land in the run
    /// directory twice.
    #[test]
    fn one_oversized_chunk_is_cut_at_a_character_boundary() {
        let long = "가".repeat(MAX_CHUNK);
        let cut = truncated(long);
        assert!(cut.len() <= MAX_CHUNK + "…".len());
        assert!(cut.ends_with('…'));
    }

    /// A transcript that could not be opened must not fail a node that is
    /// otherwise fine, and must not name a file nothing wrote.
    #[test]
    fn an_unwritable_transcript_reports_nothing_and_swallows_its_writes() {
        let mut t = Transcript::create(Path::new("/proc/nonexistent-dir"), "design", 1, 7);
        t.prompt("still fine");
        assert!(t.artifacts().is_empty());
    }

    /// The per-chunk cap bounds one write, not the file: a turn streams as
    /// many chunks as it likes, and an agent looping until its node timeout
    /// is what the run's ceilings exist for. The file has to stop on its
    /// own, and say that it did.
    #[test]
    fn a_turn_that_never_stops_talking_does_not_fill_the_disk() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut t = transcript(dir.path());
        for _ in 0..1_000 {
            t.write(&"x".repeat(MAX_CHUNK));
        }
        let text = std::fs::read_to_string(&t.path).expect("readable");
        assert!(
            (text.len() as u64) < MAX_TRANSCRIPT + MAX_CHUNK as u64 + 64,
            "{} bytes",
            text.len()
        );
        assert!(text.ends_with("_(transcript truncated)_\n"), "no notice");
        // Still an artifact: what landed before the cap is what a repair reads.
        assert_eq!(t.artifacts().len(), 1);
    }
}
