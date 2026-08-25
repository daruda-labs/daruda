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
    /// Whether the attempt used a second turn to correct its output.
    /// Absent on every journal written before it existed, which reads back
    /// as `false` — an older line describes a run that could not have.
    #[serde(default, skip_serializing_if = "is_false")]
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
            corrected: attempt.corrected,
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
                    corrected,
                },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A settled instant, so a record rendered in a test reads the same on
    /// every machine and at every hour. `Instant::now()` in a fixture is a
    /// test that passes because nobody looked at the line it produced.
    const FIXED_INSTANT: std::time::SystemTime = std::time::SystemTime::UNIX_EPOCH;

    fn attempt(n: u32, seq: u32, outcome: AttemptOutcome) -> AttemptRecord {
        AttemptRecord {
            tools: Vec::new(),
            attempt: n,
            evidence_seq: seq,
            at: FIXED_INSTANT,
            took: Duration::from_secs(0),
            outcome,
            invalidated: Invalidation::default(),
            git_status: None,
            waited: Waiting::default(),
            corrected: false,
        }
    }

    fn spent(runs: u32) -> Spent {
        Spent {
            node_runs: runs,
            ..Spent::default()
        }
    }

    /// A journal an earlier build left behind still reads. The line is
    /// written out here by hand rather than round-tripped, because a
    /// round-trip through our own writer agrees with itself no matter what
    /// shape it picked — and what has to hold is that `node` and
    /// `invalidated` are still bare strings, which is how every journal
    /// already on disk spells them.
    #[test]
    fn a_journal_written_before_an_id_was_a_type_still_reads() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(JOURNAL_FILE);
        std::fs::write(
            &path,
            concat!(
                r#"{"kind":"started","v":1}"#,
                "\n",
                r#"{"kind":"attempt","v":1,"node":"design","attempt":1,"#,
                r#""evidence_seq":1,"outcome":{"result":"passed"},"#,
                r#""invalidated":["stale"],"spent":{"node_runs":1}}"#,
                "\n",
            ),
        )
        .expect("write");

        let replay = read(dir.path());
        assert_eq!(replay.passed, vec![NodeId::from("design")]);
        assert_eq!(
            replay.records[0].attempts[0].invalidated.nodes,
            vec![NodeId::from("stale")]
        );
        assert!(!replay.torn, "a line an older build wrote is not damage");
    }

    /// The other half of the same contract: what we write now is still what
    /// an earlier build would recognise.
    #[test]
    fn an_id_reaches_the_file_as_a_bare_string() {
        let dir = tempfile::tempdir().expect("tempdir");
        start(dir.path(), None, None, &[]).expect("start");
        append_attempt(
            dir.path(),
            &"design".into(),
            &attempt(1, 1, AttemptOutcome::Passed),
            &spent(1),
        )
        .expect("append");
        let text = std::fs::read_to_string(dir.path().join(JOURNAL_FILE)).expect("read");
        assert!(text.contains(r#""node":"design""#), "{text}");
    }

    /// The whole point: which nodes passed survives the process. Nothing
    /// else on disk can say it — a command node's pass writes no file.
    #[test]
    fn what_passed_reads_back() {
        let dir = tempfile::tempdir().expect("tempdir");
        start(dir.path(), Some("cheap"), None, &[]).expect("start");
        append_attempt(
            dir.path(),
            &"design".into(),
            &attempt(1, 1, AttemptOutcome::Passed),
            &spent(1),
        )
        .expect("append");
        append_attempt(
            dir.path(),
            &"gate".into(),
            &attempt(
                1,
                2,
                AttemptOutcome::Reported("failed: no output".to_string()),
            ),
            &spent(2),
        )
        .expect("append");

        let replay = read(dir.path());
        assert_eq!(replay.passed, vec![NodeId::from("design")]);
        assert_eq!(replay.profile.as_deref(), Some("cheap"));
        assert_eq!(replay.spent.node_runs, 2);
        assert_eq!(replay.records.len(), 2, "both nodes have a history");
        assert!(!replay.torn);
    }

    /// A repair failure invalidates outputs that used to be good. If the
    /// process dies after that line, a resume must re-run those nodes rather
    /// than skipping them as already passed.
    #[test]
    fn invalidation_takes_nodes_back_out_of_the_passed_set() {
        let dir = tempfile::tempdir().expect("tempdir");
        start(dir.path(), None, None, &[]).expect("start");
        append_attempt(
            dir.path(),
            &"design".into(),
            &attempt(1, 1, AttemptOutcome::Passed),
            &spent(1),
        )
        .expect("append");
        let mut gate = attempt(
            1,
            2,
            AttemptOutcome::Reported("failed: exit status 1".to_string()),
        );
        gate.invalidated.nodes = vec!["design".into(), "gate".into()];
        append_attempt(dir.path(), &"gate".into(), &gate, &spent(2)).expect("append");

        assert!(
            read(dir.path()).passed.is_empty(),
            "an invalidated node was still treated as passed"
        );
    }

    /// Once currencies mix, later costs are ignored. That is state, not a
    /// warning alone, so a resume has to keep carrying it.
    #[test]
    fn mixed_cost_accounting_reads_back() {
        let dir = tempfile::tempdir().expect("tempdir");
        start(dir.path(), None, None, &[]).expect("start");
        append_attempt(
            dir.path(),
            &"design".into(),
            &attempt(1, 1, AttemptOutcome::Passed),
            &Spent {
                node_runs: 1,
                cost: Some(CostLimit {
                    amount: 2.0,
                    currency: "USD".to_string(),
                }),
                cost_mixed: true,
                ..Spent::default()
            },
        )
        .expect("append");

        assert!(read(dir.path()).spent.cost_mixed);
    }

    /// The evidence counter continues. Restarting it would make the first
    /// attempt after a resume write `evidence-1` over a log that is still
    /// the only account of what happened the first time.
    #[test]
    fn the_evidence_counter_continues_where_it_stopped() {
        let dir = tempfile::tempdir().expect("tempdir");
        start(dir.path(), None, None, &[]).expect("start");
        for seq in 1..=4 {
            append_attempt(
                dir.path(),
                &"a".into(),
                &attempt(seq, seq, AttemptOutcome::Passed),
                &spent(seq),
            )
            .expect("append");
        }
        assert_eq!(read(dir.path()).next_seq, 5);
    }

    /// The writer is a process that can be killed mid-write, so the last
    /// line can be half a line. It is dropped, everything before it stands,
    /// and the reader says so rather than pretending the run stopped there.
    #[test]
    fn a_line_torn_by_the_kill_costs_only_itself() {
        let dir = tempfile::tempdir().expect("tempdir");
        start(dir.path(), None, None, &[]).expect("start");
        append_attempt(
            dir.path(),
            &"design".into(),
            &attempt(1, 1, AttemptOutcome::Passed),
            &spent(1),
        )
        .expect("append");
        let path = dir.path().join(JOURNAL_FILE);
        let mut text = std::fs::read_to_string(&path).expect("read");
        text.push_str("{\"kind\":\"attempt\",\"v\":1,\"node\":\"ga");
        std::fs::write(&path, text).expect("write");

        let replay = read(dir.path());
        assert_eq!(replay.passed, vec![NodeId::from("design")]);
        assert_eq!(replay.spent.node_runs, 1, "the torn line was counted");
        assert!(replay.torn, "the tear was swallowed");
    }

    /// A build reading a newer journal skips what it does not know instead
    /// of refusing the whole file — the shape is expected to change, and a
    /// resume that dies on an unknown line is worse than one that resumes
    /// from what it understood.
    #[test]
    fn an_entry_from_a_newer_build_is_skipped_not_fatal() {
        let dir = tempfile::tempdir().expect("tempdir");
        start(dir.path(), None, None, &[]).expect("start");
        append_attempt(
            dir.path(),
            &"design".into(),
            &attempt(1, 1, AttemptOutcome::Passed),
            &spent(1),
        )
        .expect("append");
        let path = dir.path().join(JOURNAL_FILE);
        let mut text = std::fs::read_to_string(&path).expect("read");
        text.push_str("{\"kind\":\"from_the_future\",\"v\":99}\n");
        text.push_str("{\"kind\":\"attempt\",\"v\":99,\"node\":\"x\"}\n");
        std::fs::write(&path, text).expect("write");

        let replay = read(dir.path());
        assert_eq!(replay.passed, vec![NodeId::from("design")]);
        assert!(!replay.torn, "a newer entry is not damage");
    }

    /// **The trap this closes.** `absorb` destructures with a trailing `..`,
    /// so a field added to the line compiles clean and is silently dropped on
    /// resume — a corrected attempt would read back as an ordinary one, and
    /// the record of a run continued after a crash would understate what it
    /// cost. Nothing but a round trip catches that.
    #[test]
    fn a_correction_survives_the_round_trip() {
        let dir = tempfile::tempdir().expect("tempdir");
        start(dir.path(), None, None, &[]).expect("start");
        let mut corrected = attempt(1, 1, AttemptOutcome::Passed);
        corrected.corrected = true;
        append_attempt(dir.path(), &"design".into(), &corrected, &spent(2)).expect("append");
        append_attempt(
            dir.path(),
            &"review".into(),
            &attempt(1, 2, AttemptOutcome::Passed),
            &spent(3),
        )
        .expect("append");

        let replay = read(dir.path());
        assert!(
            replay.records[0].attempts[0].corrected,
            "the correction was dropped on the way back"
        );
        assert!(
            !replay.records[1].attempts[0].corrected,
            "an ordinary attempt must not read as corrected"
        );
    }

    /// The field is skipped when false, so a journal an older build wrote —
    /// which has no such key — still reads, and every line a run without
    /// corrections writes is unchanged.
    #[test]
    fn an_uncorrected_attempt_writes_no_such_key() {
        let dir = tempfile::tempdir().expect("tempdir");
        append_attempt(
            dir.path(),
            &"design".into(),
            &attempt(1, 1, AttemptOutcome::Passed),
            &spent(1),
        )
        .expect("append");
        let text = std::fs::read_to_string(dir.path().join(JOURNAL_FILE)).expect("read");
        assert!(!text.contains("corrected"), "{text}");
    }

    /// A directory with no journal is a run that never got past setup, not
    /// an error — `read` answers with an empty replay and `exists` is how a
    /// caller tells the two apart.
    #[test]
    fn a_run_with_no_journal_reads_empty() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(!exists(dir.path()));
        let replay = read(dir.path());
        assert!(replay.passed.is_empty() && replay.records.is_empty());
        assert_eq!(replay.next_seq, 0);
    }

    /// The selection has to survive a crash, or the continuation runs the
    /// nodes the first process was told to skip.
    #[test]
    fn the_selection_reads_back() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = NodeId::from("design");
        start(dir.path(), None, Some(&target), &[]).expect("start");
        assert_eq!(read(dir.path()).until.as_ref(), Some(&target));
    }

    #[test]
    fn a_run_with_no_selection_reads_back_none() {
        let dir = tempfile::tempdir().expect("tempdir");
        start(dir.path(), None, None, &[]).expect("start");
        assert_eq!(read(dir.path()).until, None);
    }
}
