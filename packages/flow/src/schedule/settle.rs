//! An attempt's life on the run log: its start is committed before the
//! session is paid for (O2), and its settle is the record of it. Separate
//! from the drive loop, which decides what happens next; this only records
//! what did.

use super::{Run, RunOutcome, doing, node_io, policy};
use crate::NodeId;
use crate::error::{FlowIoError, IoSite};
use crate::record::{AttemptOutcome, AttemptRecord, Invalidation, Reported};
use crate::run_log::line::AttemptLine;
use crate::run_log::machine::{AttemptState, FixOutcome, FlowRunEvent};
use crate::runner::{RunContext, RunResult};
use std::path::{Path, PathBuf};

impl Run<'_> {
    /// The number this node's next attempt will be given: the log assigns
    /// it, and nothing else starts this node meanwhile.
    pub(super) fn next_attempt(&self, id: &NodeId) -> u32 {
        self.log
            .state()
            .node(id)
            .map_or(1, |node| node.attempts + 1)
    }

    /// Attempts in this node's generation that count against its cap: a
    /// crash-interrupted one does not, a stop or a failure does (D4-2).
    pub(super) fn spent_attempts(&self, id: &NodeId) -> u32 {
        self.log.state().node(id).map_or(0, |node| node.spent())
    }

    /// A resumed node with no attempts left in its generation ends the run
    /// the way its last attempt did, without running again: a failure fails
    /// it, and a stop (the user's or the overage switch's) cancels it — the
    /// end that stop would have recorded had the process lived to. One a
    /// crash interrupted runs again, since that attempt did not count.
    pub(super) fn attempts_spent_before(
        &self,
        id: &NodeId,
        policy: &policy::Policy,
    ) -> Option<RunOutcome> {
        let state = self.log.state();
        let node = state.node(id)?;
        if node.spent() < policy.max_attempts {
            return None;
        }
        match &node.last {
            AttemptState::Failed { failure, .. } => Some(RunOutcome::Failed {
                node: id.clone(),
                failure: failure.into(),
            }),
            AttemptState::Canceled { .. } => Some(RunOutcome::Canceled {
                node: Some(id.clone()),
            }),
            _ => None,
        }
    }

    /// Commit `AttemptStarted` and hand back the numbers the log gave it.
    pub(super) async fn start_attempt(
        &self,
        id: &NodeId,
        next: u32,
    ) -> Result<Started, RunOutcome> {
        let generation = self.log.state().node(id).map_or(0, |node| node.generation);
        let committed = self
            .log
            .commit_start(FlowRunEvent::attempt_started(id.clone(), generation))
            .await
            .map_err(|e| self.log.failure(site(id, next), e))?;
        match committed.event {
            FlowRunEvent::AttemptStarted {
                attempt, evidence, ..
            } => Ok(Started {
                generation,
                attempt,
                evidence,
            }),
            _ => Err(self.log.failure(
                site(id, next),
                daruda_runlog::CommitError::Io(std::io::Error::other(
                    "the log answered an attempt with another event",
                )),
            )),
        }
    }

    /// Called wherever an attempt's fate is sealed, including the arms that
    /// return before `judge` or before any archiving. The `git_status` ask
    /// happens here, so the number of asks equals the number of attempts.
    ///
    /// The log is the record: a settle it cannot take ends the run, since
    /// nothing after it could be resumed from.
    pub(super) async fn settle(
        &self,
        ctx: &RunContext<'_>,
        outcome: AttemptOutcome,
        invalidated: Invalidation,
        reported: Reported,
    ) -> Result<(), RunOutcome> {
        let failure = match &outcome {
            AttemptOutcome::Failed(failure) => Some(failure.into()),
            _ => None,
        };
        let canceled = matches!(outcome, AttemptOutcome::Canceled);
        let record = self
            .attempt_record(ctx, outcome, invalidated, reported)
            .await;
        let line = Box::new(AttemptLine::new(&record, &self.spent.spent()));
        let node = ctx.node_id.clone();
        let event = match failure {
            Some(failure) => FlowRunEvent::AttemptFailed {
                node,
                failure,
                record: line,
            },
            None if canceled => FlowRunEvent::AttemptCanceled { node, record: line },
            None => FlowRunEvent::AttemptPassed { node, record: line },
        };
        self.spent
            .record(|records| crate::record::push_attempt(records, ctx.node_id, record));
        self.log
            .commit(event)
            .await
            .map(drop)
            .map_err(|e| self.log.failure(site(ctx.node_id, ctx.attempt), e))
    }

    /// The fix session's settle: recorded under `FIX_SESSION_ID`, committed
    /// as the end of `gate`'s fix.
    pub(super) async fn settle_fix(
        &self,
        ctx: &RunContext<'_>,
        gate: &NodeId,
        outcome: AttemptOutcome,
        reported: Reported,
    ) -> Result<(), RunOutcome> {
        let fix = match &outcome {
            AttemptOutcome::Failed(failure) => FixOutcome::Failed {
                failure: failure.into(),
            },
            AttemptOutcome::Canceled => FixOutcome::Canceled,
            _ => FixOutcome::Passed,
        };
        let record = self
            .attempt_record(ctx, outcome, Invalidation::default(), reported)
            .await;
        let line = Box::new(AttemptLine::new(&record, &self.spent.spent()));
        self.spent
            .record(|records| crate::record::push_attempt(records, ctx.node_id, record));
        let event = FlowRunEvent::FixEnded {
            gate: gate.clone(),
            outcome: fix,
            record: line,
        };
        self.log
            .commit(event)
            .await
            .map(drop)
            .map_err(|e| self.log.failure(site(gate, ctx.attempt), e))
    }

    async fn attempt_record(
        &self,
        ctx: &RunContext<'_>,
        outcome: AttemptOutcome,
        invalidated: Invalidation,
        reported: Reported,
    ) -> AttemptRecord {
        AttemptRecord {
            attempt: ctx.attempt,
            evidence_seq: ctx.evidence_seq,
            at: std::time::SystemTime::now(),
            // `Err` is a clock that moved backwards under us; the duration
            // is then simply not shown, which beats a wrong one.
            took: ctx.started_at.elapsed().unwrap_or_default(),
            outcome,
            invalidated,
            git_status: self.tree_note(ctx.cwd).await,
            waited: reported.waited,
            turns: reported.turns,
            tools: reported.tools,
            usage: reported.usage,
        }
    }

    /// The host's `git status` for `cwd`, asked off this thread: it is a
    /// blocking subprocess, and the wave's other nodes must keep going (F11).
    async fn tree_note(&self, cwd: &Path) -> Option<String> {
        let ask = self.git_status.clone()?;
        let cwd = cwd.to_path_buf();
        smol::unblock(move || ask(&cwd)).await
    }

    /// The user stopped the run while this node was in flight.
    ///
    /// Its output goes aside: everything left live in the run directory has
    /// to be a completed node's, or the next reader cannot tell which is
    /// which. The attempt is recorded either way — it happened and it cost
    /// a runner call, whether or not its output could be moved.
    pub(super) async fn settle_cancel(
        &self,
        ctx: &RunContext<'_>,
        id: &NodeId,
        output: Option<&Path>,
        result: &RunResult,
    ) -> RunOutcome {
        let mut archived = Vec::new();
        if let Some(output) = output {
            let stamp = crate::archive::CancelStamp::Evidence(ctx.evidence_seq);
            match crate::archive::archive_canceled(&self.log_dir, id, output, stamp) {
                Ok(moved) => {
                    // O1: the move is on disk before the record naming it.
                    let set = [(id.clone(), Some(output.to_path_buf()))];
                    super::durable::sync_dirs(&set, &self.log_dir).await;
                    archived.extend(moved);
                }
                Err(e) => {
                    let settled = self
                        .settle(
                            ctx,
                            AttemptOutcome::Canceled,
                            Invalidation::default(),
                            Reported::from(result),
                        )
                        .await;
                    if let Err(outcome) = settled {
                        return outcome;
                    }
                    let (path, source) = e.into_io();
                    return node_io(ctx, doing::ARCHIVE_CANCELED, path, source);
                }
            }
        }
        // A cancel invalidates only what it interrupted, so the set is this
        // node and nothing downstream re-derives.
        let settled = self
            .settle(
                ctx,
                AttemptOutcome::Canceled,
                Invalidation {
                    nodes: Vec::new(),
                    archived,
                },
                Reported::from(result),
            )
            .await;
        match settled {
            Err(outcome) => outcome,
            Ok(()) => RunOutcome::Canceled {
                node: Some(id.clone()),
            },
        }
    }
}

/// The numbers the log gave one attempt.
pub(super) struct Started {
    pub(super) generation: u32,
    pub(super) attempt: u32,
    pub(super) evidence: u32,
}

pub(super) fn site(node: &NodeId, attempt: u32) -> IoSite {
    IoSite::Node {
        node: node.clone(),
        attempt,
    }
}

/// An I/O failure before the attempt existed, named by the number it would
/// have had.
pub(super) fn io_at(
    node: &NodeId,
    attempt: u32,
    doing: &'static str,
    path: PathBuf,
    source: std::io::Error,
) -> RunOutcome {
    RunOutcome::Io(FlowIoError {
        site: site(node, attempt),
        doing,
        path,
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_site_names_the_node_and_its_attempt() {
        assert!(matches!(
            site(&NodeId::from("design"), 3),
            IoSite::Node { attempt: 3, .. }
        ));
    }
}
