//! Moving a node's evidence aside the moment it stops being live — because
//! it failed, or because a cancel interrupted it. Two things depend on this
//! happening before anything else runs: a half-written output must not be
//! mistaken for a good one, and a repair prompt has nothing to read if the
//! evidence is still live.

use crate::NodeId;
use std::path::{Path, PathBuf};

/// A failed move, with the path it was on. The set can hold several nodes,
/// so "archiving failed" without a path leaves the caller unable to say
/// which member of it died.
#[derive(Debug)]
pub struct ArchiveError {
    pub path: PathBuf,
    pub source: std::io::Error,
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
    std::fs::create_dir_all(log_dir).map_err(|source| ArchiveError {
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
    std::fs::create_dir_all(log_dir).map_err(|source| ArchiveError {
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
    if !output.is_file() {
        return Ok(None);
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
        return Err(ArchiveError {
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
    std::fs::rename(output, &destination).map_err(|source| ArchiveError {
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
            &[("review".to_string(), Some(output.clone()))],
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
                ("review".to_string(), Some(review.clone())),
                ("summary".to_string(), Some(summary.clone())),
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
            &[("gate".to_string(), None)],
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

        let archived = archive_attempt(&log_dir, &[("notes".to_string(), Some(output))], 3, 9, &[])
            .expect("ok");
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
            archive_canceled(&log_dir, &"design".to_string(), &output).expect("archiving succeeds");

        assert!(!output.exists(), "the live output must be gone");
        assert_eq!(archived, Some(log_dir.join("design.canceled.md")));
    }

    /// A cancel that lands before the node wrote anything has nothing to
    /// move, and that is not a failure to report.
    #[test]
    fn a_cancel_with_nothing_written_archives_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let log_dir = dir.path().join("logs");
        let archived = archive_canceled(
            &log_dir,
            &"design".to_string(),
            &dir.path().join("design.md"),
        )
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

            let error = archive_attempt(
                &log_dir,
                &[(id.to_string(), Some(output.clone()))],
                1,
                1,
                &[],
            )
            .expect_err("must refuse");

            assert_eq!(
                error.source.kind(),
                std::io::ErrorKind::InvalidInput,
                "`{id}`"
            );
            assert_eq!(error.path, output, "the refusal names the file it was on");
            assert!(output.is_file(), "`{id}` must leave the output untouched");
        }
    }
}
