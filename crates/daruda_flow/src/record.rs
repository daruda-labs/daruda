//! What a run did, attempt by attempt. `RunReport` carries the totals; this
//! is the detail behind them — what was tried, how it ended, what evidence
//! it left, and what the working tree looked like afterwards — plus the
//! `run.md` rendering of all of it, which is the form a person reads.

use crate::NodeId;
use crate::request::CostLimit;
use crate::runner::NodeFailure;
use crate::schedule::{BudgetLimit, RunOutcome, RunReport};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The working tree's state right now, as `git status --porcelain` prints
/// it. `None` when the host has nothing to say — not a git repo, or the
/// command failed. Best-effort by design: this is an audit note, and a
/// missing one must never fail a run.
/// How the host reports a working tree's state, asked about the directory
/// the attempt actually ran in — which is the node's own when it names one,
/// and not the run's.
pub type GitStatus<'a> = Option<&'a dyn Fn(&std::path::Path) -> Option<String>>;

/// One node's history this run. A node can appear once (it passed) or many
/// times over several generations (a gate's repair re-derived it).
#[derive(Debug, Clone)]
pub struct NodeRecord {
    pub id: NodeId,
    pub attempts: Vec<AttemptRecord>,
}

#[derive(Debug, Clone)]
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
    /// When this attempt settled, and how long it had been going.
    ///
    /// Wall clock, so it can be lined up against anything else that was
    /// happening — the app's log, another run, a person's memory of when
    /// they walked away. Without it the record says what happened in what
    /// order and leaves *when* to be guessed from file timestamps, which
    /// is what a reader ends up doing.
    pub at: std::time::SystemTime,
    /// Wall time from the attempt starting to it settling, waiting
    /// included. Two nodes running together have overlapping spans, so
    /// this cannot be derived by subtracting one attempt's `at` from the
    /// next one's.
    pub took: Duration,
    /// What this attempt spent waiting for a person, and what they said.
    ///
    /// The duration is recorded because the clocks *stop* for it — without
    /// it the account cannot explain why an attempt took forty minutes. The
    /// answers are recorded because they are often *why the attempt ended
    /// the way it did*: an agent refused its tool and then, correctly,
    /// writing nothing reads as a plain "no output written" unless the
    /// refusal is on the same page.
    pub waited: crate::runner::Waiting,
    /// What the attempt's turn reached for. A diagnostic aid so `run.md`
    /// answers "did it try to write the file" without opening the transcript.
    /// Not journaled: a resumed run's earlier attempts lose it, which costs a
    /// reader nothing the transcript beside them does not still say.
    pub tools: Vec<crate::runner::ToolUse>,
    /// Whether this attempt used a second turn to correct its output.
    /// Recorded because it consumes another budget unit and nothing else in
    /// the line would say so.
    pub corrected: bool,
    /// What this attempt's session reported it had used by the time it ended.
    ///
    /// The run already sums the cost to bound it, and a ceiling is not an
    /// answer to "which node was expensive" — the sum says a flow cost $0.71
    /// and leaves the reader to guess which of five nodes spent it. Each node
    /// is its own session, so the figure a session ends on *is* that node's,
    /// which is what makes keeping it per attempt as honest as summing it.
    ///
    /// `None` for a command node and for an agent whose adapter reports
    /// nothing. Not journaled, for the same reason [`Self::tools`] is not: a
    /// resumed run's earlier attempts lose it, and the total — which is
    /// journaled — still adds up.
    pub usage: Option<daruda_acp::UsageView>,
}

/// What a runner call reported, for the record. One struct because every
/// piece comes from one `RunResult` and the call sites were threading them
/// positionally; `Default` is the refusal that never made a call.
#[derive(Clone, Default)]
pub(crate) struct Reported {
    pub(crate) waited: crate::runner::Waiting,
    pub(crate) corrected: bool,
    pub(crate) tools: Vec<crate::runner::ToolUse>,
    pub(crate) usage: Option<daruda_acp::UsageView>,
}

impl From<&crate::runner::RunResult> for Reported {
    fn from(result: &crate::runner::RunResult) -> Self {
        Self {
            waited: result.waiting.clone(),
            corrected: result.corrected,
            tools: result.tools.clone(),
            usage: result.usage.clone(),
        }
    }
}

/// The blast radius of one failed attempt. The two travel together because
/// they are computed together — the nodes are the invalidation set and the
/// paths are where that same set's evidence went.
#[derive(Debug, Clone, Default)]
pub struct Invalidation {
    /// The set the failure invalidated, gate included. Design §10 calls this
    /// the rerun set and asks `run.md` to name it.
    pub nodes: Vec<NodeId>,
    pub archived: Vec<PathBuf>,
}

/// How one attempt ended. Three states, not `Option<NodeFailure>`: a cancel
/// is neither a pass nor a failure, and collapsing it into either makes the
/// record lie about why the run stopped.
#[derive(Debug, Clone)]
pub enum AttemptOutcome {
    Passed,
    Failed(NodeFailure),
    Canceled,
    /// An attempt an earlier process made, read back from the journal on
    /// resume. Only the rendered reason survives, and that is enough: a
    /// resumed run reports a settled attempt, it never re-decides its
    /// policy. A variant here rather than one on [`NodeFailure`] because
    /// that enum is the scheduler's control flow, and a value that can only
    /// come from a file has no business in it.
    Reported(String),
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
    if let Some(profile) = &report.provenance.profile {
        out.push_str(&format!("- **Profile** — `{profile}`\n"));
    }
    if !report.provenance.pinned.is_empty() {
        let names: Vec<&str> = report
            .provenance
            .pinned
            .iter()
            .map(|n| n.as_str())
            .collect();
        out.push_str(&format!(
            "- **Reused** — {} did not run; their output was pinned from an earlier run\n",
            names.join(", ")
        ));
    }
    if let Some(until) = &report.provenance.until {
        out.push_str(&format!(
            "- **Stopped at** — `{until}`, as the run asked; nothing downstream ran\n"
        ));
    }
    if report.provenance.carried_over > 0 {
        out.push_str(&format!(
            "- **Continued** — picked up with {} node(s) already done\n",
            report.provenance.carried_over
        ));
    }
    out.push_str(&format!(
        "- **Cost limit** — {}\n",
        cost_standing(report.cost.as_ref())
    ));
    for warning in report.warnings() {
        out.push_str(&format!("- **Warning** — {warning}\n"));
    }

    out.push_str("\n## Attempts\n\n");
    if report.nodes.is_empty() {
        // A node can fail before any session is paid for — a refused output
        // path — and "nothing ran" then contradicts the failure line above it.
        out.push_str(&match &report.outcome {
            RunOutcome::Failed { node, .. } => {
                format!("`{node}` failed before a session opened, so no attempt was recorded.\n")
            }
            _ => "Nothing ran.\n".to_string(),
        });
        return out;
    }
    out.push_str(&format!(
        "**Budget units:** {} · **Nodes:** {}\n",
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

/// What the person said, when it is worth saying. A refusal is always
/// worth it — a node that then wrote nothing did not merely fail to write.
fn said(waiting: &crate::runner::Waiting) -> String {
    use crate::runner::AskAnswer as A;
    let count = |want: A| waiting.answers.iter().filter(|a| **a == want).count();
    let (refused, unanswered) = (count(A::Refused), count(A::Unanswered));
    match (refused, unanswered) {
        (0, 0) => String::new(),
        (r, 0) => format!(", who refused {r}"),
        (0, u) => format!(", and {u} went unanswered"),
        (r, u) => format!(", who refused {r} and left {u} unanswered"),
    }
}

fn limit_of(limit: &BudgetLimit) -> &'static str {
    match limit {
        BudgetLimit::WallClock => "the run's wall-clock limit",
        BudgetLimit::NodeRuns => "the run's budget-unit limit",
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
                 ceiling — the budget-unit cap and the per-node timeouts were the only limits in \
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

/// `2026-08-12T01:18:05Z`. UTC on purpose: this file is read wherever it
/// is copied to, and a local time with no offset is a time nobody can
/// place. The panel renders the same instants in local time, where the
/// reader's clock is known.
pub(crate) fn stamp(at: std::time::SystemTime) -> String {
    humantime::format_rfc3339_seconds(at).to_string()
}

/// ` (took 3s)`, or nothing at all under a second — most command nodes
/// settle in milliseconds and a parenthesis saying so on every line is
/// noise over the attempts where duration is the whole story.
fn took(duration: Duration) -> String {
    if duration.as_secs() == 0 {
        return String::new();
    }
    format!(" (took {}s)", duration.as_secs())
}

/// One attempt and everything it left behind. `attempt` repeats across repair
/// generations, so `evidence_seq` travels with it — that pair is what tells
/// two runs of the same node apart.
///
/// Evidence is named relative to `run_dir`, which is where this file itself
/// lives: an absolute path here is one long prefix repeated on every line.
/// `read x3, execute (1 failed), edit` — a count per name, with failures and
/// calls the turn never settled called out, since those are the ones a reader
/// is looking for.
fn tools_said(tools: &[crate::runner::ToolUse]) -> String {
    use crate::runner::ToolOutcome;
    let mut order: Vec<&str> = Vec::new();
    let mut counts: std::collections::HashMap<&str, (usize, usize, usize)> =
        std::collections::HashMap::new();
    for tool in tools {
        let entry = counts.entry(tool.name.as_str()).or_insert_with(|| {
            order.push(tool.name.as_str());
            (0, 0, 0)
        });
        entry.0 += 1;
        match tool.outcome {
            ToolOutcome::Failed => entry.1 += 1,
            ToolOutcome::Unsettled => entry.2 += 1,
            ToolOutcome::Ok => {}
        }
    }
    order
        .into_iter()
        .map(|name| {
            let (total, failed, unsettled) = counts[name];
            let mut said = if total > 1 {
                format!("{name} x{total}")
            } else {
                name.to_string()
            };
            let mut notes = Vec::new();
            if failed > 0 {
                notes.push(format!("{failed} failed"));
            }
            if unsettled > 0 {
                notes.push(format!("{unsettled} unsettled"));
            }
            if !notes.is_empty() {
                said.push_str(&format!(" ({})", notes.join(", ")));
            }
            said
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// `cost 0.1200 USD, context 45%` — what a session had used when it ended.
///
/// Both halves are optional and the line is skipped when neither is there: a
/// command node opens no session, and an adapter that reports nothing leaves
/// the figure genuinely unknown rather than zero.
///
/// The context share is the one that predicts a *failure*: an attempt that
/// ended at 98% is the one to split before it runs out, and the cost alone
/// does not say so.
fn usage_said(usage: Option<&daruda_acp::UsageView>) -> Option<String> {
    let usage = usage?;
    let mut parts = Vec::new();
    if let Some(cost) = usage.cost.as_ref() {
        parts.push(format!("cost {:.4} {}", cost.amount, cost.currency));
    }
    if let Some(share) = usage.used.saturating_mul(100).checked_div(usage.size) {
        parts.push(format!("context {}%", share.min(100)));
    }
    (!parts.is_empty()).then(|| parts.join(", "))
}

fn push_attempt_lines(out: &mut String, attempt: &AttemptRecord, run_dir: &Path) {
    out.push_str(&format!(
        "- attempt {} (evidence {}) — {} at {}{}\n",
        attempt.attempt,
        attempt.evidence_seq,
        ended_as(&attempt.outcome),
        stamp(attempt.at),
        took(attempt.took),
    ));
    // Said before the evidence, because it changes how every duration on
    // this attempt reads: the clocks stop while a person is being waited
    // on, so an attempt that took forty minutes of wall time may have done
    // three minutes of work. Omitted when nobody was asked, which is most
    // attempts.
    if !attempt.waited.total.is_zero() || !attempt.waited.answers.is_empty() {
        out.push_str(&format!(
            "  - waited {}s for a person{}\n",
            attempt.waited.total.as_secs(),
            said(&attempt.waited)
        ));
    }
    // What it reached for, so "did it even try to write the file" is answered
    // here rather than by opening the transcript. Grouped by name because the
    // count is the question, not the order.
    if !attempt.tools.is_empty() {
        out.push_str(&format!("  - used {}\n", tools_said(&attempt.tools)));
    }
    // The budget-unit total includes this second turn, but the attempt count
    // does not — and an attempt that passed on its correction reads exactly
    // like one that got it right first time without this line.
    // What this node cost, beside what it did. The run's total is a ceiling's
    // arithmetic and answers "was it too much"; this answers "which one", and
    // a reader deciding what to change next needs the second.
    if let Some(line) = usage_said(attempt.usage.as_ref()) {
        out.push_str(&format!("  - {line}\n"));
    }
    if attempt.corrected {
        out.push_str("  - spent a correction turn: the first turn left no usable output\n");
    }
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
        AttemptOutcome::Reported(reason) => reason.clone(),
    }
}

#[cfg(test)]
mod tests;
