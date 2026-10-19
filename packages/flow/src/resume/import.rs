//! Opening a resumed run's log under its lock, and taking over the journal
//! of a run an older build started.

use std::path::Path;

use daruda_runlog::RunLog;

use super::ResumeError;
use crate::NodeId;
use crate::error::IoSite;
use crate::load::LoadedFlow;
use crate::run_log::line::AttemptLine;
use crate::run_log::machine::{
    FlowRunEvent, FlowRunMachine, Imported, ImportedAttempt, ImportedLast, ImportedNode,
    ImportedOutcome,
};
use crate::run_log::reader;
use crate::schedule::RunOutcome;

const IMPORT: &str = "taking over the run's progress journal";

/// Open the run's log under the lock, taking over the journal of a run an
/// older build started.
pub(crate) fn open_under_lock(
    run_dir: &Path,
    loaded: &LoadedFlow,
) -> Result<RunLog<FlowRunMachine, daruda_runlog::FileBackend>, RunOutcome> {
    if !reader::started(run_dir) {
        // Found once the lock is held: nothing was recorded, so the run
        // directory is left as it was.
        return Err(RunOutcome::NotResumed(ResumeError::NothingStarted));
    }
    let mut log = crate::run_log::store::open_file_log(run_dir)?;
    // Decided on what the opened log holds, not on whether it exists: a
    // crash between opening it and committing the import leaves a log with
    // nothing in it, and the journal is still the run's record.
    if log.state().phase() != crate::run_log::machine::RunPhase::Fresh
        || !crate::journal::exists(run_dir)
    {
        return Ok(log);
    }
    log.commit(import_journal(run_dir, loaded)).map_err(|e| {
        crate::run_log::store::LogFault::from(e).outcome(
            IoSite::Run,
            IMPORT,
            crate::run_log::log_dir(run_dir),
        )
    })?;
    Ok(log)
}

/// What the old journal knew, as one record. Only into an empty log.
fn import_journal(run_dir: &Path, loaded: &LoadedFlow) -> FlowRunEvent {
    let replay = crate::journal::read(run_dir);
    let records = replay
        .records
        .iter()
        .flat_map(|node| {
            node.attempts.iter().map(|attempt| ImportedAttempt {
                node: node.id.clone(),
                outcome: match &attempt.outcome {
                    crate::record::AttemptOutcome::Passed => ImportedOutcome::Passed,
                    crate::record::AttemptOutcome::Canceled => ImportedOutcome::Canceled,
                    crate::record::AttemptOutcome::Reported(reason) => ImportedOutcome::Reported {
                        reason: reason.clone(),
                    },
                    crate::record::AttemptOutcome::Failed(failure) => ImportedOutcome::Reported {
                        reason: format!("failed: {failure}"),
                    },
                },
                record: AttemptLine::new(attempt, &Default::default()),
            })
        })
        .collect();
    FlowRunEvent::Imported(Box::new(Imported {
        nodes: crate::run_log::node_starts(loaded),
        until: replay.until.clone(),
        passed: replay
            .passed
            .iter()
            .filter(|id| !replay.pinned.contains(id))
            .cloned()
            .collect(),
        pinned: replay.pinned.clone(),
        profile: replay.profile.clone(),
        next_evidence: replay.next_seq.max(1),
        spent: (&replay.spent).into(),
        records,
        carried: carried(&replay.records, &replay.passed),
    }))
}

/// Each unpassed node's settled attempts since a repair last invalidated
/// it. Counted, not read off attempt numbers: an older build's resume
/// restarted those at 1, so the numbers alone undercount.
fn carried(records: &[crate::record::NodeRecord], passed: &[NodeId]) -> Vec<ImportedNode> {
    use crate::record::AttemptOutcome;
    // The evidence number is run-wide and only rises, so an invalidation
    // splits a node's attempts into the generation before it and after.
    let mut boundary = std::collections::BTreeMap::<&NodeId, u32>::new();
    for attempt in records.iter().flat_map(|node| &node.attempts) {
        for id in &attempt.invalidated.nodes {
            let seen = boundary.entry(id).or_default();
            *seen = (*seen).max(attempt.evidence_seq);
        }
    }
    records
        .iter()
        .filter(|node| !passed.contains(&node.id))
        .filter_map(|node| {
            let since = boundary.get(&node.id).copied();
            let current: Vec<_> = node
                .attempts
                .iter()
                .filter(|attempt| since.is_none_or(|seq| attempt.evidence_seq > seq))
                .collect();
            let last = match &current.iter().max_by_key(|a| a.evidence_seq)?.outcome {
                AttemptOutcome::Canceled => ImportedLast::Canceled,
                AttemptOutcome::Reported(reason) => ImportedLast::Failed {
                    reason: reason.clone(),
                },
                AttemptOutcome::Failed(failure) => ImportedLast::Failed {
                    reason: failure.to_string(),
                },
                AttemptOutcome::Passed => return None,
            };
            Some(ImportedNode {
                node: node.id.clone(),
                attempts: u32::try_from(current.len()).unwrap_or(u32::MAX),
                last,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn journaled(
        seq: u32,
        outcome: crate::record::AttemptOutcome,
        invalidated: &[&str],
    ) -> crate::record::AttemptRecord {
        crate::record::AttemptRecord {
            attempt: 1,
            evidence_seq: seq,
            outcome,
            invalidated: crate::record::Invalidation {
                nodes: invalidated.iter().map(|id| NodeId::from(*id)).collect(),
                archived: Vec::new(),
            },
            git_status: None,
            at: std::time::UNIX_EPOCH,
            took: Duration::ZERO,
            waited: crate::runner::Waiting::default(),
            tools: Vec::new(),
            turns: 1,
            usage: None,
        }
    }

    /// A repair's invalidation starts the member's count over: only the
    /// attempts after it are carried, and a passed node carries nothing.
    #[test]
    fn only_the_current_generations_attempts_are_carried() {
        use crate::record::{AttemptOutcome, NodeRecord};
        let failed = || AttemptOutcome::Reported("exited with status 1".into());
        let records = vec![
            NodeRecord {
                id: "review".into(),
                attempts: vec![
                    journaled(1, failed(), &[]),
                    journaled(4, AttemptOutcome::Canceled, &[]),
                ],
            },
            NodeRecord {
                id: "gate".into(),
                attempts: vec![journaled(3, failed(), &["review", "gate"])],
            },
            NodeRecord {
                id: "design".into(),
                attempts: vec![journaled(2, AttemptOutcome::Passed, &[])],
            },
        ];

        let carried = carried(&records, &["design".into()]);

        assert_eq!(
            carried,
            vec![ImportedNode {
                node: "review".into(),
                attempts: 1,
                last: ImportedLast::Canceled,
            }]
        );
    }
}
