//! A flow run's progress, committed to an append-only run log
//! (`daruda_runlog`) before anything is said about it: what started, what
//! settled, what a repair invalidated and which questions are open. A resume
//! restores numbers, outputs, invalidations and questions from it.

pub(crate) mod line;
pub mod machine;
mod narrate;
pub mod reader;
pub(crate) mod store;

use std::path::{Path, PathBuf};

/// Under the engine's own `logs/`, which no node output may enter, and apart
/// from `progress.jsonl` so an older build never reads it as its journal.
const RUN_LOG_DIR: &str = "progress";
/// Where the run in `run_dir` keeps its log.
pub fn log_dir(run_dir: &Path) -> PathBuf {
    run_dir
        .join(crate::schedule::LOG_DIR_NAME)
        .join(RUN_LOG_DIR)
}

/// The file holding the run's records, for a person or a test that looks at
/// the bytes; reading them goes through [`reader`]. Flow run logs never
/// compact, so the first generation is the whole run.
pub fn log_file(run_dir: &Path) -> PathBuf {
    daruda_runlog::FileBackend::new(log_dir(run_dir)).log_path(0)
}

/// Every node a run starts with, in the order a host draws them.
pub(crate) fn node_starts(loaded: &crate::load::LoadedFlow) -> Vec<machine::NodeStart> {
    let flow = loaded.flow();
    loaded
        .graph()
        .topological_order()
        .into_iter()
        .map(|id| {
            let produces = match flow.nodes.iter().find(|node| node.id == id) {
                Some(node) if crate::schedule::node_output(node, Path::new("")).is_some() => {
                    machine::Produces::File
                }
                _ => machine::Produces::Nothing,
            };
            machine::NodeStart { id, produces }
        })
        .collect()
}

#[cfg(test)]
pub(crate) mod testing;

#[cfg(test)]
mod tests;
