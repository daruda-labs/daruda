//! Agent-facing pointer protocol, separate from localized desktop labels.
use crate::runs::{
    result::{RunKey, RunState, WaitingOn},
    watch::RunEvent,
};

const LINES_MAX: usize = 20;
const FIELD_CHARS_MAX: usize = 200;

fn single_line(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_control() || c == '\u{2028}' || c == '\u{2029}' {
                ' '
            } else {
                c
            }
        })
        .take(FIELD_CHARS_MAX)
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn prompt(events: &[RunEvent]) -> String {
    let mut lines: Vec<_> = events
        .iter()
        .take(LINES_MAX)
        .map(|event| {
            let snapshot = &event.snapshot;
            let handle = match &snapshot.key {
                RunKey::Flow(id) => format!("flow:{id}"),
                RunKey::Task(exec) => format!("task:{}/{}", exec.task_id, exec.execution_id),
            };
            let summary = match &snapshot.state {
                RunState::Ended { ended_as, .. } => format!("{ended_as:?}"),
                RunState::Running {
                    waiting_on: Some(WaitingOn::Question { node }),
                    ..
                } => format!("Waiting for an answer at {}", single_line(node)),
                RunState::Running {
                    waiting_on: Some(WaitingOn::Permission { tool }),
                    ..
                } => tool.as_deref().map_or_else(
                    || "Waiting for permission".into(),
                    |tool| format!("Waiting for permission: {}", single_line(tool)),
                ),
                _ => "State changed".into(),
            };
            format!(
                "{handle} {} — {summary}; call daruda_run_wait",
                single_line(&snapshot.what)
            )
        })
        .collect();
    if events.len() > LINES_MAX {
        lines.push(format!("+{} more", events.len() - LINES_MAX));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {}
