//! What `run.md` says about an attempt, and the two lines that are easy to
//! get wrong: a refusal's reason has to reach the same page as the failure
//! it caused, and a turn's tool use has to save opening the transcript.

/// A settled instant, so a record rendered in a test reads the same on
/// every machine and at every hour. `Instant::now()` in a fixture is a
/// test that passes because nobody looked at the line it produced.
const FIXED_INSTANT: std::time::SystemTime = std::time::SystemTime::UNIX_EPOCH;

use crate::runner::{AskAnswer, Waiting};
use std::time::Duration;

fn waited(total_secs: u64, answers: Vec<AskAnswer>) -> Waiting {
    Waiting {
        total: Duration::from_secs(total_secs),
        answers,
    }
}

/// The gap a real run exposed: a person refused the tool the node
/// needed, the agent correctly declined to write a file claiming it had
/// done the work, and `run.md` said only "no output written". The
/// refusal *is* the reason, and a reader had to open the transcript to
/// find it.
#[test]
fn a_refusal_is_on_the_same_page_as_the_failure_it_caused() {
    let mut out = String::new();
    push_attempt_lines(
        &mut out,
        &AttemptRecord {
            tools: Vec::new(),
            attempt: 1,
            evidence_seq: 1,
            at: FIXED_INSTANT,
            took: Duration::from_secs(0),
            outcome: AttemptOutcome::Failed(NodeFailure::NoOutput {
                expected: PathBuf::from("/run/touched.md"),
            }),
            invalidated: Invalidation::default(),
            git_status: None,
            waited: waited(1, vec![AskAnswer::Refused]),
            corrected: false,
        },
        Path::new("/run"),
    );
    assert!(out.contains("refused"), "{out}");
    assert!(out.contains("no output written"), "{out}");
}

/// The line that saves opening the transcript: what it reached for, how
/// often, and which of those did not come back.
#[test]
fn what_a_turn_reached_for_is_grouped_and_its_failures_named() {
    use crate::runner::{ToolOutcome, ToolUse};
    let tool = |name: &str, outcome| ToolUse {
        name: name.to_string(),
        outcome,
    };
    let said = tools_said(&[
        tool("read", ToolOutcome::Ok),
        tool("read", ToolOutcome::Ok),
        tool("execute", ToolOutcome::Failed),
        tool("edit", ToolOutcome::Unsettled),
    ]);
    assert_eq!(said, "read x2, execute (1 failed), edit (1 unsettled)");
}

/// A node that used nothing says nothing — the line is for a reader who
/// is looking for the tools, not a reader counting empty ones.
#[test]
fn a_turn_that_reached_for_nothing_adds_no_line() {
    let mut out = String::new();
    push_attempt_lines(
        &mut out,
        &AttemptRecord {
            attempt: 1,
            evidence_seq: 1,
            at: FIXED_INSTANT,
            took: Duration::from_secs(0),
            outcome: AttemptOutcome::Passed,
            invalidated: Invalidation::default(),
            git_status: None,
            waited: Waiting::default(),
            corrected: false,
            tools: Vec::new(),
        },
        Path::new("/run"),
    );
    assert!(!out.contains("used"), "{out}");
}

/// An approval needs no telling — the work went ahead, which the
/// outcome already says. Only the duration is worth a line.
#[test]
fn an_approval_adds_no_commentary() {
    let mut out = String::new();
    push_attempt_lines(
        &mut out,
        &AttemptRecord {
            tools: Vec::new(),
            attempt: 1,
            evidence_seq: 1,
            at: FIXED_INSTANT,
            took: Duration::from_secs(0),
            outcome: AttemptOutcome::Passed,
            invalidated: Invalidation::default(),
            git_status: None,
            waited: waited(143, vec![AskAnswer::Allowed]),
            corrected: false,
        },
        Path::new("/run"),
    );
    assert!(out.contains("waited 143s"), "{out}");
    assert!(!out.contains("refused"), "{out}");
}

/// A correction is a second turn on the same session. The budget-unit
/// total includes it, but without this line an attempt that only passed
/// on its correction is identical to one that got it right first time.
#[test]
fn a_corrected_attempt_says_so() {
    let mut out = String::new();
    let mut attempt = AttemptRecord {
        tools: Vec::new(),
        attempt: 1,
        evidence_seq: 1,
        at: FIXED_INSTANT,
        took: Duration::from_secs(0),
        outcome: AttemptOutcome::Passed,
        invalidated: Invalidation::default(),
        git_status: None,
        waited: Waiting::default(),
        corrected: true,
    };
    push_attempt_lines(&mut out, &attempt, Path::new("/run"));
    assert!(out.contains("correction turn"), "{out}");

    let mut plain = String::new();
    attempt.corrected = false;
    push_attempt_lines(&mut plain, &attempt, Path::new("/run"));
    assert!(!plain.contains("correction"), "{plain}");
}

/// An attempt nobody was asked anything in says nothing about waiting.
#[test]
fn an_attempt_that_asked_nothing_stays_silent() {
    let mut out = String::new();
    push_attempt_lines(
        &mut out,
        &AttemptRecord {
            tools: Vec::new(),
            attempt: 1,
            evidence_seq: 1,
            at: FIXED_INSTANT,
            took: Duration::from_secs(0),
            outcome: AttemptOutcome::Passed,
            invalidated: Invalidation::default(),
            git_status: None,
            waited: Waiting::default(),
            corrected: false,
        },
        Path::new("/run"),
    );
    assert!(!out.contains("waited"), "{out}");
}

use super::*;

fn attempt(n: u32) -> AttemptRecord {
    AttemptRecord {
        tools: Vec::new(),
        attempt: n,
        evidence_seq: n,
        at: FIXED_INSTANT,
        took: Duration::from_secs(0),
        outcome: AttemptOutcome::Passed,
        invalidated: Invalidation {
            nodes: Vec::new(),
            archived: Vec::new(),
        },
        git_status: None,
        waited: Default::default(),
        corrected: false,
    }
}

/// A report with nothing but an ending and whatever the run had to say
/// about itself — the shape the lead of `run.md` is rendered from.
fn report_with(outcome: RunOutcome, warnings: Vec<String>) -> RunReport {
    let mut report = RunReport::refused(PathBuf::from("run"), outcome);
    for warning in warnings {
        report.warn(warning);
    }
    report
}

/// The first thing a user reads has to be whether the guard rails were
/// actually guarding. A cost limit that never applied looks identical to
/// one that held, unless the report says so.
#[test]
fn run_md_leads_with_whether_the_cost_limit_applied() {
    let report = report_with(
        RunOutcome::Done,
        vec!["nothing reported a cost, so the 5 USD limit never applied".to_string()],
    );
    let md = render_run_md(&report);
    let first_lines: String = md.lines().take(6).collect::<Vec<_>>().join("\n");
    assert!(first_lines.contains("never applied"), "{md}");
    // The engine's warning is the evidence; the lead has to state the
    // conclusion itself, because a run with no limit set issues no
    // warning and still leaves the ceiling unmeasurable.
    assert!(first_lines.contains("no agent reported a cost"), "{md}");
}

/// A run that measured a cost and one that measured none must not read
/// the same — that difference is the whole point of the leading line.
///
/// The amount is a running sum of `f64`s, so it arrives carrying its own
/// accumulation error. A ledger line reading `0.30000000000000004 USD`
/// is noise in a document about whether a ceiling held.
#[test]
fn run_md_says_so_when_a_cost_was_measured() {
    let mut report = report_with(RunOutcome::Done, Vec::new());
    report.cost = Some(CostLimit {
        amount: 0.1 + 0.2,
        currency: "USD".to_string(),
    });
    let md = render_run_md(&report);
    let first_lines: String = md.lines().take(6).collect::<Vec<_>>().join("\n");
    assert!(first_lines.contains("0.3 USD"), "{md}");
    assert!(!first_lines.contains("no agent reported a cost"), "{md}");
}

/// A failure's reason is the thing someone opens this file for. Naming
/// only the node would leave them re-reading logs to find out why.
#[test]
fn run_md_names_the_node_and_the_reason_it_failed() {
    let report = report_with(
        RunOutcome::Failed {
            node: "gate".into(),
            failure: NodeFailure::Exit { code: Some(2) },
        },
        Vec::new(),
    );
    let md = render_run_md(&report);
    assert!(md.contains("gate"), "{md}");
    assert!(md.contains("exit"), "{md}");
    // The preflight refuses a node before any session, so there is a
    // failure to report and no attempt to report it under. Saying
    // "nothing ran" under a line naming the node that failed leaves a
    // reader with two statements to choose between.
    assert!(!md.contains("Nothing ran"), "{md}");
    assert!(md.contains("before a session opened"), "{md}");
}

/// Every attempt, not just the last: a run that passed on attempt 3 is a
/// different story from one that passed first time, and the difference is
/// what a user tunes their flow on.
#[test]
fn run_md_lists_every_attempt_with_its_evidence() {
    let mut report = report_with(RunOutcome::Done, Vec::new());
    report.node_runs = 2;
    report.nodes = vec![NodeRecord {
        id: "gate".into(),
        attempts: vec![
            AttemptRecord {
                tools: Vec::new(),
                attempt: 1,
                evidence_seq: 3,
                at: FIXED_INSTANT,
                took: Duration::from_secs(0),
                outcome: AttemptOutcome::Failed(NodeFailure::Exit { code: Some(1) }),
                invalidated: Invalidation {
                    nodes: Vec::new(),
                    archived: vec![PathBuf::from("run/logs/gate.attempt-1.evidence-3.log")],
                },
                git_status: None,
                waited: Default::default(),
                corrected: false,
            },
            AttemptRecord {
                tools: Vec::new(),
                attempt: 2,
                evidence_seq: 6,
                at: FIXED_INSTANT,
                took: Duration::from_secs(0),
                outcome: AttemptOutcome::Passed,
                invalidated: Invalidation {
                    nodes: Vec::new(),
                    archived: Vec::new(),
                },
                git_status: None,
                waited: Default::default(),
                corrected: false,
            },
        ],
    }];
    let md = render_run_md(&report);
    assert!(md.contains("**Budget units:** 2 · **Nodes:** 1"), "{md}");
    assert!(!md.contains("**Sessions:**"), "{md}");
    assert!(
        md.contains("attempt 1 (evidence 3) — failed: exited with status 1"),
        "{md}"
    );
    assert!(md.contains("attempt 2 (evidence 6) — passed"), "{md}");
    // The evidence belongs to the attempt that produced it, so the path
    // has to be there and not merely the fact that something was moved —
    // named from the run directory this file itself sits in.
    assert!(
        md.contains("archived `logs/gate.attempt-1.evidence-3.log`"),
        "{md}"
    );
}

/// The design records the tree because the engine deliberately does not
/// manage it. A run that dirtied the tree and a run that did not must not
/// read the same.
#[test]
fn run_md_shows_the_tree_state_when_the_host_answered() {
    let mut report = report_with(RunOutcome::Done, Vec::new());
    report.node_runs = 2;
    report.nodes = vec![NodeRecord {
        id: "design".into(),
        attempts: vec![
            AttemptRecord {
                tools: Vec::new(),
                attempt: 1,
                evidence_seq: 1,
                at: FIXED_INSTANT,
                took: Duration::from_secs(0),
                outcome: AttemptOutcome::Passed,
                invalidated: Invalidation {
                    nodes: Vec::new(),
                    archived: Vec::new(),
                },
                git_status: Some(" M src/lib.rs\n?? notes.md".to_string()),
                waited: Default::default(),
                corrected: false,
            },
            // The host answering "nothing changed" is an answer, not a
            // silence — a clean tree after an attempt is a fact.
            AttemptRecord {
                tools: Vec::new(),
                attempt: 1,
                evidence_seq: 2,
                at: FIXED_INSTANT,
                took: Duration::from_secs(0),
                outcome: AttemptOutcome::Passed,
                invalidated: Invalidation {
                    nodes: Vec::new(),
                    archived: Vec::new(),
                },
                git_status: Some(String::new()),
                waited: Default::default(),
                corrected: false,
            },
        ],
    }];
    let md = render_run_md(&report);
    assert!(md.contains(" M src/lib.rs"), "{md}");
    assert!(md.contains("?? notes.md"), "{md}");
    assert!(md.contains("clean"), "{md}");

    // A host with nothing to say leaves no note at all, rather than an
    // empty one a reader would take for a clean tree.
    let mut silent = report_with(RunOutcome::Done, Vec::new());
    silent.node_runs = 1;
    silent.nodes = vec![NodeRecord {
        id: "design".into(),
        attempts: vec![attempt(1)],
    }];
    assert!(!render_run_md(&silent).contains("working tree"), "{md}");
}
