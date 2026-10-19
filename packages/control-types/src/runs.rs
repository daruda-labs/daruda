//! Stable execution addresses used by the external control surface.

use std::{fmt, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A concrete execution, or the latest execution of a task.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RunHandle {
    Flow {
        run_id: String,
    },
    Task {
        task_id: String,
        execution_id: String,
    },
    TaskLatest {
        task_id: String,
    },
}

/// A handle has an unknown prefix or an unsafe/empty identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidRunHandle;

impl fmt::Display for InvalidRunHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid run handle")
    }
}

impl std::error::Error for InvalidRunHandle {}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id != "."
        && id != ".."
        && !id
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '/' | '\\' | ':'))
}

impl FromStr for RunHandle {
    type Err = InvalidRunHandle;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if let Some(run_id) = value.strip_prefix("flow:").filter(|id| valid_id(id)) {
            return Ok(Self::Flow {
                run_id: run_id.into(),
            });
        }
        if let Some(task) = value.strip_prefix("task:") {
            return match task.split_once('/') {
                Some((task_id, execution_id)) if valid_id(task_id) && valid_id(execution_id) => {
                    Ok(Self::Task {
                        task_id: task_id.into(),
                        execution_id: execution_id.into(),
                    })
                }
                None if valid_id(task) => Ok(Self::TaskLatest {
                    task_id: task.into(),
                }),
                _ => Err(InvalidRunHandle),
            };
        }
        Err(InvalidRunHandle)
    }
}

impl fmt::Display for RunHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Flow { run_id } => write!(f, "flow:{run_id}"),
            Self::Task {
                task_id,
                execution_id,
            } => write!(f, "task:{task_id}/{execution_id}"),
            Self::TaskLatest { task_id } => write!(f, "task:{task_id}"),
        }
    }
}

impl Serialize for RunHandle {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for RunHandle {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {}

/// One execution's current facts. Completed means the session ended normally;
/// it does not judge whether the requested work succeeded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunResult {
    pub handle: RunHandle,
    pub what: String,
    pub why: Option<String>,
    pub state: RunState,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum RunState {
    Running {
        progress: Progress,
        waiting_on: Option<WaitingOn>,
    },
    Ended {
        ended_as: EndedAs,
        text: Option<String>,
        pointers: Pointers,
    },
    Unknown,
    Gone,
    Unreadable {
        reason: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndedAs {
    Completed,
    Failed,
    Canceled,
    Stalled,
    Crashed,
    Interrupted,
    Undelivered,
    Error,
    BudgetExhausted,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WaitingOn {
    Question { node: String },
    Permission { tool: Option<String> },
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Progress {
    pub node: Option<String>,
    pub attempt: Option<u32>,
    pub elapsed_secs: u64,
    pub chat: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pointers {
    pub run_dir: Option<std::path::PathBuf>,
    pub report: Option<std::path::PathBuf>,
    pub outputs: Vec<std::path::PathBuf>,
    pub chat: Option<String>,
    pub transcript: Option<std::path::PathBuf>,
}

/// A normal wait result; timeouts and empty scope are not tool failures.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitOutcome {
    Settled,
    TimedOut,
    Nothing,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaitAnswer {
    pub outcome: WaitOutcome,
    pub ended: Vec<RunResult>,
    pub blocked: Vec<RunResult>,
    pub already_reported: Vec<RunResult>,
    pub not_running: Vec<RunResult>,
    pub pending: Vec<RunResult>,
    pub more: usize,
}
