//! The steps a resume takes once its log is open, up to scheduling.

use std::path::{Path, PathBuf};

use daruda_runlog::{LogBackend, RunLog};

use super::ResumeError;
use crate::NodeId;
use crate::error::IoSite;
use crate::load::LoadedFlow;
use crate::run_log::machine::{
    AbandonReason, AttemptState, FlowRunEvent, FlowRunMachine, FlowRunState, Produces,
};
use crate::run_log::reader::{self, Replay};
use crate::run_log::store::FlowRunStore;
use crate::runner::OutputContract;
use crate::schedule::RunOutcome;

/// The rest of §5.5 once the log is open: steps 3–7. Returns the store and
/// what the scheduler starts from (step 8).
pub(crate) fn pick_up<B: LogBackend + Send + 'static>(
    log: RunLog<FlowRunMachine, B>,
    loaded: &LoadedFlow,
    run_dir: &Path,
    events: Option<smol::channel::Sender<crate::event::RunNotice>>,
    warnings: &mut Vec<String>,
) -> Result<(FlowRunStore, Replay), RunOutcome> {
    if !log.state().is_resumable() {
        let why = match log.state().phase() {
            crate::run_log::machine::RunPhase::Fresh => ResumeError::NothingStarted,
            _ => ResumeError::NotResumable(crate::marker::run_status(run_dir, None, &|_| false)),
        };
        return Err(RunOutcome::NotResumed(why));
    }
    // Step 3. A marker left in place would keep saying how the run last
    // ended while it runs again (F8).
    if let Err(e) = crate::marker::clear_markers(run_dir) {
        warnings.push(e.to_string());
    }
    // Step 4. Before `Resumed` turns these attempts into interrupted ones
    // and forgets their groups.
    warnings.extend(end_orphaned_commands(log.state()));
    let store = FlowRunStore::spawn(log, run_dir, events)?;
    smol::block_on(resume_steps(&store, loaded, run_dir, warnings))?;
    let history = store.events().map_err(|e| store.failure(IoSite::Run, e))?;
    Ok((store, reader::replay_events(&history)))
}

async fn resume_steps(
    store: &FlowRunStore,
    loaded: &LoadedFlow,
    run_dir: &Path,
    warnings: &mut Vec<String>,
) -> Result<(), RunOutcome> {
    let record = |e| store.failure(IoSite::Run, e);
    // Step 5. The reply channels died with the process, so nothing can
    // answer these; left open they would keep the run from ever ending (B5).
    let open = store.state().open_ask_ids();
    for id in open {
        let reason = AbandonReason::AppRestart;
        store
            .commit(FlowRunEvent::AskAbandoned { id, reason })
            .await
            .map_err(record)?;
    }
    let carried = store.state().finished().len();
    store
        .commit(FlowRunEvent::Resumed { carried })
        .await
        .map_err(record)?;

    // Step 6. A pass is only as good as the file it left; directory entries
    // may not survive a power cut, so check (F4). Commands and pins leave
    // nothing to check.
    let outputs = crate::schedule::node_outputs(loaded.flow(), run_dir);
    for node in &loaded.flow().nodes {
        if !output_lost(store, run_dir, node) {
            continue;
        }
        store
            .commit(FlowRunEvent::OutputLost {
                node: node.id.clone(),
            })
            .await
            .map_err(record)?;
    }

    // Step 7. Whatever an unfinished or invalidated node left live is
    // evidence, not a result (F3).
    let passed: Vec<NodeId> = store.state().finished().into_iter().collect();
    let resumes = store.state().resumes;
    let log_dir = run_dir.join(crate::schedule::LOG_DIR_NAME);
    warnings.extend(archive_unclaimed_outputs(
        &log_dir, &outputs, &passed, resumes,
    ));
    Ok(())
}

/// End the command groups a dead process left running (F7). A command
/// survives its host in a group of its own, and the retry would otherwise
/// run beside it. A pid whose identity no longer matches is someone else's.
fn end_orphaned_commands(state: &FlowRunState) -> Vec<String> {
    state
        .unsettled_processes()
        .into_iter()
        .filter_map(|(node, process)| {
            let identity = daruda_core::process::ProcessIdentity::from_raw(process.identity);
            daruda_core::process::end_recorded_group(process.leader, identity)
                .err()
                .map(|e| {
                    format!(
                        "`{node}`'s command from before the crash could not be stopped, so it \
                         may still be running beside its retry: {e}"
                    )
                })
        })
        .collect()
}

/// Whether a passed agent node's output no longer meets its contract.
fn output_lost(store: &FlowRunStore, run_dir: &Path, node: &crate::model::Node) -> bool {
    let passed = store.state().node(&node.id).is_some_and(|state| {
        state.produces == Produces::File
            && !state.invalidated
            && matches!(state.last, AttemptState::Passed { .. })
    });
    if !passed {
        return false;
    }
    let Some(output) = crate::schedule::node_output(node, run_dir) else {
        return false;
    };
    let contract = crate::contract::file::FileContract::new(
        run_dir,
        &output,
        node.kind.output_schema(),
        node.kind.continue_until(),
    );
    contract.check().is_err()
}

/// Move aside whatever an interrupted or invalidated node had written.
///
/// A node that did not settle may have left a half-written output, and
/// `judge` cannot tell that from a good one — it only asks whether the file
/// is there and non-empty. Every live output that no pass claims is
/// therefore evidence, not a result.
///
/// Best-effort, and it reports rather than fails: the paths it could not
/// move become warnings, because a resume that refuses over a stale file is
/// worse than one that says which file is stale.
pub(crate) fn archive_unclaimed_outputs(
    log_dir: &Path,
    outputs: &[(NodeId, PathBuf)],
    passed: &[NodeId],
    resume: u32,
) -> Vec<String> {
    let mut warnings = Vec::new();
    for (id, output) in outputs {
        // Asked without following links, so all three link shapes reach the
        // archive's refusal: `is_file` is false for a dangling one and for
        // one to a directory, which would skip both without a word.
        if passed.contains(id) || std::fs::symlink_metadata(output).is_err() {
            continue;
        }
        let stamp = crate::archive::CancelStamp::Resume(resume);
        match crate::archive::archive_canceled(log_dir, id, output, stamp) {
            Ok(_) => {}
            Err(e) => warnings.push(format!(
                "`{id}` left a partial output that could not be moved aside, so it may be \
                 mistaken for a finished one: {e}"
            )),
        }
    }
    warnings
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The output of a node that never settled is moved aside. Left live,
    /// `judge` would accept a half-written file as that node's result — the
    /// one way a resume can produce a wrong answer rather than a slow one.
    #[test]
    fn a_partial_output_from_the_interrupted_node_is_moved_aside() {
        let dir = tempfile::tempdir().expect("tempdir");
        let log_dir = dir.path().join("logs");
        let done = dir.path().join("design.md");
        let partial = dir.path().join("half.md");
        std::fs::write(&done, "finished\n").expect("write");
        std::fs::write(&partial, "half a th").expect("write");

        let warnings = archive_unclaimed_outputs(
            &log_dir,
            &[
                ("design".into(), done.clone()),
                ("half".into(), partial.clone()),
            ],
            &["design".into()],
            1,
        );

        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(done.is_file(), "a passed node's output was taken away");
        assert!(!partial.exists(), "the partial output is still live");
    }

    /// A link where an output belongs is residue too, and every shape of one
    /// has to say so: on `is_file` a dangling link and one to a directory are
    /// both indistinguishable from nothing written, so the sweep passed over
    /// them and left them live for `judge` to read.
    #[test]
    fn every_shape_of_link_left_behind_is_reported_rather_than_skipped() {
        let dir = tempfile::tempdir().expect("tempdir");
        let log_dir = dir.path().join("logs");
        let elsewhere = dir.path().join("elsewhere.md");
        std::fs::write(&elsewhere, "someone else's work\n").expect("write");
        let a_directory = dir.path().join("a-directory");
        std::fs::create_dir_all(&a_directory).expect("mkdir");

        for (name, target) in [
            ("to_a_file", elsewhere.clone()),
            ("dangling", dir.path().join("never-written.md")),
            ("to_a_directory", a_directory.clone()),
        ] {
            let output = dir.path().join(format!("{name}.md"));
            daruda_core::path::symlink(&target, &output).expect("symlink");

            let warnings =
                archive_unclaimed_outputs(&log_dir, &[(name.into(), output.clone())], &[], 1);

            assert_eq!(warnings.len(), 1, "{name}: {warnings:?}");
            assert!(warnings[0].contains(name), "{name}: {warnings:?}");
            assert!(
                std::fs::symlink_metadata(&output).is_ok(),
                "{name}: a refused link is left where a reader can see it"
            );
            daruda_core::path::remove_symlink(&output).expect("unlink");
        }
    }
}
