//! What it means to run one node, without saying how. The scheduler drives
//! this trait; the real ACP and process runners implement it later, and the
//! tests implement it with a script.

pub mod acp;
pub mod process;

pub use acp::AcpRunner;
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

/// One permission request handed to a person, with the way back.
///
/// The reply channel travels inside the request rather than being matched
/// up by id afterwards: there is no correlation table to get wrong, and
/// answering a question that is no longer live is impossible rather than
/// merely unlikely. Bounded to one — the first answer wins, so two
/// surfaces offering the same buttons cannot both send.
#[derive(Debug, Clone)]
pub struct PendingAsk {
    /// The node, or the gate whose repair is asking (`FIX_SESSION_ID` is
    /// not a node, so the gate's name is what makes it renderable).
    pub node: NodeId,
    pub attempt: u32,
    /// Distinguishes this question from the next one in the same run —
    /// what a host compares to tell a stale click from a live one.
    pub ask_id: u64,
    pub request: AskRequest,
    pub reply: smol::channel::Sender<daruda_acp::PermissionDecision>,
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
/// with it. Holding the raw event sender instead would let a runner emit
/// any event at all.
pub struct AskChannel {
    tx: smol::channel::Sender<PendingAsk>,
    next_id: std::cell::Cell<u64>,
}

impl AskChannel {
    pub(crate) fn new(tx: smol::channel::Sender<PendingAsk>) -> Self {
        Self {
            tx,
            next_id: std::cell::Cell::new(1),
        }
    }

    /// Put the question to the host. `None` is a host that has gone away,
    /// which the caller treats as no answer rather than waiting forever.
    pub(crate) fn ask(
        &self,
        node: &NodeId,
        attempt: u32,
        request: AskRequest,
    ) -> Option<smol::channel::Receiver<daruda_acp::PermissionDecision>> {
        let ask_id = self.next_id.get();
        self.next_id.set(ask_id + 1);
        let (reply, rx) = smol::channel::bounded(1);
        self.tx
            .try_send(PendingAsk {
                node: node.clone(),
                attempt,
                ask_id,
                request,
                reply,
            })
            .ok()
            .map(|()| rx)
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
    Ask(&'a AskChannel),
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
    pub timeout: Duration,
    pub permission: Permission<'a>,
    /// The run's stop switch, observable mid-turn: a real runner watches it
    /// alongside the event stream and cancels the session it is holding.
    /// The scheduler only sees it between calls, which is too late to stop
    /// a turn that is already running.
    pub cancel: &'a CancelToken,
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
    /// granted. It rides on the result because every runner call already
    /// passes through `Run::call` → `account`, which keeps one raiser.
    pub waiting: Waiting,
}

/// One attempt at one node. Async because a real runner has to race the
/// event stream against a timer and cancel on expiry — that responsibility
/// belongs to the implementor, not to the scheduler.
pub trait NodeRunner {
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
