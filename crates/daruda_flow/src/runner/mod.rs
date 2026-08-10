//! What it means to run one node, without saying how. The scheduler drives
//! this trait; the real ACP and process runners implement it later, and the
//! tests implement it with a script.

pub mod acp;
pub mod process;

pub use acp::AcpRunner;
pub use process::ProcessRunner;

use crate::NodeId;
use crate::model::{AgentSpec, PermissionPolicy};
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

/// Where one attempt runs and where it may write.
pub struct RunContext<'a> {
    pub node_id: &'a NodeId,
    /// 1-based policy attempt for this node. Not globally unique: nested
    /// repair generations can reset a child policy owner's counter.
    pub attempt: u32,
    /// The working directory every node shares.
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
    pub permission: PermissionPolicy,
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
