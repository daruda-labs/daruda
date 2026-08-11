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

/// Flows that belong to the person rather than to a repository, usable
/// from any lane.
///
/// Profile-scoped, unlike the per-repository directory above: this is the
/// user's own data, the same kind `config.toml` is, and a debug build must
/// not read and write a release install's copy.
///
/// Derived from the data directory the window was **given** rather than
/// from a fresh `default_data_dir()`: production passes exactly that, and
/// a test passes a temp one — resolving it again here would read the
/// developer's own flows in a suite that thought it was isolated. Same
/// reasoning as `right_dock::task_ops::save_tasks_dirty`.
pub(in crate::workspace) fn global_flows_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(FLOWS_DIR)
}

/// Where a flow file was found. The picker shows it, because the two
/// places can hold the same name and running the other one is silent.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(in crate::workspace) enum FlowOrigin {
    /// Committed with the repository, so everyone opening it has it.
    Repo,
    /// This person's own, reachable from every lane.
    Global,
}

/// A flow file and where it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::workspace) struct FoundFlow {
    pub path: PathBuf,
    pub origin: FlowOrigin,
}

pub(in crate::workspace) fn runs_dir(lane_cwd: &Path) -> PathBuf {
    lane_cwd.join(REPO_DIR).join(RUNS_DIR)
}

/// Every flow this lane can run, by name, sorted — the repository's and
/// the person's own together.
///
/// A union rather than a fallback: "use the global ones only when the repo
/// has none" means authoring a single repo flow makes every global one
/// vanish from the picker, which is a cliff nobody asked for. The repo
/// still has the last word where it has an opinion — see [`merge_flows`].
/// `global_dir` is passed rather than resolved here: a caller holds it (see
/// `Workspace::global_flows_dir`), so what the picker offers is decided by
/// state a test can set and not by whatever the machine happens to have.
pub(in crate::workspace) fn list_flows(lane_cwd: &Path, global_dir: &Path) -> Vec<FoundFlow> {
    merge_flows(
        flow_files_in(&flows_dir(lane_cwd)),
        flow_files_in(global_dir),
    )
}

/// Repo first, then whatever global flow the repo has not already claimed
/// by name. Sorted by file name so the list reads alphabetically whatever
/// each entry's origin is — sorting by path would clump them by directory,
/// which is an ordering nobody is looking for.
fn merge_flows(repo: Vec<PathBuf>, global: Vec<PathBuf>) -> Vec<FoundFlow> {
    let claimed: Vec<_> = repo.iter().filter_map(|p| p.file_name()).collect();
    let mut found: Vec<FoundFlow> = repo
        .iter()
        .map(|path| FoundFlow {
            path: path.clone(),
            origin: FlowOrigin::Repo,
        })
        .chain(
            global
                .iter()
                .filter(|p| !p.file_name().is_some_and(|n| claimed.contains(&n)))
                .map(|path| FoundFlow {
                    path: path.clone(),
                    origin: FlowOrigin::Global,
                }),
        )
        .collect();
    found.sort_by(|a, b| a.path.file_name().cmp(&b.path.file_name()));
    found
}

/// A missing directory is an empty list rather than an error — a lane that
/// has never authored a flow is the common case, not a failure.
fn flow_files_in(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && has_flow_extension(p))
        .collect()
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

    /// A directory of flow files that is not a lane — the person's own.
    fn global_with(files: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        for name in files {
            std::fs::write(dir.path().join(name), "version: 1\n").expect("write flow");
        }
        dir
    }

    fn names(found: &[FoundFlow]) -> Vec<String> {
        found
            .iter()
            .map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect()
    }

    /// The picker lists what is on disk, sorted, which a `const` array
    /// cannot. Anything that is not a flow file stays out of the list.
    #[test]
    fn only_flow_files_are_listed_and_they_are_sorted() {
        let lane = lane_with(&["b.yaml", "a.yaml", "notes.md", "c.yml"]);
        let global = tempfile::tempdir().expect("tempdir");
        assert_eq!(
            names(&list_flows(lane.path(), global.path())),
            vec!["a.yaml", "b.yaml", "c.yml"]
        );
    }

    /// A lane that never authored a flow is ordinary, so the picker opens
    /// empty rather than reporting a missing directory.
    #[test]
    fn a_lane_without_a_flows_directory_lists_nothing() {
        let lane = tempfile::tempdir().expect("tempdir");
        let global = tempfile::tempdir().expect("tempdir");
        assert!(list_flows(lane.path(), global.path()).is_empty());
    }

    /// Both places at once, in one alphabetical list. A fallback would
    /// hide every global flow the moment the repo authored its first.
    #[test]
    fn a_lane_with_its_own_flows_still_reaches_the_global_ones() {
        let lane = lane_with(&["b.yaml"]);
        let global = global_with(&["a.yaml", "c.yaml"]);

        let found = list_flows(lane.path(), global.path());
        assert_eq!(names(&found), vec!["a.yaml", "b.yaml", "c.yaml"]);
        assert_eq!(
            found.iter().map(|f| f.origin).collect::<Vec<_>>(),
            vec![FlowOrigin::Global, FlowOrigin::Repo, FlowOrigin::Global]
        );
    }

    /// One name, one entry — the repository's. A flow committed with the
    /// code has to be the one that runs against that code, and listing
    /// both would leave which one ran up to which row was clicked.
    #[test]
    fn the_repository_s_own_shadows_a_global_flow_of_the_same_name() {
        let lane = lane_with(&["ship.yaml"]);
        let global = global_with(&["ship.yaml", "ship.yml"]);

        let found = list_flows(lane.path(), global.path());
        assert_eq!(names(&found), vec!["ship.yaml", "ship.yml"]);
        assert_eq!(found[0].origin, FlowOrigin::Repo);
        assert!(found[0].path.starts_with(lane.path()));
        // `.yml` is a different file name, so it is not shadowed.
        assert_eq!(found[1].origin, FlowOrigin::Global);
    }

    /// Only flow files are the person's, same as the repository's.
    #[test]
    fn a_note_in_the_global_directory_is_not_a_flow() {
        let lane = tempfile::tempdir().expect("tempdir");
        let global = global_with(&["a.yaml", "notes.md"]);
        assert_eq!(names(&list_flows(lane.path(), global.path())), ["a.yaml"]);
    }
}
