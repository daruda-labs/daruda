//! Execution results owned by the process, independent of transport types.
use daruda_core::text::{bound_agent_text, char_prefix};
use daruda_store::tasks::{ExecutionRef, Lifecycle, TaskExecution, TaskRecord};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RunKey {
    Flow(String),
    Task(ExecutionRef),
}
impl std::hash::Hash for RunKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            Self::Flow(id) => id.hash(state),
            Self::Task(exec) => {
                exec.task_id.hash(state);
                exec.execution_id.hash(state);
            }
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RunSnapshot {
    pub key: RunKey,
    pub what: String,
    pub why: Option<String>,
    pub state: RunState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RunState {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EndedAs {
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum WaitingOn {
    Question { node: String },
    Permission { tool: Option<String> },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Progress {
    pub node: Option<String>,
    pub attempt: Option<u32>,
    pub elapsed_secs: u64,
    pub chat: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Pointers {
    pub run_dir: Option<std::path::PathBuf>,
    pub report: Option<std::path::PathBuf>,
    pub outputs: Vec<std::path::PathBuf>,
    pub chat: Option<String>,
    pub transcript: Option<std::path::PathBuf>,
}

#[derive(Clone, Debug)]
pub(crate) struct FlowRunLocation {
    pub run_dir: std::path::PathBuf,
    pub lock_dir: Option<std::path::PathBuf>,
}

const WHY_CHARS: usize = 300;

pub(super) fn task_snapshot(record: &TaskRecord, exec: &ExecutionRef) -> Option<RunSnapshot> {
    let ended = record.ended_executions.iter().find(|end| &end.exec == exec);
    let (lifecycle, execution, started_at, text) = if let Some(end) = ended {
        (
            &end.ended_as,
            end.execution.as_ref(),
            end.started_at,
            end.result.as_deref(),
        )
    } else if record.current_exec() == Some(exec) {
        (
            &record.lifecycle,
            record.execution.as_ref(),
            record.started_at,
            None,
        )
    } else {
        return None;
    };
    let description = ended.map_or(record.description.as_ref(), |end| end.description.as_ref());
    let pointers = task_pointers(execution);
    let ended_as = match lifecycle {
        Lifecycle::Done { .. } => Some(EndedAs::Completed),
        Lifecycle::Error { .. } => Some(EndedAs::Error),
        Lifecycle::Cancelled { .. } => Some(EndedAs::Canceled),
        Lifecycle::Undelivered { .. } => Some(EndedAs::Undelivered),
        Lifecycle::Interrupted { .. } => Some(EndedAs::Interrupted),
        _ => None,
    };
    let state = if let Some(ended_as) = ended_as {
        RunState::Ended {
            ended_as,
            text: text.and_then(bound_agent_text),
            pointers,
        }
    } else {
        RunState::Running {
            progress: Progress {
                elapsed_secs: started_at
                    .map(|at| (chrono::Utc::now() - at).num_seconds().max(0) as u64)
                    .unwrap_or(0),
                chat: pointers.chat.clone(),
                ..Default::default()
            },
            waiting_on: None,
        }
    };
    Some(RunSnapshot {
        key: RunKey::Task(exec.clone()),
        what: description
            .map(|description| description.title.clone())
            .unwrap_or_default(),
        why: description
            .map(|description| char_prefix(&description.prompt, WHY_CHARS).0.to_owned())
            .filter(|v| !v.trim().is_empty()),
        state,
    })
}

fn task_pointers(execution: Option<&TaskExecution>) -> Pointers {
    Pointers {
        chat: execution.and_then(|run| run.session_id.clone()),
        transcript: execution.and_then(|run| match &run.source {
            daruda_store::tasks::TaskExecutionSource::ClaudeCli {
                transcript_path, ..
            } => transcript_path.clone(),
            daruda_store::tasks::TaskExecutionSource::AgentChat => None,
        }),
        ..Default::default()
    }
}

/// Freeze the end from process facts even when the log/report could not be written.
pub(super) fn flow_ended(
    mut snapshot: RunSnapshot,
    end: &daruda_flow::event::RunEnd,
    pointers: Pointers,
) -> RunSnapshot {
    use daruda_flow::event::RunEnd;
    let ended_as = match end {
        RunEnd::Done => EndedAs::Completed,
        RunEnd::Failed { .. } => EndedAs::Failed,
        RunEnd::Canceled { .. } => EndedAs::Canceled,
        RunEnd::BudgetExhausted { .. } => EndedAs::BudgetExhausted,
        RunEnd::Stalled { .. } => EndedAs::Stalled,
        _ => EndedAs::Error,
    };
    let text = match end {
        RunEnd::Io { message, .. }
        | RunEnd::Log { message, .. }
        | RunEnd::NotResumed { message }
        | RunEnd::Unprovisioned { message, .. } => bound_agent_text(message),
        _ => None,
    };
    snapshot.state = RunState::Ended {
        ended_as,
        text,
        pointers,
    };
    snapshot
}

pub(super) fn flow_why(
    until: Option<&daruda_flow::NodeId>,
    pinned: impl Iterator<Item = String>,
) -> Option<String> {
    let mut why = Vec::new();
    if let Some(until) = until {
        why.push(format!("until: {until}"));
    }
    let pinned = pinned.collect::<Vec<_>>();
    if !pinned.is_empty() {
        why.push(format!("pinned: {}", pinned.join(", ")));
    }
    (!why.is_empty()).then(|| why.join("; "))
}

#[cfg(test)]
mod tests;
