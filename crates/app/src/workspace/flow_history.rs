//! What a lane's past flow runs were, read off disk, and the two entry
//! points the workspace reaches it through.
//!
//! Disk is the only witness. `daruda_flow::marker::run_status` is what
//! reads it, because a crashed run exists solely as a *missing* completion
//! marker — a status derived from what this process still remembers would
//! report it as never having happened.
//!
//! Reading is not free: it lists a directory and stats a few files per run.
//! So the result is cached per lane and rebuilt only when something makes
//! it wrong, never per frame. [`super::flow_cache::LaneCache::get`] is
//! where that rule lives — it answers only for the lane it was read for —
//! and `Workspace::flow_history` is the one field holding it.

use std::path::{Path, PathBuf};

use daruda_flow::marker::RunStatus;
use daruda_store::project::LaneRef;
use gpui::SharedString;

use super::Workspace;

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

/// One lane's runs, newest first.
///
/// Which lane is [`super::flow_cache::LaneCache`]'s to answer: it holds the
/// pair, so a list read for one lane cannot be handed out for another.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::workspace) struct FlowHistory {
    runs: Vec<FlowRunEntry>,
}

impl FlowHistory {
    /// Read a lane's runs, newest first.
    ///
    /// Sorted by directory name, which is chronological only because the
    /// host names runs with a leading fixed-width millisecond field — the
    /// same property `daruda_flow`'s retention sweep depends on.
    pub(in crate::workspace) fn read(runs_dir: &Path) -> Self {
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
        Self { runs }
    }

    /// A history for `--screenshot`. A run that was killed only exists as a
    /// particular arrangement of files, and a capture cannot make one
    /// without writing into whichever repository happens to be open.
    #[cfg(feature = "screenshot")]
    pub(in crate::workspace) fn for_shot(dir: PathBuf) -> Self {
        Self {
            runs: vec![
                FlowRunEntry {
                    dir: dir.join("0000019fee7d80b8-00005ad5-0003"),
                    started: "08-11 14:36".into(),
                    status: RunStatus::Crashed,
                    report: None,
                },
                FlowRunEntry {
                    dir: dir.join("0000019fee7ccf03-00005ad5-0002"),
                    started: "08-11 10:49".into(),
                    status: RunStatus::Done,
                    report: None,
                },
            ],
        }
    }

    pub(in crate::workspace) fn runs(&self) -> &[FlowRunEntry] {
        &self.runs
    }
}

impl Workspace {
    /// The active lane's past runs, reading them if the cache cannot
    /// answer. The **one** place the history is built.
    ///
    /// Derived here rather than pushed from each transition: the active
    /// lane changes at five call sites (activate, project add / close /
    /// rename, restore), and a refresh hook on each is a set the next one
    /// forgets to join. Asking the cache whose lane it holds cannot be
    /// forgotten.
    ///
    /// Reads disk only when the Flows tab is showing and the cache is
    /// absent or built for another lane — so a tab the user is not on
    /// costs nothing, and the tab they are on costs one listing per
    /// change rather than one per frame.
    pub(in crate::workspace) fn flow_history_for_panel(&mut self) -> Option<FlowHistory> {
        if self.right_dock_view != daruda_store::project::RightDockView::Flows {
            return None;
        }
        let lane = self.active;
        if self.flow_history.get(lane).is_none() {
            let cwd = self.active_lane_root()?;
            let read = FlowHistory::read(&super::flow_paths::runs_dir(&cwd));
            self.flow_history.put(lane, read);
        }
        self.flow_history.get(lane).cloned()
    }

    /// Drop the cached history so the next snapshot reads disk again.
    /// Scoped to the lane it happened in — another lane's run says nothing
    /// about this lane's directory.
    pub(in crate::workspace) fn invalidate_flow_history(&mut self, lane: LaneRef) {
        self.flow_history.invalidate_for(lane);
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

/// The report a run directory holds, if it wrote one. Stat'ed rather than
/// taken on trust: opening a path that is not there would replace whatever
/// the caller had to say with an unrelated complaint about a missing file.
pub(super) fn report_in(dir: &Path) -> Option<PathBuf> {
    let report = dir.join(daruda_flow::record::RUN_REPORT_FILE);
    report.is_file().then_some(report)
}

#[cfg(test)]
mod tests {
    use super::*;

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

        let statuses: Vec<RunStatus> = FlowHistory::read(runs)
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

        let read = FlowHistory::read(runs);
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
        let read = FlowHistory::read(&tmp.path().join("never-created"));
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

        let read = FlowHistory::read(runs);
        let reports: Vec<bool> = read.runs().iter().map(|r| r.report.is_some()).collect();
        assert_eq!(reports, vec![true, false]);
    }

    /// A directory this host did not name gets no time rather than a
    /// wrong one — the id layout is the only clock there is.
    #[test]
    fn a_directory_that_is_not_a_run_id_shows_no_time() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(tmp.path().join("scratch")).expect("mkdir");
        let read = FlowHistory::read(tmp.path());
        assert_eq!(read.runs().len(), 1);
        assert!(read.runs()[0].started.is_empty(), "invented a start time");
    }
}
