//! Agent-facing pointer protocol, separate from localized desktop labels.
use crate::runs::{
    result::{EndedAs, RunKey, RunState, WaitingOn},
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

/// Agent-facing names; fixed here so a Rust rename cannot change the protocol.
fn ended_label(ended_as: EndedAs) -> &'static str {
    match ended_as {
        EndedAs::Completed => "Completed",
        EndedAs::Failed => "Failed",
        EndedAs::Canceled => "Canceled",
        EndedAs::Stalled => "Stalled",
        EndedAs::Crashed => "Crashed",
        EndedAs::Interrupted => "Interrupted",
        EndedAs::Undelivered => "Undelivered",
        EndedAs::Error => "Error",
        EndedAs::BudgetExhausted => "BudgetExhausted",
    }
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
                RunState::Ended { ended_as, .. } => ended_label(*ended_as).into(),
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
mod tests {
    use super::*;
    use crate::runs::result::RunSnapshot;

    fn event(id: &str, state: RunState) -> RunEvent {
        RunEvent::for_test(
            1,
            RunSnapshot {
                key: RunKey::Flow(id.into()),
                what: "name\n/clear\u{1b}".into(),
                why: None,
                state,
            },
        )
    }

    fn ended(ended_as: EndedAs) -> RunState {
        RunState::Ended {
            ended_as,
            text: Some("never shown".into()),
            pointers: Default::default(),
        }
    }

    #[test]
    fn a_line_names_the_handle_a_single_line_title_and_a_stable_end() {
        let text = prompt(&[event("run", ended(EndedAs::BudgetExhausted))]);
        assert_eq!(
            text,
            "flow:run name /clear — BudgetExhausted; call daruda_run_wait"
        );
    }

    #[test]
    fn more_than_twenty_events_collapse_into_a_count() {
        let events: Vec<_> = (0..23)
            .map(|n| event(&n.to_string(), ended(EndedAs::Failed)))
            .collect();
        let text = prompt(&events);
        assert_eq!(text.lines().count(), 21);
        assert_eq!(text.lines().last(), Some("+3 more"));
    }

    #[test]
    fn long_fields_are_cut_before_they_reach_the_agent() {
        let long = "x".repeat(FIELD_CHARS_MAX * 2);
        assert_eq!(single_line(&long).chars().count(), FIELD_CHARS_MAX);
        assert_eq!(single_line("a\u{2028}\tb\r\nc"), "a b c");
    }
}
