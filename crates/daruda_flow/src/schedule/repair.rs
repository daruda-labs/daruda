//! Carrying out a repair: run the `fix` session, then re-derive what the
//! gate's failure invalidated. Next door in `policy.rs` decides which
//! policy a failure gets; this file is what happens when the answer is
//! `Repair`.

use super::{FIX_SESSION_ID, Run, RunOutcome};
use crate::NodeId;
use crate::event::FlowEvent;
use crate::record::{AttemptOutcome, Invalidation};
use crate::runner::{NodeFailure, RunContext};
use crate::template::{Surface, TemplateContext, render};
use std::path::PathBuf;

impl Run<'_> {
    /// One repair generation: run the `fix`, then re-derive what the gate's
    /// failure invalidated. `Err` ends the run — a failed fix changed
    /// nothing, so re-deriving would only re-prove the same verdict.
    pub(super) async fn repair(
        &self,
        fix: &str,
        rerun: &[NodeId],
        ctx: &RunContext<'_>,
        gate: &NodeId,
        evidence: &[PathBuf],
        failure: &NodeFailure,
    ) -> Result<(), RunOutcome> {
        self.run_fix(fix, ctx, evidence, failure).await?;
        // Each member starts a fresh generation of its own — that is the
        // rule that gives a nested gate its cap back.
        let members = self.rerun_members(rerun, gate);
        // The computed set, not the declared roots: a host cannot infer the
        // closure, the `∩ executed` filter or the recursion guard. An empty
        // one still says work resumed.
        self.emit(FlowEvent::Rerunning {
            gate: gate.clone(),
            members: members.clone(),
        });
        for member in members {
            self.drive(&member).await?;
        }
        Ok(())
    }

    /// Run `fix` as `flow.default_agent` under `FIX_SESSION_ID`. `Err` when
    /// the fix itself fails — the set is then not re-derived, because
    /// nothing was changed.
    async fn run_fix(
        &self,
        fix: &str,
        ctx: &RunContext<'_>,
        evidence: &[PathBuf],
        failure: &NodeFailure,
    ) -> Result<(), RunOutcome> {
        // `crate::validate` rejects a repair without an agent, so reaching
        // this arm means that rule has a hole; reporting beats a panic.
        let Some(agent) = &self.flow.default_agent else {
            return Err(RunOutcome::Failed {
                node: ctx.node_id.clone(),
                failure: NodeFailure::SessionError(
                    "this flow names no agent for a repair's fix session".to_string(),
                ),
            });
        };

        let tctx = TemplateContext {
            run_dir: self.run_dir,
            output: None,
            node_outputs: &self.node_outputs,
            failure: Some(failure),
            attempts: evidence,
        };
        let text = render(fix, &tctx, Surface::Prompt);

        let fix_id = NodeId::from(FIX_SESSION_ID);
        let reserve = || self.reserve_extra_turn();
        let fix_ctx = RunContext {
            // A fix session writes no file, so there is no contract to go round
            // for and nothing a second turn could change.
            max_turns: 1,
            node_id: &fix_id,
            attempt: ctx.attempt,
            // Its own: the fix is a session of its own, and dating it from
            // the gate's start would report it as having taken the gate's
            // whole life.
            started_at: std::time::SystemTime::now(),
            cwd: self.cwd,
            run_dir: self.run_dir,
            log_dir: &self.log_dir,
            // The fix owes no file: it edits the tree, and the re-derived
            // nodes are what produce evidence of that.
            output: None,
            contract: None,
            evidence_seq: self.take_seq(),
            // The gate's own timeout, by design (§6): a fix is a prompt
            // inside a policy rather than a node, so nothing else would
            // bound it and a hung fix session's only defence would be the
            // run's wall clock — which every other gate then has to share.
            //
            // The sharp edge that buys: a gate declared `timeout: 30s`
            // because it only runs `grep` gives its fix session 30s too.
            // An author wanting a longer repair raises the gate's.
            timeout: ctx.timeout,
            permission: self.permission_for_fix(agent),
            cancel: self.cancel,
            // The run's real door, though a fix owing no file never asks.
            reserve_extra_turn: &reserve,
        };
        // A fix is a real agent session and can take minutes. With no event
        // for it a host sits on `NodeFailed` and looks hung.
        self.emit(FlowEvent::FixStarted {
            gate: ctx.node_id.clone(),
        });
        let result = self
            .accounted_call(|| self.runner.run_agent(&fix_ctx, agent, &text))
            .await;
        // The fix never reaches `drive_inner` or `judge`, so its fate is
        // sealed here instead.
        let recorded = if self.cancel.is_canceled() {
            AttemptOutcome::Canceled
        } else {
            match &result.outcome {
                Ok(()) => AttemptOutcome::Passed,
                Err(failure) => AttemptOutcome::Failed(failure.clone()),
            }
        };
        self.record(
            &fix_ctx,
            recorded,
            Invalidation::default(),
            super::Reported::from(&result),
        );
        // Like any node: a cancel that interrupted the session is not a
        // failure of it, and reporting one would be wrong about why the run
        // stopped. The fix owes no output, so there is nothing to archive.
        if self.cancel.is_canceled() {
            // No `FixEnded`: a stop is not an ending the fix reached, and
            // `failure: None` would say it succeeded. `RunEnded` follows and
            // says why — the same rule an interrupted node follows.
            return Err(RunOutcome::Canceled { node: Some(fix_id) });
        }
        self.emit(FlowEvent::FixEnded {
            gate: ctx.node_id.clone(),
            failure: result.outcome.as_ref().err().cloned(),
        });
        match result.outcome {
            Ok(()) => Ok(()),
            Err(failure) => Err(RunOutcome::Failed {
                node: fix_id,
                failure,
            }),
        }
    }
}
