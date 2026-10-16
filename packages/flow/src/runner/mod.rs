//! What it means to run one node, without saying how. The scheduler drives
//! this trait; the real ACP and process runners implement it later, and the
//! tests implement it with a script.

pub mod acp;
pub mod process;

pub use acp::{AcpRunner, OverageStop};
pub use process::ProcessRunner;

use crate::NodeId;
use crate::model::AgentSpec;
use daruda_acp::UsageView;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Why one attempt at a node did not succeed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeFailure {
    Timeout {
        elapsed: Duration,
    },
    /// The turn ended normally but the node wrote no usable output.
    NoOutput {
        expected: PathBuf,
    },
    /// Something other than a plain file is standing where the output
    /// belongs — a link, most often, whose target's size would otherwise
    /// pass for work this node did.
    OutputNotAFile {
        expected: PathBuf,
    },
    /// The output path does not stay inside the run directory: a link on
    /// the way to it, or on it, points somewhere else.
    OutputEscapes {
        expected: PathBuf,
        resolved: PathBuf,
    },
    /// The output is a plain file this node wrote, and its contents are not
    /// the shape the node declared.
    ///
    /// One `problem` and a count, not the list: this reaches `run.md`, the
    /// run log's record and a repair's `{{failure}}`, all of which want a
    /// line. The whole list goes to the correction prompt, which is the only
    /// reader that can act on it.
    OutputSchema {
        expected: PathBuf,
        problem: String,
        more: usize,
    },
    /// `stop_reason` was `MaxTokens` — the output may be truncated.
    ContextExhausted,
    /// `stop_reason` was `MaxTurnRequests`.
    TurnLimit,
    /// `stop_reason` was `Refusal`. Re-asking the same thing gets refused
    /// again, so this never retries.
    Refused,
    TurnFailed(String),
    SessionError(String),
    /// A mode, model or effort the adapter did not advertise, or requested
    /// and not applied.
    UnsupportedSetting {
        field: &'static str,
        value: String,
        available: Vec<String>,
    },
    PermissionDenied {
        tool: String,
    },
    /// A model / effort the adapter advertised and then refused to set.
    /// Distinct from [`NodeFailure::UnsupportedSetting`]: that one is what
    /// the adapter never offered, this one is what it offered and then
    /// declined, which is the adapter's own inconsistency and worth saying
    /// separately in `run.md`.
    SettingRejected {
        config_id: String,
        reason: String,
    },
    Exit {
        code: Option<i32>,
    },
    /// The process running the attempt died, and a resume found its
    /// attempts already spent: nothing reached a verdict.
    Interrupted,
}

impl NodeFailure {
    /// The failures a retry cannot change, because nothing a second
    /// attempt does is different.
    ///
    /// A refusal is the obvious one: re-asking the same thing gets refused
    /// again. `UnsupportedSetting` is the quieter one — it compares the
    /// node's pinned `model` / `effort` / `mode` against what the adapter
    /// advertises, and neither side moves within a run. It also fails
    /// *before* the prompt is sent, so every further attempt reaches the
    /// same point and stops there: a node with `max_attempts: 5` opens five
    /// sessions and sits through four waits to be told the same thing.
    ///
    /// `PermissionDenied` deliberately stays retryable. The policy is fixed
    /// for the run, but whether the agent reaches for that tool at all is
    /// not — a second attempt may simply not ask.
    pub fn forbids_retry(&self) -> bool {
        matches!(
            self,
            NodeFailure::Refused
                | NodeFailure::UnsupportedSetting { .. }
                | NodeFailure::SettingRejected { .. }
        )
    }
}

impl std::fmt::Display for NodeFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NodeFailure::Timeout { elapsed } => {
                write!(f, "timed out after {} seconds", elapsed.as_secs())
            }
            NodeFailure::NoOutput { expected } => {
                write!(f, "no output written to {}", expected.display())
            }
            NodeFailure::OutputNotAFile { expected } => {
                write!(f, "{} is not a plain file", expected.display())
            }
            NodeFailure::OutputEscapes { expected, resolved } => write!(
                f,
                "the output {} resolves through a link, to {}",
                expected.display(),
                resolved.display()
            ),
            NodeFailure::OutputSchema {
                expected,
                problem,
                more: 0,
            } => write!(
                f,
                "{} does not match its output schema: {problem}",
                expected.display()
            ),
            NodeFailure::OutputSchema {
                expected,
                problem,
                more,
            } => write!(
                f,
                "{} does not match its output schema: {problem} (and {more} more)",
                expected.display()
            ),
            NodeFailure::ContextExhausted => {
                write!(f, "the context window ran out; the output may be truncated")
            }
            NodeFailure::TurnLimit => write!(f, "the turn-request limit was reached"),
            NodeFailure::Refused => write!(f, "the agent refused the request"),
            NodeFailure::TurnFailed(msg) => write!(f, "the turn failed: {msg}"),
            NodeFailure::SessionError(msg) => write!(f, "the session failed: {msg}"),
            NodeFailure::UnsupportedSetting {
                field,
                value,
                available,
            } => write!(
                f,
                "the agent does not offer {field} `{value}` (it offers {available:?})"
            ),
            NodeFailure::PermissionDenied { tool } => {
                write!(f, "permission was refused for `{tool}`")
            }
            NodeFailure::SettingRejected { config_id, reason } => write!(
                f,
                "the agent offered `{config_id}` and then refused to set it: {reason}"
            ),
            NodeFailure::Exit { code: Some(c) } => write!(f, "exited with status {c}"),
            NodeFailure::Exit { code: None } => write!(f, "was killed by a signal"),
            NodeFailure::Interrupted => {
                write!(
                    f,
                    "was interrupted when the app stopped, and has no attempts left"
                )
            }
        }
    }
}

/// Whether what a node wrote is what it owed.
///
/// A trait so the rule has one implementation and two moments to be asked
/// at: the scheduler asks after the runner has returned, and a correction
/// turn has to ask while the session it would speak into is still open.
pub trait OutputContract {
    fn check(&self) -> Result<(), ContractBreach>;
}

/// Every reason one output did not meet its contract.
///
/// Separate from [`NodeFailure`] because the two are read by different
/// readers. A `NodeFailure`'s `Display` is deliberately one terse line —
/// it is rendered into `run.md`, the run log, and a repair
/// prompt's `{{failure}}` — while an agent being asked to correct what it
/// wrote needs every reason it was wrong, not the first one.
///
/// `first` plus `rest` rather than a `Vec`: an `Err` carrying no reason is
/// then unrepresentable, and there is always a line to lead with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractBreach {
    pub kind: BreachKind,
    /// Always at least one line.
    pub first: String,
    pub rest: Vec<String>,
}

/// Which way the contract was not met. Each variant carries what its
/// failure line names, so a breach cannot be built without the paths the
/// [`NodeFailure`] it becomes needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BreachKind {
    /// Nothing on the path, or a file with nothing in it — the same thing
    /// to whoever reads it next.
    Missing {
        expected: PathBuf,
    },
    NotAFile {
        expected: PathBuf,
    },
    Escapes {
        expected: PathBuf,
        resolved: PathBuf,
    },
    /// The file is this node's and its contents are not the declared shape.
    /// The problems themselves are the breach's lines — this carries only what
    /// the failure line names.
    Schema {
        expected: PathBuf,
    },
    /// The output is this node's, well-formed, and does not say the work is
    /// over — the node declared `continue_until` and the field it named says
    /// otherwise.
    ///
    /// A breach rather than a verdict of its own because everything that
    /// follows one is what this needs too: another turn on the open session,
    /// a line in the record, a failure if the turns run out.
    Unfinished {
        expected: PathBuf,
    },
}

/// The one place a breach becomes the failure a run reports, so the two
/// vocabularies cannot drift into disagreeing about the same file.
impl From<ContractBreach> for NodeFailure {
    fn from(breach: ContractBreach) -> Self {
        let ContractBreach { kind, first, rest } = breach;
        match kind {
            BreachKind::Missing { expected } => NodeFailure::NoOutput { expected },
            BreachKind::NotAFile { expected } => NodeFailure::OutputNotAFile { expected },
            BreachKind::Escapes { expected, resolved } => {
                NodeFailure::OutputEscapes { expected, resolved }
            }
            BreachKind::Schema { expected } => NodeFailure::OutputSchema {
                expected,
                problem: first,
                more: rest.len(),
            },
            // Reported as the schema failure it is a cousin of: the file is
            // there and its contents are not what the node was asked for.
            // Distinguishing the two in the *failure* would split a vocabulary
            // whose only reader is a person, and `first` already says which.
            BreachKind::Unfinished { expected } => NodeFailure::OutputSchema {
                expected,
                problem: first,
                more: rest.len(),
            },
        }
    }
}

/// The cancel token carries no waker, so watching it means polling. Short
/// against anything a node does, and an idle timer costs nothing.
const CANCEL_POLL: Duration = Duration::from_millis(20);

/// What a run's stop reports. There is no cancel variant on [`NodeFailure`]
/// and there should not be: the scheduler discards this outcome the moment it
/// sees the token set, so this text only ever reaches a runner test.
pub(crate) const CANCELED: &str = "the run was canceled";

/// Both runners race a wait against work. This crate is GPUI-free, so there
/// is no `BackgroundExecutor` to time on.
pub(crate) async fn sleep(duration: Duration) {
    // ALLOW: no BackgroundExecutor exists here, and the durations are
    // resolved config the suite sets.
    #[allow(clippy::disallowed_methods)]
    smol::Timer::after(duration).await;
}

/// Resolves once the run's stop switch is set.
pub(crate) async fn canceled(cancel: &CancelToken) {
    while !cancel.is_canceled() {
        sleep(CANCEL_POLL).await;
    }
}

/// A run's stop switch. Cloneable and cheap so the host can hold one half
/// while the run holds the other; the engine spawns nothing, so this is
/// the whole of what the engine offers for stopping a run.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl CancelToken {
    pub fn cancel(&self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn is_canceled(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::SeqCst)
    }
}

/// Who is asking. A repair's fix session is not a node, so it asks as the
/// gate whose repair it is.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AskOwner {
    Node {
        node: NodeId,
        generation: u32,
        attempt: u32,
    },
    Fix {
        gate: NodeId,
    },
}

impl AskOwner {
    /// The node a host shows the question against: the asking node, or the
    /// gate whose fix is asking.
    pub fn node(&self) -> &NodeId {
        match self {
            AskOwner::Node { node, .. } => node,
            AskOwner::Fix { gate } => gate,
        }
    }
}

/// One permission request handed to a person, with the way back.
///
/// The reply channel travels inside the request rather than being matched
/// up by id afterwards, so answering a question that is no longer live fails
/// instead of landing elsewhere. Bounded to one: the first answer wins.
#[derive(Debug, Clone)]
pub struct PendingAsk {
    pub owner: AskOwner,
    /// The id the run log gave this question. Unique within the run, across
    /// resumes too.
    pub ask_id: u64,
    pub request: AskRequest,
    pub reply: smol::channel::Sender<daruda_acp::PermissionDecision>,
}

/// The engine closed the question before this answer arrived (F13).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the run is no longer waiting for this answer")]
pub struct AskClosed;

impl PendingAsk {
    /// Answer the question. `Err` when the engine has already abandoned it.
    pub fn answer(&self, decision: daruda_acp::PermissionDecision) -> Result<(), AskClosed> {
        self.reply.try_send(decision).map_err(|_| AskClosed)
    }
}

/// What a person said to one question. Coarser than the decision that went
/// on the wire: which option they picked is the transcript's to keep, and
/// what a *record* has to answer is whether the work was allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AskAnswer {
    Allowed,
    Refused,
    /// Nobody answered — the host went away, or the run was stopped while
    /// the question was still up.
    Unanswered,
}

/// What one runner call spent waiting for a person, and what they said.
///
/// One value because the two are only meaningful together: a record that
/// says an attempt waited forty minutes without saying what came of it
/// leaves the reader to open the transcript, and the answer is often the
/// reason the attempt ended the way it did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Waiting {
    pub total: Duration,
    pub answers: Vec<AskAnswer>,
}

impl Waiting {
    /// Whether anybody refused. The one thing a failure line needs from
    /// this: a node that wrote nothing after a refusal did not simply fail
    /// to write.
    pub fn any_refused(&self) -> bool {
        self.answers.contains(&AskAnswer::Refused)
    }
}

/// The question itself, in host terms. Built from the protocol request by
/// the runner, through `daruda_acp`'s own conversion, so no protocol type
/// crosses this boundary.
#[derive(Debug, Clone, PartialEq)]
pub struct AskRequest {
    pub tool: String,
    /// What the tool is about to do, when the adapter said. A tool name
    /// alone ("Bash") is not a question a person can answer.
    pub detail: Option<String>,
    pub options: Vec<daruda_acp::PermissionChoice>,
}

/// The host's answering port, narrowed to the one thing a runner may do
/// with it. Every question is committed to the run log before the host sees
/// it, and closed there however the wait ends.
pub struct AskChannel {
    tx: smol::channel::Sender<PendingAsk>,
    log: crate::run_log::store::FlowRunStore,
}

impl AskChannel {
    pub(crate) fn new(
        tx: smol::channel::Sender<PendingAsk>,
        log: crate::run_log::store::FlowRunStore,
    ) -> Self {
        Self { tx, log }
    }

    /// Put the question to the host and wait for the answer. A question the
    /// log could not record, or a host that is gone, is no answer.
    pub(crate) async fn ask(
        &self,
        owner: &AskOwner,
        request: AskRequest,
    ) -> daruda_acp::PermissionDecision {
        use crate::run_log::machine::{AbandonReason, FlowRunEvent};
        let cancelled = daruda_acp::PermissionDecision::Cancelled;
        let raised = self
            .log
            .commit(FlowRunEvent::ask_raised(owner.clone(), (&request).into()))
            .await;
        let Ok(committed) = raised else {
            return cancelled;
        };
        let FlowRunEvent::AskRaised { id, .. } = committed.event else {
            return cancelled;
        };
        let mut open = OpenQuestion {
            log: &self.log,
            id: Some(id),
        };
        let (reply, answers) = smol::channel::bounded(1);
        let sent = self.tx.try_send(PendingAsk {
            owner: owner.clone(),
            ask_id: id,
            request,
            reply,
        });
        if sent.is_err() {
            open.close(AbandonReason::HostGone).await;
            return cancelled;
        }
        match answers.recv().await {
            Ok(decision) => {
                open.answered(&decision).await;
                decision
            }
            Err(_) => {
                open.close(AbandonReason::HostGone).await;
                cancelled
            }
        }
    }
}

/// A question the log holds open. Dropped unanswered — the asking call was
/// stopped or timed out — it closes itself, so no open question outlives
/// the wait for it.
struct OpenQuestion<'a> {
    log: &'a crate::run_log::store::FlowRunStore,
    id: Option<u64>,
}

impl OpenQuestion<'_> {
    async fn answered(&mut self, decision: &daruda_acp::PermissionDecision) {
        if let Some(id) = self.id.take() {
            let answer = decision.into();
            let event = crate::run_log::machine::FlowRunEvent::AskAnswered { id, answer };
            // SILENT-OK: a failed commit closes the log; the attempt's own
            // settle reports it.
            let _ = self.log.commit(event).await;
        }
    }

    async fn close(&mut self, reason: crate::run_log::machine::AbandonReason) {
        if let Some(id) = self.id.take() {
            let event = crate::run_log::machine::FlowRunEvent::AskAbandoned { id, reason };
            // SILENT-OK: as in `answered`.
            let _ = self.log.commit(event).await;
        }
    }
}

impl Drop for OpenQuestion<'_> {
    fn drop(&mut self) {
        if let Some(id) = self.id.take() {
            self.log
                .commit_detached(crate::run_log::machine::FlowRunEvent::AskAbandoned {
                    id,
                    reason: crate::run_log::machine::AbandonReason::Interrupted,
                });
        }
    }
}

/// One asker's way to the host: the run's channel and who is asking.
pub struct AskPort<'a> {
    channel: &'a AskChannel,
    owner: AskOwner,
}

impl<'a> AskPort<'a> {
    pub(crate) fn new(channel: &'a AskChannel, owner: AskOwner) -> Self {
        Self { channel, owner }
    }

    pub(crate) async fn ask(&self, request: AskRequest) -> daruda_acp::PermissionDecision {
        self.channel.ask(&self.owner, request).await
    }
}

/// What this node does when the agent asks for permission.
///
/// `Ask` carries the port rather than sitting beside an `Option` of it:
/// a policy of "ask a person" with nobody to ask is not a state worth
/// representing, and [`crate::request::validate_request`] is the one place
/// that has to rule it out.
pub enum Permission<'a> {
    Deny,
    AllowOnce,
    Ask(AskPort<'a>),
}

impl Permission<'_> {
    /// Whether a request arriving at all means the node was launched in
    /// the wrong mode. Only `Deny` says that — a person declining one tool
    /// is a judgement, not a misconfiguration.
    pub(crate) fn denies(&self) -> bool {
        matches!(self, Permission::Deny)
    }
}

/// Where one attempt runs and where it may write.
pub struct RunContext<'a> {
    pub node_id: &'a NodeId,
    /// 1-based policy attempt for this node. Not globally unique: nested
    /// repair generations can reset a child policy owner's counter.
    pub attempt: u32,
    /// When this attempt began, for the record's sake. Wall clock rather
    /// than `Instant` because what it becomes is a timestamp somebody lines
    /// up against other logs; a clock that jumps backwards costs the
    /// duration on one line and nothing else.
    pub started_at: std::time::SystemTime,
    /// Where this node runs — its own directory when it names one, and the
    /// run's otherwise.
    pub cwd: &'a Path,
    /// `<cwd>/.daruda/flow-runs/<run-id>/` — outputs land here.
    pub run_dir: &'a Path,
    /// `<run_dir>/logs/` — command output, transcripts, archived outputs.
    pub log_dir: &'a Path,
    /// The absolute path an agent node must write, or `None` for a command
    /// node. The runner does not derive this — the scheduler resolves it
    /// from the node's `output` against `run_dir`.
    pub output: Option<&'a Path>,
    /// Monotonic evidence id for this runner call. Attempts can restart
    /// inside a parent repair generation; evidence ids never do, so logs
    /// and archived outputs do not collide.
    pub evidence_seq: u32,
    /// What this attempt's output has to be for the node to pass, when the
    /// node owes one. `None` exactly when `output` is: a command node and a
    /// repair's fix session owe no file, and a contract on either would
    /// fail them for not writing something never asked for.
    pub contract: Option<&'a dyn OutputContract>,
    /// How many prompts this attempt may send, the first included.
    ///
    /// Rides here rather than being read off the node, for the same reason
    /// `contract` does: the runner is handed what it may do, not the flow to
    /// work it out from. A fix session and a command node get 1 — there is
    /// nothing for a second turn to change.
    pub max_turns: u32,
    pub timeout: Duration,
    pub permission: Permission<'a>,
    /// The run's stop switch, observable mid-turn: a real runner watches it
    /// alongside the event stream and cancels the session it is holding.
    /// The scheduler only sees it between calls, which is too late to stop
    /// a turn that is already running.
    pub cancel: &'a CancelToken,
    /// Permission to spend one more budget unit on a correction turn inside
    /// this call — **a reservation, not a question**: `true` means the run's
    /// budget has already been charged for it.
    ///
    /// The scheduler charges the call's first turn before entering runner
    /// code. An additional turn has to use this door while the session is
    /// still open so the same ceiling covers both.
    pub reserve_extra_turn: &'a dyn Fn() -> bool,
}

/// One tool call a node's turn made, as the protocol reported it.
///
/// The name is the protocol's coarse `kind` (`read`, `execute`, …) and not the
/// agent's own tool name: that is resolved through the session's adapter, which
/// a runner does not hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolUse {
    pub name: String,
    pub outcome: ToolOutcome,
}

/// How a tool call ended. `Unsettled` is its own answer, not a failure: a turn
/// that ended while a call was still running says something a reader needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolOutcome {
    Ok,
    Failed,
    Unsettled,
}

/// What one attempt produced. Carries more than a verdict because the
/// scheduler has to archive evidence and carry usage forward.
#[derive(Debug, Clone, PartialEq)]
pub struct RunResult {
    pub outcome: Result<(), NodeFailure>,
    /// Files this attempt wrote under `log_dir` — the scheduler archives
    /// these alongside the node's output and feeds their paths to a repair.
    pub artifacts: Vec<PathBuf>,
    /// Usage at session end. `None` for a command node, and for an agent
    /// whose adapter reports none.
    pub usage: Option<UsageView>,
    /// What this call spent waiting for a person, and what they said.
    ///
    /// The run's own deadline is an absolute `Instant`, so the node clock
    /// stopping does nothing for it — `total` is what the scheduler
    /// subtracts so a long approval does not end the run the moment it is
    /// granted. It rides on the result because every scheduler call passes
    /// through `Run::accounted_call`, which keeps one accounting path.
    pub waiting: Waiting,
    /// Whether this call used a second turn to correct its output.
    ///
    /// Recorded because it consumes another budget unit: without it, a node
    /// that only passed on its correction is byte-identical in the record to
    /// How many prompts this call sent, the first included.
    ///
    /// Recorded because every turn past the first spends a budget unit:
    /// without it, a node that only got there on its third turn is
    /// byte-identical in the record to one that got it right first time.
    pub turns: u32,
    /// What tools the turn used, in the order it reached for them. A
    /// diagnostic aid, not a decision input — nothing in the engine reads it.
    pub tools: Vec<ToolUse>,
    /// Whether this call's own report is what stopped the run — an overage
    /// stop. A turn that still ended cleanly is then judged and kept.
    pub stopped_the_run: bool,
}

/// One attempt at one node. Async because a real runner has to race the
/// event stream against a timer and cancel on expiry — that responsibility
/// belongs to the implementor, not to the scheduler.
pub trait NodeRunner {
    /// Optional setup owned by the concrete runner, reused by all of its nodes.
    fn prepare_agent(&self, _id: &str, _cancel: &CancelToken) -> Result<Vec<String>, String> {
        Ok(Vec::new())
    }

    fn run_agent<'a>(
        &'a self,
        ctx: &'a RunContext<'a>,
        agent: &'a AgentSpec,
        prompt: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = RunResult> + 'a>>;

    fn run_command<'a>(
        &'a self,
        ctx: &'a RunContext<'a>,
        run: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = RunResult> + 'a>>;
}

/// The pair a real run needs: an agent node opens a session, a command node
/// starts a shell. Each of the two refuses the other kind by name, so a host
/// that passed one alone would fail half its nodes.
///
/// The dispatch lives here rather than in every host because the scheduler
/// already picks the method by node kind — this only says which runner owns
/// which method, which is the crate's own knowledge.
pub struct Runners {
    pub agent: AcpRunner,
    pub command: ProcessRunner,
}

impl NodeRunner for Runners {
    fn prepare_agent(&self, id: &str, cancel: &CancelToken) -> Result<Vec<String>, String> {
        self.agent.prepare_agent(id, cancel)
    }

    fn run_agent<'a>(
        &'a self,
        ctx: &'a RunContext<'a>,
        agent: &'a AgentSpec,
        prompt: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = RunResult> + 'a>> {
        self.agent.run_agent(ctx, agent, prompt)
    }

    fn run_command<'a>(
        &'a self,
        ctx: &'a RunContext<'a>,
        run: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = RunResult> + 'a>> {
        self.command.run_command(ctx, run)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// The run's own line has to name the same file the breach was about,
    /// or the two vocabularies are describing different outputs.
    #[test]
    fn a_breach_becomes_the_failure_that_names_the_same_file() {
        let expected = PathBuf::from("/run/design.md");
        let breach = ContractBreach {
            kind: BreachKind::Missing {
                expected: expected.clone(),
            },
            first: "nothing usable is at /run/design.md".to_string(),
            rest: vec!["it needs a `## Risks` section".to_string()],
        };

        assert_eq!(
            NodeFailure::from(breach),
            NodeFailure::NoOutput { expected }
        );
    }

    #[test]
    fn failure_display_is_prompt_facing_and_names_the_cause() {
        let timeout = NodeFailure::Timeout {
            elapsed: Duration::from_secs(600),
        };
        let text = timeout.to_string();
        assert!(
            text.contains("600"),
            "the elapsed time belongs in the text: {text}"
        );

        let refused = NodeFailure::Refused;
        assert!(!refused.to_string().is_empty());

        let exit = NodeFailure::Exit { code: Some(101) };
        assert!(exit.to_string().contains("101"));
    }

    /// The line is "would a second attempt reach a different point", not
    /// "was this the agent's fault". A pinned setting the adapter does not
    /// offer fails before the prompt every time; a denied tool may simply
    /// not be reached for on the next attempt.
    #[test]
    fn a_retry_is_refused_only_where_it_could_not_change_anything() {
        assert!(NodeFailure::Refused.forbids_retry());
        assert!(
            NodeFailure::UnsupportedSetting {
                field: "model",
                value: "opus".to_string(),
                available: vec!["sonnet".to_string()],
            }
            .forbids_retry()
        );

        assert!(
            !NodeFailure::PermissionDenied {
                tool: "write".to_string()
            }
            .forbids_retry()
        );
        assert!(
            !NodeFailure::Timeout {
                elapsed: Duration::ZERO
            }
            .forbids_retry()
        );
        assert!(!NodeFailure::Exit { code: Some(1) }.forbids_retry());
    }
}
