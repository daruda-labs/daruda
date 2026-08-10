//! Where a lane keeps its flows and its runs.
//!
//! `daruda_flow` documents this layout but does not own it — the engine
//! takes `run_dir` from whoever calls it, so the names live on the host
//! side. They live *here* rather than at each call site so there is one
//! place to change, and one entry on
//! `scripts/lint-daruda-path-literals.sh`'s whitelist.
//!
//! These are per-repository artifacts, deliberately **not** profile-scoped:
//! a flow is committed alongside the code it drives, and every profile
//! opening the same repo should see the same flows. That is the same
//! reasoning `task_edit_pane`'s `.daruda/task-*.md` files are whitelisted
//! under.

use std::path::{Path, PathBuf};

/// Per-repository daruda directory, checked in with the repo.
const REPO_DIR: &str = ".daruda";
/// Flow definitions the user authors and commits.
const FLOWS_DIR: &str = "flows";
/// One directory per run. `.gitignore`d by the engine on first run.
const RUNS_DIR: &str = "flow-runs";

/// Extensions a flow file may carry. Both are accepted because a file
/// named `.yml` that simply never appears in the picker is a worse
/// failure than one extra `ends_with`.
const FLOW_EXTENSIONS: [&str; 2] = ["yaml", "yml"];

pub(in crate::workspace) fn flows_dir(lane_cwd: &Path) -> PathBuf {
    lane_cwd.join(REPO_DIR).join(FLOWS_DIR)
}

pub(in crate::workspace) fn runs_dir(lane_cwd: &Path) -> PathBuf {
    lane_cwd.join(REPO_DIR).join(RUNS_DIR)
}

/// Every flow file in the lane, by name, sorted. A missing directory is
/// an empty list rather than an error — a lane that has never authored a
/// flow is the common case, not a failure.
pub(in crate::workspace) fn list_flows(lane_cwd: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(flows_dir(lane_cwd)) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && has_flow_extension(p))
        .collect();
    found.sort();
    found
}

fn has_flow_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| FLOW_EXTENSIONS.contains(&e))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lane_with(files: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        let flows = flows_dir(dir.path());
        std::fs::create_dir_all(&flows).expect("create flows dir");
        for name in files {
            std::fs::write(flows.join(name), "version: 1\n").expect("write flow");
        }
        dir
    }

    fn names(paths: &[PathBuf]) -> Vec<String> {
        paths
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect()
    }

    /// The picker lists what is on disk, sorted, which a `const` array
    /// cannot. Anything that is not a flow file stays out of the list.
    #[test]
    fn only_flow_files_are_listed_and_they_are_sorted() {
        let lane = lane_with(&["b.yaml", "a.yaml", "notes.md", "c.yml"]);
        assert_eq!(
            names(&list_flows(lane.path())),
            vec!["a.yaml", "b.yaml", "c.yml"]
        );
    }

    /// A lane that never authored a flow is ordinary, so the picker opens
    /// empty rather than reporting a missing directory.
    #[test]
    fn a_lane_without_a_flows_directory_lists_nothing() {
        let lane = tempfile::tempdir().expect("tempdir");
        assert!(list_flows(lane.path()).is_empty());
    }
}
