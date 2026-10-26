//! Invalidate and archive a failed attempt before its retry or repair starts.

use super::{Run, RunOutcome, doing, policy};
use crate::NodeId;
use crate::model::Node;
use crate::record::{AttemptOutcome, Invalidation, Reported};
use crate::run_log::machine::FlowRunEvent;
use crate::runner::{NodeFailure, RunContext};
use std::path::PathBuf;

pub(super) struct ArchivedAttempt {
    pub(super) members: Vec<NodeId>,
    pub(super) evidence: Vec<PathBuf>,
}

impl Run<'_> {
    pub(super) async fn archive_failed_attempt(
        &self,
        node: &Node,
        policy: &policy::Policy,
        ctx: &RunContext<'_>,
        failure: &NodeFailure,
        reported: Reported,
        artifacts: &[PathBuf],
    ) -> Result<ArchivedAttempt, RunOutcome> {
        // Archive the whole invalidation set before anything else
        // runs — a fix that starts first would read a live output
        // instead of the evidence it is supposed to answer.
        let set = self.invalidation_set(node, policy);
        // Which outputs stop being current is durable before any
        // of them is moved, so a crash in between is a resume's to
        // finish rather than a pass with no output.
        let members: Vec<NodeId> = set
            .iter()
            .map(|(member, _)| member.clone())
            .filter(|member| member != ctx.node_id)
            .collect();
        if !members.is_empty() {
            let gate = ctx.node_id.clone();
            let members = members.clone();
            self.log
                .commit(FlowRunEvent::Invalidated { gate, members })
                .await
                .map_err(|e| self.log.failure(super::site(ctx.node_id, ctx.attempt), e))?;
        }
        match crate::archive::archive_attempt(
            &self.log_dir,
            &set,
            ctx.attempt,
            ctx.evidence_seq,
            artifacts,
        ) {
            Ok(paths) => {
                super::durable::sync_dirs(&set, &self.log_dir).await;
                // The archive's own return, not a second derivation
                // of it: two computations of the same set drift.
                let invalidation = Invalidation {
                    nodes: set.iter().map(|(id, _)| id.clone()).collect(),
                    archived: paths.clone(),
                };
                self.settle(
                    ctx,
                    AttemptOutcome::Failed(failure.clone()),
                    invalidation,
                    reported,
                )
                .await?;
                Ok(ArchivedAttempt {
                    members,
                    evidence: paths,
                })
            }
            Err(e) => {
                // Same reasoning as the cancel path: the attempt is
                // in the accounting whether or not its evidence
                // could be moved, so it is in the record too.
                self.settle(
                    ctx,
                    AttemptOutcome::Failed(failure.clone()),
                    Invalidation::default(),
                    reported,
                )
                .await?;
                let (path, source) = e.into_io();
                Err(super::node_io(ctx, doing::ARCHIVE, path, source))
            }
        }
    }
}

#[cfg(test)]
mod tests {}
