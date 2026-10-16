//! What a committed record tells a watcher. One function for the live stream
//! and for a replay, so the two cannot say different things.

use std::path::Path;

use super::machine::{FixOutcome, FlowRunEvent as E, FlowRunState};
use crate::event::FlowEvent;

pub(crate) fn narrate(run_dir: &Path, event: &E, after: &FlowRunState) -> Option<FlowEvent> {
    Some(match event {
        E::RunStarted { nodes, .. } => FlowEvent::RunStarted {
            run_dir: run_dir.to_path_buf(),
            nodes: nodes.iter().map(|node| node.id.clone()).collect(),
        },
        // A continuation announces itself the way a fresh run does.
        E::Resumed { .. } => FlowEvent::RunStarted {
            run_dir: run_dir.to_path_buf(),
            nodes: after.start.order.clone(),
        },
        E::AttemptStarted { node, attempt, .. } => FlowEvent::NodeStarted {
            node: node.clone(),
            attempt: *attempt,
        },
        E::NodeRefused {
            node,
            attempt,
            failure,
        } => FlowEvent::NodeFailed {
            node: node.clone(),
            attempt: *attempt,
            failure: failure.into(),
        },
        E::AttemptPassed { node, record } => FlowEvent::NodePassed {
            node: node.clone(),
            attempt: record.attempt,
        },
        E::AttemptFailed {
            node,
            failure,
            record,
        } => FlowEvent::NodeFailed {
            node: node.clone(),
            attempt: record.attempt,
            failure: failure.into(),
        },
        E::FixStarted { gate, .. } => FlowEvent::FixStarted { gate: gate.clone() },
        E::FixEnded { gate, outcome, .. } => match outcome {
            FixOutcome::Passed => FlowEvent::FixEnded {
                gate: gate.clone(),
                failure: None,
            },
            FixOutcome::Failed { failure } => FlowEvent::FixEnded {
                gate: gate.clone(),
                failure: Some(failure.into()),
            },
            // A stop is not an ending the fix reached; `RunEnded` says why.
            FixOutcome::Canceled => return None,
        },
        E::RerunScheduled { gate, members } => FlowEvent::Rerunning {
            gate: gate.clone(),
            members: members.clone(),
        },
        E::RunEnded { end } => FlowEvent::RunEnded {
            end: end.to_run_end(),
        },
        E::Imported(_)
        | E::AttemptCanceled { .. }
        | E::OutputLost { .. }
        | E::Invalidated { .. }
        | E::AskRaised { .. }
        | E::AskAnswered { .. }
        | E::AskAbandoned { .. } => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run_log::line::{AttemptLine, SpentLine};

    fn line() -> Box<AttemptLine> {
        Box::new(AttemptLine {
            attempt: 1,
            evidence: 1,
            at_ms: 0,
            took_ms: 0,
            waited_ms: 0,
            answers: Vec::new(),
            turns: 1,
            git_status: None,
            invalidated: Vec::new(),
            archived: Vec::new(),
            spent: SpentLine::default(),
        })
    }

    /// A stopped fix reached no ending, so it narrates none.
    #[test]
    fn a_stopped_fix_says_nothing() {
        let event = E::FixEnded {
            gate: "gate".into(),
            outcome: FixOutcome::Canceled,
            record: line(),
        };
        assert!(narrate(Path::new("/run"), &event, &FlowRunState::default()).is_none());
    }

    /// A continuation announces the nodes the run started with.
    #[test]
    fn a_resume_announces_the_run_again() {
        let mut after = FlowRunState::default();
        after.start.order = vec!["design".into(), "review".into()];
        let said = narrate(Path::new("/run"), &E::Resumed { carried: 1 }, &after);
        assert!(matches!(
            said,
            Some(FlowEvent::RunStarted { nodes, .. }) if nodes == after.start.order
        ));
    }
}
