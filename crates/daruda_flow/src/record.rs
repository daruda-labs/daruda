//! What a run did, attempt by attempt. `RunReport` carries the totals; this
//! is the detail behind them — what was tried, how it ended, what evidence
//! it left, and what the working tree looked like afterwards — plus the
//! `run.md` rendering of all of it, which is the form a person reads.

use crate::NodeId;
use crate::request::CostLimit;
use crate::runner::NodeFailure;
use crate::schedule::{BudgetLimit, RunOutcome, RunReport};
use std::path::{Path, PathBuf};

/// The working tree's state right now, as `git status --porcelain` prints
/// it. `None` when the host has nothing to say — not a git repo, or the
/// command failed. Best-effort by design: this is an audit note, and a
/// missing one must never fail a run.
pub type GitStatus<'a> = Option<&'a dyn Fn() -> Option<String>>;

/// One node's history this run. A node can appear once (it passed) or many
/// times over several generations (a gate's repair re-derived it).
#[derive(Debug)]
pub struct NodeRecord {
    pub id: NodeId,
    pub attempts: Vec<AttemptRecord>,
}

#[derive(Debug)]
pub struct AttemptRecord {
    /// The node's own counter, which resets each generation — so this is not
    /// unique within a run. `evidence_seq` is.
    pub attempt: u32,
    pub evidence_seq: u32,
    pub outcome: AttemptOutcome,
    /// What this attempt's failure invalidated: the nodes whose outputs
    /// stopped being current, and where their evidence was moved. Empty
    /// unless a policy re-derived something — a passing attempt, or a
    /// failure that ended the run before any set was computed.
    ///
    /// Without this the record shows a re-derivation only by implication
    /// (a node appearing twice at attempt 1 with different evidence ids),
    /// which leaves the reader to guess which failure caused it.
    pub invalidated: Invalidation,
    /// `git status --porcelain` right after this attempt, when the host
    /// answers. Best-effort: the design records the tree's state because the
    /// engine deliberately does not manage it.
    pub git_status: Option<String>,
}

/// The blast radius of one failed attempt. The two travel together because
/// they are computed together — the nodes are the invalidation set and the
/// paths are where that same set's evidence went.
#[derive(Debug, Default)]
pub struct Invalidation {
    /// The set the failure invalidated, gate included. Design §10 calls this
    /// the rerun set and asks `run.md` to name it.
    pub nodes: Vec<NodeId>,
    pub archived: Vec<PathBuf>,
}

/// How one attempt ended. Three states, not `Option<NodeFailure>`: a cancel
/// is neither a pass nor a failure, and collapsing it into either makes the
/// record lie about why the run stopped.
#[derive(Debug)]
pub enum AttemptOutcome {
    Passed,
    Failed(NodeFailure),
    Canceled,
}

/// Append `attempt` to `id`'s history, starting one if this is the node's
/// first. One place grows the list, because the scheduler seals an attempt's
/// fate at five different sites and each of them would otherwise carry its
/// own copy of the find-or-insert.
pub(crate) fn push_attempt(records: &mut Vec<NodeRecord>, id: &NodeId, attempt: AttemptRecord) {
    match records.iter_mut().find(|record| &record.id == id) {
        Some(record) => record.attempts.push(attempt),
        None => records.push(NodeRecord {
            id: id.clone(),
            attempts: vec![attempt],
        }),
    }
}

/// What the run's account of itself is called. Named here beside the
/// renderer rather than beside the writer, because a host that wants to
/// open the file needs it too and a second literal would be a second
/// thing to keep right.
pub const RUN_REPORT_FILE: &str = "run.md";

/// The run's own account of itself — the design's `run.md`. Pure, so the
/// layout is settled without a filesystem; `schedule::execute` writes it.
///
/// The lead is three kinds of line and nothing else: how the run ended,
/// whether the cost ceiling was ever measurable, and what the run had to say.
/// Design §6 puts the cost line there because a limit nothing reported
/// against is indistinguishable from one that held — and an overnight run is
/// started on the strength of that belief.
pub fn render_run_md(report: &RunReport) -> String {
    let mut out = String::from("# Run\n\n");
    out.push_str(&format!("- **Result** — {}\n", result_of(&report.outcome)));
    out.push_str(&format!(
        "- **Cost limit** — {}\n",
        cost_standing(report.cost.as_ref())
    ));
    for warning in report.warnings() {
        out.push_str(&format!("- **Warning** — {warning}\n"));
    }

    out.push_str("\n## Attempts\n\n");
    if report.nodes.is_empty() {
        out.push_str("Nothing ran.\n");
        return out;
    }
    out.push_str(&format!(
        "**Sessions:** {} · **Nodes:** {}\n",
        report.node_runs,
        report.nodes.len()
    ));
    for node in &report.nodes {
        out.push_str(&format!("\n### `{}`\n\n", node.id));
        for attempt in &node.attempts {
            push_attempt_lines(&mut out, attempt, &report.run_dir);
        }
    }
    out
}

/// What happened, in one clause. Every variant answers "and why", because a
/// reader who has to open another file for that is back to reading logs.
fn result_of(outcome: &RunOutcome) -> String {
    match outcome {
        RunOutcome::Done => "done: every node passed.".to_string(),
        RunOutcome::Failed { node, failure } => format!("failed at `{node}`: {failure}."),
        RunOutcome::Canceled { node: Some(node) } => {
            format!("canceled while `{node}` was running.")
        }
        RunOutcome::Canceled { node: None } => "canceled between nodes.".to_string(),
        RunOutcome::BudgetExhausted { limit } => format!("stopped by {}.", limit_of(limit)),
        RunOutcome::Io(e) => format!("stopped by an I/O failure: {e}."),
        RunOutcome::Invalid { issues } => format!(
            "never started: the request had {} problem(s), the first being {}.",
            issues.len(),
            issues
                .first()
                .map(|i| i.message.as_str())
                .unwrap_or("unstated")
        ),
        RunOutcome::LockHeld { holder } => format!(
            "never started: run `{}` (pid {}) holds this working directory.",
            holder.run_id, holder.pid
        ),
        RunOutcome::Unprovisioned { agent, message } => {
            format!("never ran: `{agent}`'s runtime could not be prepared: {message}.")
        }
    }
}

fn limit_of(limit: &BudgetLimit) -> &'static str {
    match limit {
        BudgetLimit::WallClock => "the run's wall-clock limit",
        BudgetLimit::NodeRuns => "the run's node-run limit",
        BudgetLimit::Cost => "the run's cost limit",
    }
}

/// Whether a cost ceiling could have applied at all. A total is the only
/// cumulative figure there is, so no total means no ceiling was ever measured
/// against — whether or not the user set one.
fn cost_standing(total: Option<&CostLimit>) -> String {
    match total {
        Some(total) => format!(
            "enforceable: agents reported {} {} in total, which a limit in that currency is \
             measured against.",
            money(total.amount),
            total.currency
        ),
        None => "not enforceable: no agent reported a cost, so nothing was measured against a \
                 ceiling — the node-run cap and the per-node timeouts were the only limits in \
                 force."
            .to_string(),
    }
}

/// A cost as a reader expects to see it. The run's total is a sum of `f64`s,
/// which prints its own accumulation error — `0.42000000000000004` in a
/// document about whether a 5 USD ceiling held is noise, not precision.
fn money(amount: f64) -> String {
    let text = format!("{amount:.4}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// One attempt and everything it left behind. `attempt` repeats across repair
/// generations, so `evidence_seq` travels with it — that pair is what tells
/// two runs of the same node apart.
///
/// Evidence is named relative to `run_dir`, which is where this file itself
/// lives: an absolute path here is one long prefix repeated on every line.
fn push_attempt_lines(out: &mut String, attempt: &AttemptRecord, run_dir: &Path) {
    out.push_str(&format!(
        "- attempt {} (evidence {}) — {}\n",
        attempt.attempt,
        attempt.evidence_seq,
        ended_as(&attempt.outcome)
    ));
    // The set first: it says why the attempts below it happened again,
    // which is otherwise only inferable from repeated attempt numbers.
    if !attempt.invalidated.nodes.is_empty() {
        let named: Vec<String> = attempt
            .invalidated
            .nodes
            .iter()
            .map(|id| format!("`{id}`"))
            .collect();
        out.push_str(&format!("  - re-derived {}\n", named.join(", ")));
    }
    for path in &attempt.invalidated.archived {
        let shown = path.strip_prefix(run_dir).unwrap_or(path);
        out.push_str(&format!("  - archived `{}`\n", shown.display()));
    }
    match attempt.git_status.as_deref() {
        // No note at all, rather than an empty one: a host with nothing to
        // say has not told us the tree was clean.
        None => {}
        Some(status) if status.trim().is_empty() => out.push_str("  - working tree: clean\n"),
        Some(status) => {
            out.push_str("  - working tree:\n");
            for line in status.lines() {
                out.push_str(&format!("    - `{line}`\n"));
            }
        }
    }
}

fn ended_as(outcome: &AttemptOutcome) -> String {
    match outcome {
        AttemptOutcome::Passed => "passed".to_string(),
        AttemptOutcome::Failed(failure) => format!("failed: {failure}"),
        AttemptOutcome::Canceled => "canceled".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attempt(n: u32) -> AttemptRecord {
        AttemptRecord {
            attempt: n,
            evidence_seq: n,
            outcome: AttemptOutcome::Passed,
            invalidated: Invalidation {
                nodes: Vec::new(),
                archived: Vec::new(),
            },
            git_status: None,
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
                node: "gate".to_string(),
                failure: NodeFailure::Exit { code: Some(2) },
            },
            Vec::new(),
        );
        let md = render_run_md(&report);
        assert!(md.contains("gate"), "{md}");
        assert!(md.contains("exit"), "{md}");
    }

    /// Every attempt, not just the last: a run that passed on attempt 3 is a
    /// different story from one that passed first time, and the difference is
    /// what a user tunes their flow on.
    #[test]
    fn run_md_lists_every_attempt_with_its_evidence() {
        let mut report = report_with(RunOutcome::Done, Vec::new());
        report.node_runs = 2;
        report.nodes = vec![NodeRecord {
            id: "gate".to_string(),
            attempts: vec![
                AttemptRecord {
                    attempt: 1,
                    evidence_seq: 3,
                    outcome: AttemptOutcome::Failed(NodeFailure::Exit { code: Some(1) }),
                    invalidated: Invalidation {
                        nodes: Vec::new(),
                        archived: vec![PathBuf::from("run/logs/gate.attempt-1.evidence-3.log")],
                    },
                    git_status: None,
                },
                AttemptRecord {
                    attempt: 2,
                    evidence_seq: 6,
                    outcome: AttemptOutcome::Passed,
                    invalidated: Invalidation {
                        nodes: Vec::new(),
                        archived: Vec::new(),
                    },
                    git_status: None,
                },
            ],
        }];
        let md = render_run_md(&report);
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
            id: "design".to_string(),
            attempts: vec![
                AttemptRecord {
                    attempt: 1,
                    evidence_seq: 1,
                    outcome: AttemptOutcome::Passed,
                    invalidated: Invalidation {
                        nodes: Vec::new(),
                        archived: Vec::new(),
                    },
                    git_status: Some(" M src/lib.rs\n?? notes.md".to_string()),
                },
                // The host answering "nothing changed" is an answer, not a
                // silence — a clean tree after an attempt is a fact.
                AttemptRecord {
                    attempt: 1,
                    evidence_seq: 2,
                    outcome: AttemptOutcome::Passed,
                    invalidated: Invalidation {
                        nodes: Vec::new(),
                        archived: Vec::new(),
                    },
                    git_status: Some(String::new()),
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
            id: "design".to_string(),
            attempts: vec![attempt(1)],
        }];
        assert!(!render_run_md(&silent).contains("working tree"), "{md}");
    }
}
