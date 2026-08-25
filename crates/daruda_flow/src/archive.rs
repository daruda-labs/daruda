//! Moving a node's evidence aside the moment it stops being live — because
//! it failed, or because a cancel interrupted it. Two things depend on this
//! happening before anything else runs: a half-written output must not be
//! mistaken for a good one, and a repair prompt has nothing to read if the
//! evidence is still live.

use crate::NodeId;
use std::path::{Path, PathBuf};

/// Why one node's evidence could not be put aside. Every variant carries
/// the path: the set can hold several nodes, so "archiving failed" without
/// one leaves the caller unable to say which member of it died.
#[derive(Debug)]
pub enum ArchiveError {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    /// A link is standing where the output belongs. Renaming it would file
    /// the link as evidence, and `{{attempts}}` then tells the next agent
    /// to read a path that resolves to whatever it points at.
    OutputIsLink { path: PathBuf },
}

impl ArchiveError {
    /// The scheduler reports paths, not archive internals, so every failure
    /// reaches it as an I/O one. A refusal has no `errno` and carries its
    /// reason in the message instead.
    pub fn into_io(self) -> (PathBuf, std::io::Error) {
        let message = self.to_string();
        match self {
            ArchiveError::Io { path, source } => (path, source),
            ArchiveError::OutputIsLink { path } => (
                path,
                std::io::Error::new(std::io::ErrorKind::InvalidInput, message),
            ),
        }
    }
}

impl std::fmt::Display for ArchiveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ArchiveError::Io { path, source } => {
                write!(f, "could not move {}: {source}", path.display())
            }
            ArchiveError::OutputIsLink { path } => {
                write!(f, "{} is a link, not evidence", path.display())
            }
        }
    }
}

/// Move each node's live output into `log_dir` under an attempt- and
/// evidence-stamped name and remove the original, then return every path a
/// repair or retry may read — the archived outputs followed by artifacts.
///
/// Called once, on the attempt that failed, for the whole invalidation set.
pub fn archive_attempt(
    log_dir: &Path,
    nodes: &[(NodeId, Option<PathBuf>)],
    attempt: u32,
    evidence_seq: u32,
    artifacts: &[PathBuf],
) -> Result<Vec<PathBuf>, ArchiveError> {
    std::fs::create_dir_all(log_dir).map_err(|source| ArchiveError::Io {
        path: log_dir.to_path_buf(),
        source,
    })?;
    let mut archived = Vec::with_capacity(nodes.len() + artifacts.len());

    for (id, output) in nodes {
        let Some(output) = output else { continue };
        let stamp = format!("attempt-{attempt}.evidence-{evidence_seq}");
        if let Some(destination) = move_aside(log_dir, id, &stamp, output)? {
            archived.push(destination);
        }
    }

    archived.extend(artifacts.iter().cloned());
    Ok(archived)
}

/// Move the output of the node a cancel interrupted. Distinct from
/// `archive_attempt` in its naming only: a cancel has no attempt number
/// worth recording — nothing will retry it.
///
/// `None` when the node wrote nothing: there is then nothing to move, and
/// that is not an error.
pub fn archive_canceled(
    log_dir: &Path,
    node: &NodeId,
    output: &Path,
) -> Result<Option<PathBuf>, ArchiveError> {
    std::fs::create_dir_all(log_dir).map_err(|source| ArchiveError::Io {
        path: log_dir.to_path_buf(),
        source,
    })?;
    move_aside(log_dir, node, "canceled", output)
}

/// Rename `output` into `log_dir` under `<id>.<stamp>[.<ext>]`, or `None`
/// when there is no file there to move. The stamp is the only thing the two
/// callers disagree about.
fn move_aside(
    log_dir: &Path,
    id: &NodeId,
    stamp: &str,
    output: &Path,
) -> Result<Option<PathBuf>, ArchiveError> {
    // Asked without following links: a link here is neither a file to move
    // nor an absence to skip over quietly, and `is_file` cannot tell the
    // three apart.
    match std::fs::symlink_metadata(output) {
        // Nothing was written, which is not an error.
        Err(_) => return Ok(None),
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(ArchiveError::OutputIsLink {
                path: output.to_path_buf(),
            });
        }
        Ok(meta) if !meta.is_file() => return Ok(None),
        Ok(_) => {}
    }
    let name = match output.extension().and_then(|e| e.to_str()) {
        Some(ext) => format!("{id}.{stamp}.{ext}"),
        None => format!("{id}.{stamp}"),
    };
    // The name is built from a node id, so this function's reach depends
    // on a rule enforced elsewhere. `validate` rejects an id that is not
    // a plain filename; refusing here too keeps a caller that skipped it
    // from renaming an output to wherever the id points.
    if Path::new(&name).components().count() != 1 {
        return Err(ArchiveError::Io {
            path: output.to_path_buf(),
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("`{id}` is not a usable archive name"),
            ),
        });
    }
    let destination = log_dir.join(name);
    // `rename` keeps the bytes and clears the live path in one step, so
    // there is no window where both exist.
    std::fs::rename(output, &destination).map_err(|source| ArchiveError::Io {
        path: output.to_path_buf(),
        source,
    })?;
    Ok(Some(destination))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &std::path::Path, text: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("mkdir");
        }
        std::fs::write(path, text).expect("write");
    }

    /// The defect this exists to prevent: attempt 1 writes half a file and
    /// fails; without archiving, attempt 2 can end without writing and the
    /// stale half-file passes the "did it write anything" judgment.
    #[test]
    fn a_failed_attempts_output_is_moved_aside_and_the_original_is_gone() {
        let dir = tempfile::tempdir().expect("tempdir");
        let run_dir = dir.path();
        let log_dir = run_dir.join("logs");
        let output = run_dir.join("review.md");
        write(&output, "VERDICT: FAIL\nhalf a thought");

        let archived = archive_attempt(
            &log_dir,
            &[("review".into(), Some(output.clone()))],
            1,
            1,
            &[],
        )
        .expect("archiving succeeds");

        assert!(!output.exists(), "the live output must be gone");
        let moved = log_dir.join("review.attempt-1.evidence-1.md");
        assert!(moved.exists());
        assert_eq!(
            std::fs::read_to_string(&moved).expect("read"),
            "VERDICT: FAIL\nhalf a thought"
        );
        assert_eq!(archived, vec![moved]);
    }

    /// The whole invalidation set moves at once — a repair that reads
    /// `{{attempts}}` must see every node it is about to re-derive.
    #[test]
    fn every_node_in_the_set_is_archived_in_one_call() {
        let dir = tempfile::tempdir().expect("tempdir");
        let run_dir = dir.path();
        let log_dir = run_dir.join("logs");
        let review = run_dir.join("review.md");
        let summary = run_dir.join("summary.md");
        write(&review, "r");
        write(&summary, "s");

        let archived = archive_attempt(
            &log_dir,
            &[
                ("review".into(), Some(review.clone())),
                ("summary".into(), Some(summary.clone())),
            ],
            2,
            7,
            &[],
        )
        .expect("archiving succeeds");

        assert!(!review.exists() && !summary.exists());
        assert_eq!(
            archived,
            vec![
                log_dir.join("review.attempt-2.evidence-7.md"),
                log_dir.join("summary.attempt-2.evidence-7.md")
            ]
        );
    }

    /// A command gate has no output, and a node may have failed before
    /// writing. Neither is an error — the runner's own artifacts are still
    /// the evidence a repair reads.
    #[test]
    fn nodes_without_an_output_contribute_only_the_runners_artifacts() {
        let dir = tempfile::tempdir().expect("tempdir");
        let log_dir = dir.path().join("logs");
        let gate_log = log_dir.join("gate.attempt-1.evidence-1.log");
        write(&gate_log, "exit 1");

        let archived = archive_attempt(
            &log_dir,
            &[("gate".into(), None)],
            1,
            1,
            std::slice::from_ref(&gate_log),
        )
        .expect("archiving succeeds");

        assert_eq!(archived, vec![gate_log]);
    }

    /// An output with no extension still gets a distinct archive name.
    #[test]
    fn an_extensionless_output_keeps_a_unique_archive_name() {
        let dir = tempfile::tempdir().expect("tempdir");
        let run_dir = dir.path();
        let log_dir = run_dir.join("logs");
        let output = run_dir.join("NOTES");
        write(&output, "x");

        let archived =
            archive_attempt(&log_dir, &[("notes".into(), Some(output))], 3, 9, &[]).expect("ok");
        assert_eq!(archived, vec![log_dir.join("notes.attempt-3.evidence-9")]);
    }

    /// A cancel gets no attempt number: nothing will retry the node, so the
    /// name only has to say why the file stopped being live.
    #[test]
    fn a_canceled_nodes_output_moves_aside_under_a_name_with_no_attempt() {
        let dir = tempfile::tempdir().expect("tempdir");
        let log_dir = dir.path().join("logs");
        let output = dir.path().join("design.md");
        write(&output, "half a thought");

        let archived =
            archive_canceled(&log_dir, &"design".into(), &output).expect("archiving succeeds");

        assert!(!output.exists(), "the live output must be gone");
        assert_eq!(archived, Some(log_dir.join("design.canceled.md")));
    }

    /// A cancel that lands before the node wrote anything has nothing to
    /// move, and that is not a failure to report.
    #[test]
    fn a_cancel_with_nothing_written_archives_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let log_dir = dir.path().join("logs");
        let archived = archive_canceled(&log_dir, &"design".into(), &dir.path().join("design.md"))
            .expect("archiving succeeds");
        assert_eq!(archived, None);
    }

    /// `validate` rejects these ids, so reaching here means a caller built a
    /// `Flow` by hand. Refusing beats renaming a real output to `../../`.
    #[test]
    fn an_id_that_would_leave_the_log_directory_is_refused() {
        for id in ["../../pwned", "/tmp/abs", "a/b"] {
            let dir = tempfile::tempdir().expect("tempdir");
            let log_dir = dir.path().join("run/logs");
            let output = dir.path().join("run/review.md");
            write(&output, "secret");

            let error = archive_attempt(&log_dir, &[(id.into(), Some(output.clone()))], 1, 1, &[])
                .expect_err("must refuse");

            let (path, source) = error.into_io();
            assert_eq!(source.kind(), std::io::ErrorKind::InvalidInput, "`{id}`");
            assert_eq!(path, output, "the refusal names the file it was on");
            assert!(output.is_file(), "`{id}` must leave the output untouched");
        }
    }

    /// A link is not evidence. Archiving it would file the link under
    /// `logs/` and hand that path to a repair agent through `{{attempts}}`,
    /// which is an instruction to read whatever it points at — and skipping
    /// it silently would let the node's refusal go unreported.
    #[test]
    fn a_linked_output_is_refused_rather_than_archived_or_skipped() {
        let dir = tempfile::tempdir().expect("tempdir");
        let run_dir = dir.path();
        let log_dir = run_dir.join("logs");
        let elsewhere = run_dir.join("elsewhere.md");
        write(&elsewhere, "someone else's work");
        let output = run_dir.join("review.md");
        std::os::unix::fs::symlink(&elsewhere, &output).expect("symlink");

        let error = archive_attempt(
            &log_dir,
            &[("review".into(), Some(output.clone()))],
            1,
            1,
            &[],
        )
        .expect_err("a link must be refused, not quietly skipped");

        assert!(
            matches!(&error, ArchiveError::OutputIsLink { path } if path == &output),
            "{error:?}"
        );
        assert!(
            !log_dir.join("review.attempt-1.evidence-1.md").exists(),
            "the link must not be filed as evidence"
        );
        assert!(
            std::fs::read_to_string(&elsewhere).expect("read") == "someone else's work",
            "the target must be left where it is"
        );
    }

    /// A directory standing where the output belongs is not evidence
    /// either, and not a refusal: nothing was written.
    #[test]
    fn a_directory_where_the_output_belongs_archives_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let log_dir = dir.path().join("logs");
        let output = dir.path().join("review.md");
        std::fs::create_dir_all(&output).expect("mkdir");

        let archived = archive_attempt(&log_dir, &[("review".into(), Some(output))], 1, 1, &[])
            .expect("archiving succeeds");
        assert!(archived.is_empty());
    }
}
