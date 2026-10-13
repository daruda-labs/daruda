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
//! and `FlowContext::history` is the one field holding it.

use std::path::{Path, PathBuf};

use daruda_flow::marker::RunStatus;
use daruda_store::project::LaneRef;
use gpui::{Context, SharedString};

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
    #[cfg(any(test, feature = "screenshot"))]
    pub(in crate::workspace) fn seeded(runs: Vec<FlowRunEntry>) -> Self {
        Self { runs }
    }

    /// Read a lane's runs, newest first.
    ///
    /// Sorted by directory name, which is chronological only because the
    /// host names runs with a leading fixed-width millisecond field — the
    /// same property `daruda_flow`'s retention sweep depends on.
    /// `lock_dir` is where this lane's lock lives — outside the tree, so
    /// there is nothing under `runs_dir` left to derive it from. Getting it
    /// wrong shows every live run as idle rather than `Running`.
    pub(in crate::workspace) fn read(runs_dir: &Path, lock_dir: Option<&Path>) -> Self {
        let mut names: Vec<PathBuf> = std::fs::read_dir(runs_dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect();
        names.sort();
        names.reverse();

        let runs = names
            .into_iter()
            .map(|dir| entry_for(&dir, lock_dir))
            .collect();
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
    /// The browsed worktree's history, read only while Flows is visible.
    /// `None` until a read off the main thread lands; the lane-keyed cache
    /// then holds it until its scope changes or a run transition
    /// invalidates it, and shows the old one while it is read again.
    pub(in crate::workspace) fn flow_history_for_panel(
        &mut self,
        cx: &Context<Self>,
    ) -> Option<FlowHistory> {
        if self.active_page() != Some(super::pages::Page::Flows) {
            return None;
        }
        let lane = self.flow_browser_lane();
        self.lane_for(lane)?;
        self.read_flow_history(lane, cx);
        self.flows.history.shown(lane).cloned()
    }

    /// List `lane`'s runs on the background executor and repaint once they
    /// land. A read already under way, or one overtaken by an invalidation,
    /// starts nothing and lands nothing.
    fn read_flow_history(&mut self, lane: LaneRef, cx: &Context<Self>) {
        let Some(cwd) = self.lane_for(lane).map(|l| l.path.clone()) else {
            return;
        };
        let Some(ticket) = self.flows.history.begin_read(lane) else {
            return;
        };
        let lock_dir = super::flow_paths::lane_lock_dir(&self.lock_root, &cwd);
        let runs_dir = super::flow_paths::runs_dir(&cwd);
        cx.spawn(async move |this, cx| {
            let read = cx
                .background_executor()
                .spawn(async move { FlowHistory::read(&runs_dir, lock_dir.as_deref()) })
                .await;
            // SILENT-OK: the window may close while the runs are listed
            let _ = this.update(cx, |ws, cx| {
                if ws.flows.history.finish_read(lane, ticket, read) {
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Mark the cached history wrong so the next snapshot reads disk again;
    /// the old one stays on screen until that read lands.
    /// Scoped to the lane it happened in — another lane's run says nothing
    /// about this lane's directory.
    pub(in crate::workspace) fn invalidate_flow_history(&mut self, lane: LaneRef) {
        self.flows.history.invalidate_for(lane);
    }
}

/// The newest run directory under `runs_dir`, by the same ordering the
/// history lists them in.
pub(in crate::workspace) fn newest_run_dir(runs_dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(runs_dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .max()
}

/// When the run in `dir` started, worded, from its directory name alone.
/// Empty when the name is not this host's scheme — better a missing time
/// than a wrong one.
pub(in crate::workspace) fn run_started_label(dir: &Path) -> SharedString {
    dir.file_name()
        .and_then(|name| super::flow_request::run_started_at(&name.to_string_lossy()))
        .map(crate::surface::strings::flow::run_started_at)
        .unwrap_or_default()
        .into()
}

fn entry_for(dir: &Path, lock_dir: Option<&Path>) -> FlowRunEntry {
    FlowRunEntry {
        started: run_started_label(dir),
        // `is_alive` is how a lock's pid is judged, and it is the same
        // predicate submission uses — a run this window is holding must
        // read as `Running`, not `Crashed`.
        // `None` is a lane whose path would not resolve, passed on as
        // "unknown" rather than as some path with no lock in it — which
        // would read the same as a free tree.
        status: daruda_flow::marker::run_status(
            dir,
            lock_dir,
            &super::flow_request::process_is_alive,
        ),
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

        let statuses: Vec<RunStatus> = FlowHistory::read(runs, Some(runs))
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

        let read = FlowHistory::read(runs, Some(runs));
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
        let read = FlowHistory::read(&tmp.path().join("never-created"), None);
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

        let read = FlowHistory::read(runs, Some(runs));
        let reports: Vec<bool> = read.runs().iter().map(|r| r.report.is_some()).collect();
        assert_eq!(reports, vec![true, false]);
    }

    /// A directory this host did not name gets no time rather than a
    /// wrong one — the id layout is the only clock there is.
    #[test]
    fn a_directory_that_is_not_a_run_id_shows_no_time() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(tmp.path().join("scratch")).expect("mkdir");
        let read = FlowHistory::read(tmp.path(), Some(tmp.path()));
        assert_eq!(read.runs().len(), 1);
        assert!(read.runs()[0].started.is_empty(), "invented a start time");
    }
}
