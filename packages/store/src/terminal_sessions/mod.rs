//! Task-independent execution history. One journal belongs to one terminal.
//!
//! Callers serialize writers with that terminal's stable hook lock. Conversation
//! identity is provider + configuration home + session id; execution identity
//! is terminal id + the journal-assigned attempt, including repeated resumes.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::accounts::{AccountId, AccountRecipeId};
use crate::project::{LaneRef, WorkspaceUuid};

mod journal;
pub use journal::{HistoryError, SessionJournal};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalContext {
    pub workspace: WorkspaceUuid,
    #[serde(default)]
    pub lane: Option<LaneRef>,
    #[serde(default)]
    pub account: Option<AccountId>,
    #[serde(default)]
    pub cwd: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conversation {
    pub provider: AccountRecipeId,
    #[serde(default)]
    pub config_home: Option<PathBuf>,
    pub session_id: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndReason {
    SessionEnded,
    ProcessExited,
    SessionReplaced,
    PaneClosed,
    TerminalExited,
    /// The old app's observation ended; this does not prove process exit.
    ObservationLost,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalEndReason {
    PaneClosed,
    TerminalExited,
    ObservationLost,
}

impl From<TerminalEndReason> for EndReason {
    fn from(reason: TerminalEndReason) -> Self {
        match reason {
            TerminalEndReason::PaneClosed => Self::PaneClosed,
            TerminalEndReason::TerminalExited => Self::TerminalExited,
            TerminalEndReason::ObservationLost => Self::ObservationLost,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExecutionState {
    Observed,
    Ended {
        reason: EndReason,
        at: DateTime<Utc>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionExecution {
    pub attempt: u64,
    pub conversation: Conversation,
    pub first_observed_at: DateTime<Utc>,
    pub last_observed_at: DateTime<Utc>,
    pub cwd: PathBuf,
    #[serde(default)]
    pub transcript_path: Option<PathBuf>,
    #[serde(default)]
    pub agent_pid: Option<u32>,
    pub state: ExecutionState,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionHistory {
    #[serde(default)]
    pub context: Option<TerminalContext>,
    pub executions: Vec<SessionExecution>,
    terminal: TerminalState,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum TerminalState {
    #[default]
    Open,
    Closed {
        reason: TerminalEndReason,
        at: DateTime<Utc>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationKind {
    Update,
    SessionStart,
    SessionEnd,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Observation {
    pub conversation: Conversation,
    pub at: DateTime<Utc>,
    pub cwd: PathBuf,
    #[serde(default)]
    pub transcript_path: Option<PathBuf>,
    #[serde(default)]
    pub agent_pid: Option<u32>,
    pub kind: ObservationKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionEvent {
    Context {
        context: TerminalContext,
    },
    Observed {
        observation: Observation,
    },
    ProcessExited {
        session_id: String,
        pid: u32,
        at: DateTime<Utc>,
    },
    TerminalClosed {
        reason: TerminalEndReason,
        at: DateTime<Utc>,
    },
}

pub struct SessionMachine;

impl daruda_runlog::Machine for SessionMachine {
    type State = SessionHistory;
    type Event = SessionEvent;
    type Rejected = std::convert::Infallible;
    const SCHEMA: u32 = 1;

    fn check(_: &SessionHistory, _: &SessionEvent) -> Result<(), Self::Rejected> {
        Ok(())
    }

    fn apply(state: &mut SessionHistory, event: &SessionEvent) {
        match event {
            SessionEvent::Context { context } => state.context = Some(context.clone()),
            SessionEvent::Observed { observation } => state.observe(observation),
            SessionEvent::ProcessExited {
                session_id,
                pid,
                at,
            } => {
                if let Some(run) = state.executions.last_mut()
                    && run.conversation.session_id == *session_id
                    && run.agent_pid == Some(*pid)
                    && run.last_observed_at <= *at
                {
                    run.end(EndReason::ProcessExited, *at);
                }
            }
            SessionEvent::TerminalClosed { reason, at } => {
                if matches!(state.terminal, TerminalState::Open) {
                    state.terminal = TerminalState::Closed {
                        reason: *reason,
                        at: *at,
                    };
                    if let Some(run) = state.executions.last_mut() {
                        run.end((*reason).into(), *at);
                    }
                }
            }
        }
    }
}

impl SessionHistory {
    pub fn terminal_is_closed(&self) -> bool {
        matches!(self.terminal, TerminalState::Closed { .. })
    }

    fn observe(&mut self, observation: &Observation) {
        if let Some(last) = self.executions.last_mut() {
            if observation.at < last.last_observed_at {
                return;
            }
            if last.conversation == observation.conversation {
                if observation.kind == ObservationKind::SessionEnd {
                    last.observe_metadata(observation);
                    last.end(EndReason::SessionEnded, observation.at);
                    return;
                }
                if matches!(last.state, ExecutionState::Observed) {
                    last.observe_metadata(observation);
                    return;
                }
                if !matches!(self.terminal, TerminalState::Open)
                    || observation.kind != ObservationKind::SessionStart
                {
                    return;
                }
            }
        }
        if let Some(previous) = self.executions.last_mut() {
            previous.end(EndReason::SessionReplaced, observation.at);
        }
        self.executions.push(SessionExecution {
            attempt: self.executions.last().map_or(1, |last| last.attempt + 1),
            conversation: observation.conversation.clone(),
            first_observed_at: observation.at,
            last_observed_at: observation.at,
            cwd: observation.cwd.clone(),
            transcript_path: observation.transcript_path.clone(),
            agent_pid: observation.agent_pid,
            state: if let TerminalState::Closed { reason, at } = self.terminal {
                ExecutionState::Ended {
                    reason: reason.into(),
                    at: at.max(observation.at),
                }
            } else if observation.kind == ObservationKind::SessionEnd {
                ExecutionState::Ended {
                    reason: EndReason::SessionEnded,
                    at: observation.at,
                }
            } else {
                ExecutionState::Observed
            },
        });
    }
}

impl SessionExecution {
    fn observe_metadata(&mut self, observation: &Observation) {
        self.last_observed_at = observation.at;
        self.cwd = observation.cwd.clone();
        if observation.transcript_path.is_some() {
            self.transcript_path = observation.transcript_path.clone();
        }
        if observation.agent_pid.is_some() {
            self.agent_pid = observation.agent_pid;
        }
    }

    fn end(&mut self, reason: EndReason, at: DateTime<Utc>) {
        match self.state {
            ExecutionState::Observed => {
                self.last_observed_at = self.last_observed_at.max(at);
                self.state = ExecutionState::Ended { reason, at };
            }
            ExecutionState::Ended {
                reason: EndReason::ProcessExited,
                at: ended_at,
            } if matches!(reason, EndReason::SessionEnded | EndReason::PaneClosed) => {
                self.state = ExecutionState::Ended {
                    reason,
                    at: ended_at,
                };
            }
            ExecutionState::Ended { .. } => {}
        }
    }
}

#[cfg(test)]
mod tests;
