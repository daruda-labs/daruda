use super::reader::{self, Ending};
use super::*;
use crate::NodeId;
use crate::marker::{RunStatus, run_status};
use crate::record::{AttemptOutcome, AttemptRecord, Invalidation};
use crate::runner::Waiting;

const ONE_AGENT: &str = "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: design
    kind: agent
    output: design.md
    prompt: write
";

fn attempt(n: u32, outcome: AttemptOutcome) -> AttemptRecord {
    AttemptRecord {
        attempt: n,
        evidence_seq: n,
        outcome,
        invalidated: Invalidation::default(),
        git_status: None,
        at: std::time::UNIX_EPOCH,
        took: std::time::Duration::ZERO,
        waited: Waiting::default(),
        tools: Vec::new(),
        turns: 1,
        usage: None,
    }
}

/// A run directory as an older build left it: `progress.jsonl` only.
fn legacy_run(run_dir: &Path) {
    let spent = reader::Spent {
        node_runs: 2,
        ..Default::default()
    };
    crate::journal::start(run_dir, Some("cheap"), None, &[]).expect("start");
    let failed = attempt(
        1,
        AttemptOutcome::Failed(crate::runner::NodeFailure::Refused),
    );
    crate::journal::append_attempt(run_dir, &"design".into(), &failed, &spent).expect("line");
    crate::journal::append_attempt(
        run_dir,
        &"design".into(),
        &attempt(2, AttemptOutcome::Passed),
        &spent,
    )
    .expect("line");
}

/// A run an older build recorded still reads back: what passed, the
/// profile, the spend and every attempt.
#[test]
fn the_reader_reads_a_run_an_older_build_recorded() {
    let dir = tempfile::tempdir().expect("tempdir");
    legacy_run(dir.path());

    assert!(!reader::has_log(dir.path()));
    assert!(reader::started(dir.path()));
    let replay = reader::read(dir.path());
    assert_eq!(replay.passed, vec![NodeId::from("design")]);
    assert_eq!(replay.profile.as_deref(), Some("cheap"));
    assert_eq!(replay.spent.node_runs, 2);
    assert_eq!(replay.next_seq, 3);
    assert_eq!(replay.records[0].attempts.len(), 2);
    assert!(matches!(reader::ending(dir.path()), Ending::NoLog));
}

/// The same questions of a run that keeps a log, through a real run.
#[test]
fn the_reader_reads_a_run_log() {
    let dir = tempfile::tempdir().expect("tempdir");
    let report = crate::schedule::execute(
        &crate::schedule::tests::request_for(ONE_AGENT, dir.path()),
        &crate::testing::FakeRunner::new(),
        &crate::runner::CancelToken::default(),
    );

    let run_dir = &report.run_dir;
    assert!(reader::has_log(run_dir));
    let replay = reader::read(run_dir);
    assert_eq!(replay.passed, vec![NodeId::from("design")]);
    assert_eq!(replay.next_seq, 2);
    assert_eq!(replay.spent.node_runs, report.node_runs);
    assert!(!replay.torn);
    assert!(matches!(
        reader::ending(run_dir),
        Ending::Ended(machine::EndKind::Done)
    ));
}

/// The log wins over a marker that disagrees with it.
#[test]
fn a_marker_that_disagrees_with_the_log_loses() {
    let dir = tempfile::tempdir().expect("tempdir");
    let report = crate::schedule::execute(
        &crate::schedule::tests::request_for(ONE_AGENT, dir.path()),
        &crate::testing::FakeRunner::new(),
        &crate::runner::CancelToken::default(),
    );
    std::fs::write(report.run_dir.join("FAILED"), "").expect("a stale marker");
    std::fs::remove_file(report.run_dir.join("DONE")).expect("the real marker");

    assert_eq!(
        run_status(&report.run_dir, report.run_dir.parent(), &|_| true),
        RunStatus::Done
    );
}

/// A torn last line is read past, the way a crash mid-write leaves it.
#[test]
fn a_torn_last_record_is_read_past() {
    let dir = tempfile::tempdir().expect("tempdir");
    let report = crate::schedule::execute(
        &crate::schedule::tests::request_for(ONE_AGENT, dir.path()),
        &crate::testing::FakeRunner::new(),
        &crate::runner::CancelToken::default(),
    );
    let log = super::log_file(&report.run_dir);
    let mut bytes = std::fs::read(&log).expect("log");
    bytes.extend_from_slice(b"{\"v\":1,\"seq\":99,\"at");
    std::fs::write(&log, bytes).expect("tear");

    let replay = reader::read(&report.run_dir);
    assert!(replay.torn);
    assert_eq!(replay.passed, vec![NodeId::from("design")]);
}

/// A damaged record before the last is refused the way an open refuses it:
/// the reader never answers from the prefix in front of it, and with its
/// writer gone the run is offered for a resume that can say why.
#[test]
fn a_corrupt_middle_record_is_not_read_as_a_prefix() {
    let dir = tempfile::tempdir().expect("tempdir");
    let report = crate::schedule::execute(
        &crate::schedule::tests::request_for(ONE_AGENT, dir.path()),
        &crate::testing::FakeRunner::new(),
        &crate::runner::CancelToken::default(),
    );
    let log = super::log_file(&report.run_dir);
    let text = std::fs::read_to_string(&log).expect("log");
    let mut lines: Vec<&str> = text.lines().collect();
    let passed = lines
        .iter()
        .position(|line| line.contains("\"attempt_passed\""))
        .expect("the pass");
    assert!(passed + 1 < lines.len(), "the pass is not the last record");
    lines[passed] = "{not a record";
    std::fs::write(&log, lines.join("\n") + "\n").expect("damage");
    std::fs::remove_file(report.run_dir.join("DONE")).expect("the marker");

    assert!(reader::state(&report.run_dir).is_none());
    assert!(matches!(
        reader::ending(&report.run_dir),
        Ending::Unreadable
    ));
    assert!(reader::read(&report.run_dir).passed.is_empty());
    let no_lock = dir.path().join("locks");
    assert_eq!(
        run_status(&report.run_dir, Some(&no_lock), &|_| false),
        RunStatus::Crashed
    );
}
