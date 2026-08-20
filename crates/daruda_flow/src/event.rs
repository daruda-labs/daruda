//! What a run tells a subscriber while it is going, so a host can draw it
//! without polling the run directory.
//!
//! The stream is deliberately coarser than the scheduler's own state: a host
//! draws a list of nodes and a status per node, and anything finer is churn
//! it has to ignore.

use crate::NodeId;
use crate::error::IoSite;
use crate::lock::LockHolder;
use crate::runner::NodeFailure;
use crate::schedule::{BudgetLimit, RunOutcome};
use std::path::PathBuf;

/// What a subscriber needs to render a run in progress.
#[derive(Debug, Clone)]
pub enum FlowEvent {
    RunStarted {
        run_dir: PathBuf,
        /// In the order they will run, which is the order a host draws.
        nodes: Vec<NodeId>,
    },
    NodeStarted {
        node: NodeId,
        attempt: u32,
    },
    NodePassed {
        node: NodeId,
        attempt: u32,
    },
    NodeFailed {
        node: NodeId,
        attempt: u32,
        failure: NodeFailure,
    },
    /// A gate's repair is running its `fix`. Its own events rather than
    /// `NodeStarted { node: "__fix__" }`, because the fix is not in the node
    /// list `RunStarted` sent — a host drawing that list would have nowhere
    /// to put it — and because naming the gate is what makes it renderable
    /// at all.
    FixStarted {
        gate: NodeId,
    },
    FixEnded {
        gate: NodeId,
        failure: Option<NodeFailure>,
    },
    /// A repair is about to re-derive these. The host redraws them as
    /// pending, which is the one transition it cannot infer.
    Rerunning {
        gate: NodeId,
        members: Vec<NodeId>,
    },
    RunEnded {
        end: RunEnd,
    },
}

/// How the run ended, in a form a subscriber can hold. `RunOutcome` is not
/// `Clone` — `FlowIoError` wraps a `std::io::Error` — and the marker's status
/// folds `Failed`, `BudgetExhausted` and `Io` into one word, which is exactly
/// the distinction a watcher needs. So the stream carries its own shape, kept
/// in step by one `From<&RunOutcome>` and a test that walks every variant.
#[derive(Debug, Clone)]
pub enum RunEnd {
    Done,
    Failed {
        node: NodeId,
        failure: NodeFailure,
    },
    Canceled {
        node: Option<NodeId>,
    },
    BudgetExhausted {
        limit: BudgetLimit,
    },
    /// Rendered on the way in, because the source error cannot be cloned.
    Io {
        site: IoSite,
        doing: &'static str,
        path: PathBuf,
        message: String,
    },
    /// Never reaches a subscriber through `execute` — that run never took the
    /// directory, so it emits neither `RunStarted` nor `RunEnded`. The arm
    /// exists so the mapping stays total for anything converting a finished
    /// report after the fact.
    LockHeld {
        holder: LockHolder,
    },
    /// Like `LockHeld`, never seen through `execute`: the request failed
    /// before there was a run to announce. The arm keeps the mapping total.
    Invalid {
        issues: Vec<crate::error::ValidationIssue>,
    },
    /// The run took the directory and then found it had no runtime to run
    /// with, so a subscriber does see this one.
    Unprovisioned {
        agent: String,
        message: String,
    },
}

/// The one place an event is handed over. Emitting can never fail a run: a
/// full channel is impossible (unbounded) and a closed one means the host
/// stopped listening, which is not this crate's problem to report. Never
/// `send().await` — a subscriber must not be able to pause the scheduler.
pub(crate) fn emit(events: Option<&smol::channel::Sender<FlowEvent>>, event: FlowEvent) {
    if let Some(tx) = events {
        let _ = tx.try_send(event);
    }
}

impl From<&RunOutcome> for RunEnd {
    fn from(outcome: &RunOutcome) -> Self {
        match outcome {
            RunOutcome::Done => RunEnd::Done,
            RunOutcome::Failed { node, failure } => RunEnd::Failed {
                node: node.clone(),
                failure: failure.clone(),
            },
            RunOutcome::Canceled { node } => RunEnd::Canceled { node: node.clone() },
            RunOutcome::BudgetExhausted { limit } => RunEnd::BudgetExhausted { limit: *limit },
            RunOutcome::Io(e) => RunEnd::Io {
                site: e.site.clone(),
                doing: e.doing,
                path: e.path.clone(),
                message: e.source.to_string(),
            },
            RunOutcome::Invalid { issues } => RunEnd::Invalid {
                issues: issues.clone(),
            },
            RunOutcome::LockHeld { holder } => RunEnd::LockHeld {
                holder: holder.clone(),
            },
            RunOutcome::Unprovisioned { agent, message } => RunEnd::Unprovisioned {
                agent: agent.clone(),
                message: message.clone(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::FlowIoError;
    use crate::marker::{RunStatus, status_of};
    use std::collections::HashSet;

    /// The variant's name, so a table can compare shapes without demanding
    /// `PartialEq` from types that cannot offer it.
    fn end_name(end: &RunEnd) -> &'static str {
        match end {
            RunEnd::Done => "Done",
            RunEnd::Failed { .. } => "Failed",
            RunEnd::Canceled { .. } => "Canceled",
            RunEnd::BudgetExhausted { .. } => "BudgetExhausted",
            RunEnd::Io { .. } => "Io",
            RunEnd::LockHeld { .. } => "LockHeld",
            RunEnd::Invalid { .. } => "Invalid",
            RunEnd::Unprovisioned { .. } => "Unprovisioned",
        }
    }

    /// What a watcher and the marker each see, per outcome. Exhaustive on
    /// purpose: a seventh `RunOutcome` stops compiling here until someone
    /// decides both answers.
    fn expected(outcome: &RunOutcome) -> (&'static str, Option<RunStatus>) {
        match outcome {
            RunOutcome::Done => ("Done", Some(RunStatus::Done)),
            RunOutcome::Failed { .. } => ("Failed", Some(RunStatus::Failed)),
            RunOutcome::Canceled { .. } => ("Canceled", Some(RunStatus::Canceled)),
            RunOutcome::BudgetExhausted { .. } => ("BudgetExhausted", Some(RunStatus::Failed)),
            RunOutcome::Io(_) => ("Io", Some(RunStatus::Failed)),
            // No marker and no `RunEnded`: neither run started.
            RunOutcome::LockHeld { .. } => ("LockHeld", None),
            RunOutcome::Invalid { .. } => ("Invalid", None),
            RunOutcome::Unprovisioned { .. } => ("Unprovisioned", Some(RunStatus::Failed)),
        }
    }

    fn every_outcome() -> Vec<RunOutcome> {
        vec![
            RunOutcome::Done,
            RunOutcome::Failed {
                node: "gate".into(),
                failure: NodeFailure::Exit { code: Some(1) },
            },
            RunOutcome::Canceled {
                node: Some("design".into()),
            },
            RunOutcome::BudgetExhausted {
                limit: BudgetLimit::NodeRuns,
            },
            RunOutcome::Io(FlowIoError {
                site: IoSite::Node {
                    node: "design".into(),
                    attempt: 1,
                },
                doing: "reading the prompt file",
                path: PathBuf::from("gone.md"),
                source: std::io::Error::other("boom"),
            }),
            RunOutcome::Invalid {
                issues: vec![crate::error::ValidationIssue {
                    node: None,
                    kind: crate::error::ValidationKind::RelativeRequestPath { field: "cwd" },
                    message: "relative".to_string(),
                }],
            },
            RunOutcome::LockHeld {
                holder: LockHolder {
                    pid: 4242,
                    run_id: "other".to_string(),
                    started_unix_secs: 1,
                },
            },
            RunOutcome::Unprovisioned {
                agent: "claude".to_string(),
                message: "no managed Node.js build for this platform".to_string(),
            },
        ]
    }

    /// Every `RunOutcome` variant must map, including the one that emits
    /// nothing. Adding an outcome without deciding what a watcher sees is the
    /// drift this catches.
    #[test]
    fn every_outcome_has_a_run_end_and_a_marker_that_agree() {
        let outcomes = every_outcome();
        let mut names = HashSet::new();
        for outcome in &outcomes {
            let (name, status) = expected(outcome);
            assert_eq!(end_name(&RunEnd::from(outcome)), name, "{outcome:?}");
            assert_eq!(status_of(outcome), status, "{outcome:?}");
            names.insert(name);
        }
        assert_eq!(
            names.len(),
            8,
            "a variant added to `expected` needs a sample here too"
        );
    }

    /// The reason `RunEnd` exists at all: the source error is not cloneable,
    /// so its wording has to be rendered on the way in rather than kept.
    #[test]
    fn an_io_outcome_keeps_its_wording_after_the_error_is_gone() {
        let end = RunEnd::from(&RunOutcome::Io(FlowIoError {
            site: IoSite::Run,
            doing: "recording the resolved spec",
            path: PathBuf::from("/tmp/run.yaml"),
            source: std::io::Error::other("disk on fire"),
        }));
        match end {
            RunEnd::Io { message, path, .. } => {
                assert!(message.contains("disk on fire"), "{message}");
                assert!(path.ends_with("run.yaml"), "{path:?}");
            }
            other => panic!("expected Io, got {other:?}"),
        }
    }
}
