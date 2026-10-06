//! What a run has finished, written down as it happens, so a run that was
//! killed can be picked up instead of started over.
//!
//! Everything else a run leaves behind is already enough to *read* it: the
//! resolved spec is in `run.yaml`, the outputs and the evidence logs are in
//! the run directory. What is missing after a crash is which nodes passed —
//! that lives in memory and dies with the process, and outputs cannot stand
//! in for it because a command node writes none.
//!
//! **Append-only, one JSON object per line.** The writer is a process that
//! may be `kill -9`'d mid-write, so the format has to survive a torn tail:
//! a line that does not parse is dropped and everything before it stands.
//! A whole-file format could not offer that.
//!
//! **The schema is versioned and read leniently** — an entry whose `kind`
//! this build does not know, or whose `v` is from a newer one, is skipped
//! rather than failing the read. That is what lets the shape change later
//! without a resume refusing runs it half-understands.

use serde::{Deserialize, Serialize};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::NodeId;
use crate::record::{AttemptOutcome, AttemptRecord, Invalidation, NodeRecord};
use crate::request::CostLimit;
use crate::runner::{AskAnswer, Waiting};

/// Named beside the reader and the writer both, like [`RUN_REPORT_FILE`].
///
/// [`RUN_REPORT_FILE`]: crate::record::RUN_REPORT_FILE
pub const JOURNAL_FILE: &str = "progress.jsonl";

/// What this build writes. Read is lenient about anything higher — see the
/// module docs.
const JOURNAL_VERSION: u32 = 1;

/// One line of the journal.
///
/// Deliberately its own shape rather than `serde` on the scheduler's types:
/// this is a file other builds read, and pinning it to the in-memory model
/// would make every internal rename a format change.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Entry {
    /// Written once, past the lock and before the first node. Its presence
    /// is what says the run got as far as running something; its absence
    /// says the crash was in setup.
    Started {
        v: u32,
        /// The node the run was asked to stop at. Carried here for the same
        /// reason `profile` is: `run.yaml` records the whole flow, so a
        /// resume has nowhere else to read the selection back from.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        until: Option<NodeId>,
        /// Nodes whose output this run reused instead of computing. Recorded
        /// so a resume treats them as passed — otherwise the unclaimed-output
        /// sweep would archive the copies out from under it.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pinned: Vec<NodeId>,
        /// The profile the run was submitted under. Carried here because
        /// `run.yaml` deliberately records the settings rather than the
        /// name that produced them, so a resume has nowhere else to read it.
        profile: Option<String>,
    },
    /// Written where a later process picked the run up. Nothing reads it
    /// back — its whole job is to be in the file, so that anyone looking at
    /// a run directory afterwards can see it was continued and where.
    Resumed { v: u32, carried: usize },
    /// One attempt at one node, plus what the run had spent once it
    /// settled. The spend is a *snapshot*, not a delta: a torn tail then
    /// costs the reader the last attempt, never a wrong total.
    ///
    /// Boxed only for its size beside `Started`; an internally tagged
    /// newtype variant lays its fields out beside the tag exactly as an
    /// inline struct variant would, so the file is unchanged.
    Attempt(Box<AttemptLine>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AttemptLine {
    v: u32,
    node: NodeId,
    attempt: u32,
    evidence_seq: u32,
    outcome: OutcomeLine,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    invalidated: Vec<NodeId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    archived: Vec<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    git_status: Option<String>,
    /// Epoch milliseconds. A number rather than a formatted time: the file
    /// is machine-read, and how it reads belongs to whoever renders it.
    #[serde(default, skip_serializing_if = "is_zero")]
    at_ms: u64,
    #[serde(default, skip_serializing_if = "is_zero")]
    took_ms: u64,
    #[serde(default, skip_serializing_if = "is_zero")]
    waited_ms: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    answers: Vec<AnswerLine>,
    /// How many prompts the attempt sent, the first included. Absent on a
    /// journal written before turns were counted; `corrected` below is what
    /// such a line has to say instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    turns: Option<u32>,
    /// **Read, never written.** The flag `turns` replaced: a line carrying it
    /// knew only that the attempt had spent more than one turn, so it reads
    /// back as the floor that claim allows — two. Kept rather than dropped
    /// because dropping it would silently turn every corrected attempt in an
    /// existing journal into an ordinary one.
    #[serde(default, skip_serializing)]
    corrected: bool,
    spent: SpentLine,
}

fn is_zero(ms: &u64) -> bool {
    *ms == 0
}

/// Milliseconds, as a number this file can be read back as.
///
/// **Not `u128`.** `Entry` is an internally tagged enum, and serde buffers
/// the content of one through a type with no 128-bit case — a `u128` field
/// serializes happily and then fails to deserialize with "u128 is not
/// supported". Because both duration fields are skipped when zero, that
/// only bites a run that actually waited: the journal of every other run
/// reads back fine, which is exactly how it would reach a user.
/// Milliseconds since the epoch, or zero for a clock set before it.
fn epoch_millis(at: std::time::SystemTime) -> u64 {
    at.duration_since(std::time::UNIX_EPOCH)
        .map(millis)
        .unwrap_or_default()
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// How an attempt ended. `Failed` keeps only the rendered reason: the
/// failure's own variants are the scheduler's control flow, and a resumed
/// run never re-decides a settled attempt's policy — it only reports it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
enum OutcomeLine {
    Passed,
    Failed { reason: String },
    Canceled,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum AnswerLine {
    Allowed,
    Refused,
    Unanswered,
}

/// What the run had spent when the line was written.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct SpentLine {
    node_runs: u32,
    #[serde(default, skip_serializing_if = "is_zero")]
    parked_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cost: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    currency: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    cost_mixed: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<String>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// What the run had spent, in the scheduler's own terms.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Spent {
    /// Budget units consumed. The journal key keeps its established name for
    /// compatibility with existing run directories.
    pub node_runs: u32,
    pub parked: Duration,
    pub cost: Option<CostLimit>,
    pub cost_mixed: bool,
    pub warnings: Vec<String>,
}

/// Everything a resumed run needs that is not already on disk elsewhere.
#[derive(Debug, Clone, Default)]
pub struct Replay {
    /// The nodes that passed, in the order they did. A node listed here is
    /// not run again.
    pub passed: Vec<NodeId>,
    /// Where the evidence counter got to. Continued rather than restarted,
    /// or the first attempt after a resume overwrites an existing log.
    pub next_seq: u32,
    /// Every attempt the earlier process made, so the finished run's
    /// `run.md` covers both halves rather than starting at the resume.
    pub records: Vec<NodeRecord>,
    pub spent: Spent,
    pub profile: Option<String>,
    /// The selection the earlier process ran under. Re-applied on resume, or
    /// the continuation would run the nodes it was told to skip.
    pub until: Option<NodeId>,
    /// Nodes the earlier process reused rather than ran. Also in `passed`;
    /// kept apart so the record can say which of the two it was.
    pub pinned: Vec<NodeId>,
    /// Whether the journal ended in a torn line — the crash landed
    /// mid-write. Surfaced rather than swallowed: it is the one case where
    /// an attempt really happened and the record cannot show it.
    pub torn: bool,
}

/// Append one attempt. Best-effort by design: a journal that cannot be
/// written costs a resume, and failing the run over it would cost the run.
/// The caller turns the error into a warning.
pub(crate) fn append_attempt(
    run_dir: &Path,
    node: &NodeId,
    attempt: &AttemptRecord,
    spent: &Spent,
) -> std::io::Result<()> {
    append(
        run_dir,
        &Entry::Attempt(Box::new(AttemptLine {
            v: JOURNAL_VERSION,
            node: node.clone(),
            attempt: attempt.attempt,
            evidence_seq: attempt.evidence_seq,
            outcome: match &attempt.outcome {
                AttemptOutcome::Passed => OutcomeLine::Passed,
                AttemptOutcome::Failed(failure) => OutcomeLine::Failed {
                    reason: format!("failed: {failure}"),
                },
                AttemptOutcome::Canceled => OutcomeLine::Canceled,
                // Already read back from a journal once. Written through
                // unchanged so a run resumed twice keeps its whole history.
                AttemptOutcome::Reported(reason) => OutcomeLine::Failed {
                    reason: reason.clone(),
                },
            },
            invalidated: attempt.invalidated.nodes.clone(),
            archived: attempt.invalidated.archived.clone(),
            git_status: attempt.git_status.clone(),
            at_ms: epoch_millis(attempt.at),
            took_ms: millis(attempt.took),
            waited_ms: millis(attempt.waited.total),
            answers: attempt
                .waited
                .answers
                .iter()
                .map(|answer| match answer {
                    AskAnswer::Allowed => AnswerLine::Allowed,
                    AskAnswer::Refused => AnswerLine::Refused,
                    AskAnswer::Unanswered => AnswerLine::Unanswered,
                })
                .collect(),
            turns: (attempt.turns > 1).then_some(attempt.turns),
            corrected: false,
            spent: SpentLine {
                node_runs: spent.node_runs,
                parked_ms: millis(spent.parked),
                cost: spent.cost.as_ref().map(|c| c.amount),
                currency: spent.cost.as_ref().map(|c| c.currency.clone()),
                cost_mixed: spent.cost_mixed,
                warnings: spent.warnings.clone(),
            },
        })),
    )
}

/// Mark where a later process took the run over.
pub(crate) fn resumed(run_dir: &Path, carried: usize) -> std::io::Result<()> {
    append(
        run_dir,
        &Entry::Resumed {
            v: JOURNAL_VERSION,
            carried,
        },
    )
}

/// Open the journal for this run. Written past the lock and before the
/// first node, so a directory with no journal at all is one whose crash
/// came during setup — there is nothing to resume there.
pub(crate) fn start(
    run_dir: &Path,
    profile: Option<&str>,
    until: Option<&NodeId>,
    pinned: &[NodeId],
) -> std::io::Result<()> {
    append(
        run_dir,
        &Entry::Started {
            v: JOURNAL_VERSION,
            until: until.cloned(),
            pinned: pinned.to_vec(),
            profile: profile.map(str::to_string),
        },
    )
}

fn append(run_dir: &Path, entry: &Entry) -> std::io::Result<()> {
    let mut line = serde_json::to_string(entry).map_err(std::io::Error::other)?;
    line.push('\n');
    std::fs::create_dir_all(run_dir)?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(run_dir.join(JOURNAL_FILE))?;
    file.write_all(line.as_bytes())?;
    // The point of the whole file is to survive a process that does not get
    // to run its destructors, so each line is on disk before the next node
    // starts. One flush per settled attempt, not per byte of output.
    file.sync_data()
}

/// Whether this directory holds a journal at all.
pub fn exists(run_dir: &Path) -> bool {
    run_dir.join(JOURNAL_FILE).is_file()
}

/// Read a run's journal back. A missing file is an empty replay, not an
/// error: a crash before the first line is a run with nothing done.
pub fn read(run_dir: &Path) -> Replay {
    let Ok(text) = std::fs::read_to_string(run_dir.join(JOURNAL_FILE)) else {
        return Replay::default();
    };
    let mut replay = Replay::default();
    // A file that does not end in a newline ended mid-write. That last
    // fragment is dropped below by the parse, but only this tells the
    // difference between "torn" and "the run stopped cleanly here".
    let torn_tail = !text.is_empty() && !text.ends_with('\n');

    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let last = index + 1 == text.lines().count();
        match serde_json::from_str::<Entry>(line) {
            Ok(entry) => absorb(&mut replay, entry),
            // Unknown `kind`, a newer `v`, or a torn final line. Only the
            // last one is worth reporting, and only when the file also
            // ended without its newline — otherwise this is a build reading
            // a format it does not have all of, which is not damage.
            Err(_) if last && torn_tail => replay.torn = true,
            Err(_) => {}
        }
    }
    replay
}

fn warnings_say_cost_mixed(warnings: &[String]) -> bool {
    warnings
        .iter()
        .any(|warning| warning.contains("costs were reported in both"))
}

fn absorb(replay: &mut Replay, entry: Entry) {
    match entry {
        Entry::Started {
            v,
            until,
            pinned,
            profile,
        } if v <= JOURNAL_VERSION => {
            replay.profile = profile;
            replay.until = until;
            // Passed as well as pinned: the sweep skips what passed, and a
            // copied output nothing claims is exactly what it archives.
            replay.passed.extend(pinned.iter().cloned());
            replay.pinned = pinned;
        }
        Entry::Started { .. } => {}
        // Read past: the boundary is for a person reading the file. What
        // the run needs from a resume — what passed, what it spent — comes
        // from the attempt lines either side of it.
        Entry::Resumed { .. } => {}
        Entry::Attempt(line) if line.v > JOURNAL_VERSION => {}
        Entry::Attempt(line) => {
            let AttemptLine {
                node,
                attempt,
                evidence_seq,
                outcome,
                invalidated,
                archived,
                git_status,
                at_ms,
                took_ms,
                waited_ms,
                answers,
                turns,
                corrected,
                spent,
                ..
            } = *line;
            replay.next_seq = replay.next_seq.max(evidence_seq + 1);
            for id in &invalidated {
                replay.passed.retain(|passed| passed != id);
            }
            if matches!(outcome, OutcomeLine::Passed) && !replay.passed.contains(&node) {
                replay.passed.push(node.clone());
            }
            let cost_mixed = spent.cost_mixed || warnings_say_cost_mixed(&spent.warnings);
            replay.spent = Spent {
                node_runs: spent.node_runs,
                parked: Duration::from_millis(spent.parked_ms),
                cost: match (spent.cost, spent.currency) {
                    (Some(amount), Some(currency)) => Some(CostLimit { amount, currency }),
                    _ => None,
                },
                cost_mixed,
                warnings: spent.warnings,
            };
            crate::record::push_attempt(
                &mut replay.records,
                &node,
                AttemptRecord {
                    tools: Vec::new(),
                    // Not journaled, like `tools`: the total is, and it adds up.
                    usage: None,
                    attempt,
                    evidence_seq,
                    outcome: match outcome {
                        OutcomeLine::Passed => AttemptOutcome::Passed,
                        OutcomeLine::Canceled => AttemptOutcome::Canceled,
                        // Back as the variant that carries text and decides
                        // nothing: a resumed run reports this attempt, it
                        // does not re-run its policy.
                        OutcomeLine::Failed { reason } => AttemptOutcome::Reported(reason),
                    },
                    invalidated: Invalidation {
                        nodes: invalidated,
                        archived,
                    },
                    git_status,
                    at: std::time::UNIX_EPOCH + Duration::from_millis(at_ms),
                    took: Duration::from_millis(took_ms),
                    waited: Waiting {
                        total: Duration::from_millis(waited_ms),
                        answers: answers
                            .into_iter()
                            .map(|answer| match answer {
                                AnswerLine::Allowed => AskAnswer::Allowed,
                                AnswerLine::Refused => AskAnswer::Refused,
                                AnswerLine::Unanswered => AskAnswer::Unanswered,
                            })
                            .collect(),
                    },
                    // The new key when the line has it, and what the old flag
                    // could assert when it does not.
                    turns: turns.unwrap_or(if corrected { 2 } else { 1 }),
                },
            );
        }
    }
}

#[cfg(test)]
mod tests;
