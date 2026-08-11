//! What a lane's past flow runs were, read off disk.
//!
//! This is the first caller of `daruda_flow::marker::run_status`. v1 left
//! the function finished and unused, so a crashed run — the one state that
//! exists only as a *missing* marker — could be seen on disk and nowhere
//! else.
//!
//! Reading is not free: it lists a directory and stats a few files per run.
//! So the result is cached per lane and rebuilt only when something makes
//! it wrong, never per frame. [`FlowHistory::is_stale_for`] is where that
//! rule lives, and `Workspace::flow_history` is the one field holding it.

use std::path::{Path, PathBuf};

use daruda_flow::marker::RunStatus;
use daruda_store::project::LaneRef;
use gpui::SharedString;

/// One past run, in the shape the panel draws.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::workspace) struct FlowRunEntry {
    /// The run directory, for opening its `run.md`.
    pub dir: PathBuf,
    /// When it started, already worded. Empty when the directory's name is
    /// not this host's scheme — better a missing time than a wrong one.
    pub started: SharedString,
    /// The report to open on click, when there is one. Decided here rather
    /// than in the panel: this is the pass that is already stat-ing the
    /// directory, and the panel's is the render path.
    pub report: Option<PathBuf>,
    pub status: RunStatus,
}

/// One lane's history, and which lane it is. The pair is the whole cache:
/// a list without its lane would be shown beside the wrong runs after a
/// lane switch, which is the failure this type exists to make impossible.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::workspace) struct FlowHistory {
    lane: LaneRef,
    runs: Vec<FlowRunEntry>,
}

impl FlowHistory {
    /// Read a lane's runs, newest first.
    ///
    /// Sorted by directory name, which is chronological only because the
    /// host names runs with a leading fixed-width millisecond field — the
    /// same property `daruda_flow`'s retention sweep depends on.
    pub(in crate::workspace) fn read(lane: LaneRef, runs_dir: &Path) -> Self {
        let mut names: Vec<PathBuf> = std::fs::read_dir(runs_dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect();
        names.sort();
        names.reverse();

        let runs = names.into_iter().map(|dir| entry_for(&dir)).collect();
        Self { lane, runs }
    }

    /// Whether this cache answers for `lane`. The panel is lane-scoped, so
    /// a cache built for another one is not stale data — it is data about
    /// something else.
    pub(in crate::workspace) fn is_stale_for(&self, lane: LaneRef) -> bool {
        self.lane != lane
    }

    pub(in crate::workspace) fn runs(&self) -> &[FlowRunEntry] {
        &self.runs
    }
}

fn entry_for(dir: &Path) -> FlowRunEntry {
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    FlowRunEntry {
        started: super::flow_request::run_started_at(&name)
            .map(crate::surface::strings::flow_run_started_at)
            .unwrap_or_default()
            .into(),
        // `is_alive` is how a lock's pid is judged, and it is the same
        // predicate submission uses — a run this window is holding must
        // read as `Running`, not `Crashed`.
        status: daruda_flow::marker::run_status(dir, &super::flow_request::process_is_alive),
        report: report_in(dir),
        dir: dir.to_path_buf(),
    }
}

/// A run refused before the lock wrote no report, and opening a path that
/// is not there would replace the question with an unrelated complaint
/// about a missing file.
fn report_in(dir: &Path) -> Option<PathBuf> {
    let report = dir.join(daruda_flow::record::RUN_REPORT_FILE);
    report.is_file().then_some(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lane() -> LaneRef {
        LaneRef {
            project: 0,
            lane: 0,
        }
    }

    /// A run directory named the way the host names them, optionally with
    /// a completion marker.
    fn run_dir(runs: &Path, millis: u128, marker: Option<&str>) -> PathBuf {
        let dir = runs.join(super::super::flow_request::run_id(millis, 42, 1));
        std::fs::create_dir_all(&dir).expect("mkdir");
        if let Some(marker) = marker {
            std::fs::write(dir.join(marker), "").expect("marker");
        }
        dir
    }

    /// Every marker the engine writes, plus the two it cannot. `Crashed`
    /// is the row this whole section exists for — v1 could only see it on
    /// disk.
    #[test]
    fn each_marker_reads_back_as_its_own_status() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let runs = tmp.path();
        run_dir(runs, 4, Some("DONE"));
        run_dir(runs, 3, Some("FAILED"));
        run_dir(runs, 2, Some("CANCELED"));
        // No marker and no lock: nothing says what happened.
        run_dir(runs, 1, None);

        let statuses: Vec<RunStatus> = FlowHistory::read(lane(), runs)
            .runs()
            .iter()
            .map(|r| r.status)
            .collect();

        assert_eq!(
            statuses,
            vec![
                RunStatus::Done,
                RunStatus::Failed,
                RunStatus::Canceled,
                RunStatus::Unknown
            ],
            "newest first, one status each"
        );
    }

    /// The panel puts the most recent run at the top, and the only thing
    /// that makes name-order chronological is the id's leading clock.
    #[test]
    fn runs_come_back_newest_first() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let runs = tmp.path();
        for millis in [1_700_000_000_000u128, 1_700_000_001_000, 1_700_000_002_000] {
            run_dir(runs, millis, Some("DONE"));
        }

        let read = FlowHistory::read(lane(), runs);
        let times: Vec<&str> = read.runs().iter().map(|r| r.started.as_ref()).collect();
        let mut descending = times.clone();
        descending.sort();
        descending.reverse();
        assert_eq!(times, descending, "{times:?}");
    }

    /// A lane that never ran a flow has no runs directory at all, which is
    /// ordinary rather than an error.
    #[test]
    fn a_lane_that_never_ran_a_flow_reads_empty() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let read = FlowHistory::read(lane(), &tmp.path().join("never-created"));
        assert!(read.runs().is_empty());
    }

    /// Only a run that wrote a report is clickable. A run refused before
    /// the lock wrote none, and the panel offering it anyway would answer
    /// the click with a complaint about a missing file.
    #[test]
    fn only_a_run_with_a_report_offers_one() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let runs = tmp.path();
        let wrote = run_dir(runs, 2, Some("DONE"));
        std::fs::write(
            wrote.join(daruda_flow::record::RUN_REPORT_FILE),
            "# Ended passed\n",
        )
        .expect("report");
        run_dir(runs, 1, Some("FAILED"));

        let read = FlowHistory::read(lane(), runs);
        let reports: Vec<bool> = read.runs().iter().map(|r| r.report.is_some()).collect();
        assert_eq!(reports, vec![true, false]);
    }

    /// The cache is only an answer about the lane it was built for.
    /// Without this the panel would show one lane's runs under another's
    /// name after a switch.
    #[test]
    fn a_cache_built_for_one_lane_is_stale_for_another() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let read = FlowHistory::read(lane(), tmp.path());
        assert!(!read.is_stale_for(lane()));
        assert!(read.is_stale_for(LaneRef {
            project: 0,
            lane: 1
        }));
    }

    /// A directory this host did not name gets no time rather than a
    /// wrong one — the id layout is the only clock there is.
    #[test]
    fn a_directory_that_is_not_a_run_id_shows_no_time() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(tmp.path().join("scratch")).expect("mkdir");
        let read = FlowHistory::read(lane(), tmp.path());
        assert_eq!(read.runs().len(), 1);
        assert!(read.runs()[0].started.is_empty(), "invented a start time");
    }
}
