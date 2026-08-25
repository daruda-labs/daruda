//! Where a node's `output` is allowed to be, asked twice: before the node
//! runs, so a link already standing on the path cannot redirect the write,
//! and after it, so a link planted during the turn cannot pass for written
//! work. Both ask through `symlink_metadata` — `metadata` follows links,
//! which is the whole thing being defended against.

use crate::parse::SchemaSubset;
use crate::runner::{BreachKind, ContractBreach, NodeFailure, OutputContract};
use std::path::{Path, PathBuf};

/// Refuse an output whose path already goes through a symlink.
///
/// Asked before the parent directory is created and before the runner is
/// called: `create_dir_all` follows a link an earlier node planted, and by
/// the time the output exists the bytes have already landed wherever the
/// link pointed. Detection afterwards is not prevention.
///
/// Not part of the contract, and deliberately not on [`FileContract`]: it
/// asks what is on the path before anything has run, which is a question
/// only the scheduler has a moment for.
pub(crate) fn preflight(run_dir: &Path, output: &Path) -> Result<(), NodeFailure> {
    match links_on(run_dir, output) {
        Ok(()) => Ok(()),
        // Reported as whatever [`FileContract::check`] calls the same on-disk
        // condition after the turn: otherwise the failure a reader sees turns
        // on when the link was planted.
        Err(LinkOnPath::Leaf) => Err(NodeFailure::OutputNotAFile {
            expected: output.to_path_buf(),
        }),
        Err(LinkOnPath::Parent | LinkOnPath::Outside) => Err(NodeFailure::OutputEscapes {
            expected: output.to_path_buf(),
            resolved: resolved_target(output),
        }),
    }
}

/// Where a link sits on an output's path, when one does.
///
/// Named rather than reported straight, because the same walk answers for
/// both sides of the contract and each words the answer for its own reader.
enum LinkOnPath {
    /// The output itself is a link.
    Leaf,
    /// A directory on the way to it is, so the bytes land elsewhere.
    Parent,
    /// The path does not start at the run directory at all.
    Outside,
}

/// Walk `run_dir` down to `output`, one component at a time, and refuse the
/// first link.
///
/// The single rule both halves of the contract ask. Asking it in only one of
/// them is how a directory swapped for a link *during* the turn passed:
/// `preflight` had already walked a real directory, and a comparison made on
/// the resolved path cannot tell "the file is under the run directory" from
/// "the file the link points at is" — which is satisfied by another node's
/// output sitting right next to it.
///
/// `symlink_metadata`, never `metadata` — the latter follows links, which is
/// the whole thing being defended against. A component that is not there is
/// not a refusal: nothing below it can be there either.
fn links_on(run_dir: &Path, output: &Path) -> Result<(), LinkOnPath> {
    // Every output path is `run_dir` joined with a declared relative one, so
    // one that does not start there did not come from a flow.
    let relative = output
        .strip_prefix(run_dir)
        .map_err(|_| LinkOnPath::Outside)?;
    let mut walked = run_dir.to_path_buf();
    let mut parts = relative.components().peekable();
    while let Some(part) = parts.next() {
        walked.push(part);
        let Ok(meta) = std::fs::symlink_metadata(&walked) else {
            return Ok(());
        };
        if meta.file_type().is_symlink() {
            return Err(if parts.peek().is_none() {
                LinkOnPath::Leaf
            } else {
                LinkOnPath::Parent
            });
        }
    }
    Ok(())
}

/// The file a node owes, and the run directory it has to stay inside.
///
/// Owns its paths rather than borrowing them: a holder has to keep one
/// alive beside the `RunContext` that borrows it, which a borrowing struct
/// makes awkward wherever the paths are built in the same expression.
pub(crate) struct FileContract {
    run_dir: PathBuf,
    output: PathBuf,
    /// Owned for the same reason as the paths, and cloned per attempt: a
    /// schema is a few dozen bytes against an agent turn.
    schema: Option<SchemaSubset>,
}

impl FileContract {
    pub(crate) fn new(run_dir: &Path, output: &Path, schema: Option<&SchemaSubset>) -> Self {
        Self {
            run_dir: run_dir.to_path_buf(),
            output: output.to_path_buf(),
            schema: schema.cloned(),
        }
    }

    /// The declared shape, asked of the file's contents parsed as JSON.
    ///
    /// An unreadable file is reported the same way: the questions above have
    /// already established that a plain, non-empty file of this node's is
    /// there, so what is left is about its contents — and re-asking the agent
    /// to write it is a plausible fix either way.
    fn shape_holds(&self, schema: &SchemaSubset) -> Result<(), ContractBreach> {
        let problems = match std::fs::read_to_string(&self.output) {
            Err(e) => vec![format!("the file could not be read: {e}")],
            Ok(text) => match serde_json::from_str::<serde_json::Value>(&text) {
                Err(e) => vec![format!("the file is not a single JSON value: {e}")],
                Ok(value) => match crate::contract::schema::validate(&value, schema) {
                    Ok(()) => return Ok(()),
                    Err(problems) => problems,
                },
            },
        };
        let mut lines = problems.into_iter();
        Err(ContractBreach {
            kind: BreachKind::Schema {
                expected: self.output.clone(),
            },
            // `validate` returns an `Err` only with at least one line, and the
            // two arms above build their own.
            first: lines
                .next()
                .unwrap_or_else(|| "the shape is wrong".to_string()),
            rest: lines.collect(),
        })
    }
}

impl OutputContract for FileContract {
    /// A plain, non-empty file at a path that resolves inside the run
    /// directory.
    fn check(&self) -> Result<(), ContractBreach> {
        let expected = || self.output.clone();
        match std::fs::symlink_metadata(&self.output) {
            Err(_) => {
                return Err(breach(BreachKind::Missing {
                    expected: expected(),
                }));
            }
            // A link reports its target's size, so "wrote something" would be
            // satisfied by bytes the node never wrote.
            Ok(meta) if !meta.file_type().is_file() => {
                return Err(breach(BreachKind::NotAFile {
                    expected: expected(),
                }));
            }
            Ok(meta) if meta.len() == 0 => {
                return Err(breach(BreachKind::Missing {
                    expected: expected(),
                }));
            }
            Ok(_) => {}
        }
        // A plain file can still sit under a directory that became a link
        // while the node ran. The same walk `preflight` does, because only a
        // walk can tell that apart: a link resolving back *inside* the run
        // directory — at a sibling node's output — satisfies every check made
        // on the resolved path.
        match links_on(&self.run_dir, &self.output) {
            Ok(()) => {}
            Err(LinkOnPath::Leaf) => {
                return Err(breach(BreachKind::NotAFile {
                    expected: expected(),
                }));
            }
            Err(LinkOnPath::Parent | LinkOnPath::Outside) => {
                return Err(breach(BreachKind::Escapes {
                    expected: expected(),
                    resolved: resolved_target(&self.output),
                }));
            }
        }
        // Last, and only when the node declared one: a shape question about a
        // file that is absent, or is not this node's work, would report the
        // wrong problem.
        match &self.schema {
            Some(schema) => self.shape_holds(schema),
            None => Ok(()),
        }
    }
}

/// One breach whose whole story is its kind, worded for whoever is being asked
/// to put it right rather than for the run's log. A schema breach is not one of
/// these: its lines come from the check, so [`FileContract::shape_holds`]
/// builds it.
fn breach(kind: BreachKind) -> ContractBreach {
    let first = match &kind {
        BreachKind::Missing { expected } => format!(
            "nothing usable is at {}: the file is absent or empty",
            expected.display()
        ),
        BreachKind::NotAFile { expected } => format!(
            "{} is not a plain file, so nothing there counts as work written",
            expected.display()
        ),
        BreachKind::Escapes { expected, resolved } => format!(
            "{} resolves through a link to {}, outside the run directory",
            expected.display(),
            resolved.display()
        ),
        // Every line of a schema breach is the check's, so it cannot be worded
        // from the kind alone.
        BreachKind::Schema { expected } => format!(
            "the contents of {} are not the declared shape",
            expected.display()
        ),
    };
    ContractBreach {
        kind,
        first,
        rest: Vec::new(),
    }
}

/// The path a write would really land on: the longest existing prefix
/// resolved, with the rest joined back on. What a refusal reports, so the
/// message names the target rather than the link.
fn resolved_target(path: &Path) -> PathBuf {
    let mut remainder: Vec<std::ffi::OsString> = Vec::new();
    let mut probe = path;
    loop {
        if let Ok(real) = std::fs::canonicalize(probe) {
            return remainder
                .iter()
                .rev()
                .fold(real, |resolved, part| resolved.join(part));
        }
        match (probe.parent(), probe.file_name()) {
            (Some(parent), Some(name)) => {
                remainder.push(name.to_os_string());
                probe = parent;
            }
            _ => return path.to_path_buf(),
        }
    }
}

#[cfg(test)]
mod tests;
