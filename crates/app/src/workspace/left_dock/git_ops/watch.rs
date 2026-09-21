//! Keeping the git caches current when something other than daruda writes.
//!
//! [`Workspace::spawn_locked_git_work`] covers every op daruda runs itself.
//! It cannot cover a commit typed into a terminal pane, an agent's `git
//! checkout`, or a fetch from another window — and those write only inside
//! the git dir, which the file-tree watcher drops by design. The watchers
//! wired here are the path by which that work reaches the UI.
//!
//! One watcher per git directory, not per lane: a repository's lanes share
//! one common dir, so a fetch writes there once no matter how many lanes
//! are open.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::system_info::redact_home;
use daruda_store::project::LaneRef;
use gpui::Context;

use crate::files::git_watcher::{GitDirEvent, GitDirSignal, GitDirWatcher};
use crate::workspace::Workspace;
use crate::workspace::lane_scoped::GitDirsState;

/// How often the git watchers are drained. Matches the file-tree poll: the
/// watchers already coalesce a burst into one signal, so this only bounds
/// how long a settled change waits to be seen.
const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(250);

impl Workspace {
    /// Hold exactly the watchers the open lanes need — no more, no fewer.
    ///
    /// Idempotent and cheap when nothing changed, which is what lets the
    /// poll below call it every tick instead of every lane lifecycle edge
    /// needing to remember to.
    pub(super) fn sync_git_watchers(&mut self, cx: &mut Context<Self>) {
        self.probe_missing_git_dirs(cx);

        let desired: HashSet<PathBuf> = self
            .lane_scoped
            .values()
            .filter_map(|state| match &state.git.dirs {
                GitDirsState::Known(dirs) => Some(dirs),
                _ => None,
            })
            .flat_map(|dirs| [dirs.git_dir.clone(), dirs.common_dir.clone()])
            .collect();

        self.git_watchers.retain(|dir, _| desired.contains(dir));
        self.git_watch_failures.retain(|dir| desired.contains(dir));
        for dir in desired {
            if self.git_watchers.contains_key(&dir) || self.git_watch_failures.contains(&dir) {
                continue;
            }
            match GitDirWatcher::new(dir.clone()) {
                Ok(watcher) => {
                    self.git_watchers.insert(dir, watcher);
                }
                Err(e) => {
                    // Losing a watcher costs freshness for outside writes
                    // only; daruda's own ops still refresh through the lock.
                    let report =
                        ErrorReport::new(crate::surface::strings::error_git_watcher_init_failed())
                            .severity(ErrorSeverity::Warning)
                            .from_error(&e)
                            .at(file!(), line!())
                            .with_context("path", redact_home(&dir))
                            .dedup("git.watcher_init")
                            .build();
                    self.report_error(report, cx);
                    // Do not retry every poll, but keep the successful sibling
                    // watch alive when only one of a lane's two dirs failed.
                    self.git_watch_failures.insert(dir);
                }
            }
        }

        if !self.git_watchers.is_empty() {
            self.ensure_git_watch_poll(cx);
        }
    }

    /// Locate the git dirs of every git lane that has not been probed yet.
    ///
    /// Walks the projects rather than `lane_scoped`, because a lane the user
    /// has not opened yet still shows an ahead/behind badge — and would
    /// otherwise never be watched.
    fn probe_missing_git_dirs(&mut self, cx: &mut Context<Self>) {
        let pending: Vec<(LaneRef, PathBuf)> = self
            .projects
            .iter()
            .flat_map(|project| {
                project.lanes.iter().map(move |lane| {
                    (
                        LaneRef {
                            project: project.id,
                            lane: lane.id,
                        },
                        lane,
                    )
                })
            })
            .filter(|(target, lane)| {
                lane.is_git()
                    && self
                        .lane_scoped
                        .get(target)
                        .is_none_or(|state| matches!(state.git.dirs, GitDirsState::Unknown))
            })
            .map(|(target, lane)| (target, lane.path.clone()))
            .collect();

        for (target, path) in pending {
            self.lane_scoped_mut(target).git.dirs = GitDirsState::Probing;
            crate::workspace::spawn_helpers::spawn_bg_work_and_mutate(
                cx,
                move || crate::lane::git::git_dirs(&path),
                move |ws, result, cx| {
                    let Some(state) = ws.lane_scoped.get_mut(&target) else {
                        return;
                    };
                    state.git.dirs = match result {
                        Ok(dirs) => GitDirsState::Known(dirs),
                        // Silent: a lane whose git dir cannot be located is
                        // already reporting through its status refresh.
                        Err(_) => GitDirsState::Unavailable,
                    };
                    ws.sync_git_watchers(cx);
                },
            )
            .detach();
        }
    }

    fn ensure_git_watch_poll(&mut self, cx: &mut Context<Self>) {
        if self.git_watch_poll.is_some() {
            return;
        }
        self.git_watch_poll = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL_INTERVAL).await;
                let alive = this
                    .update(cx, |ws, cx| {
                        ws.sync_git_watchers(cx);
                        ws.drain_git_watchers(cx);
                    })
                    .is_ok();
                if !alive {
                    break;
                }
            }
        }));
    }

    /// Route each watcher's coalesced signal to the lanes it speaks for.
    ///
    /// A lane whose *own* git dir this is takes the whole signal. A lane that
    /// merely shares it as its common dir takes the ref half only — its
    /// index and HEAD live in its own dir, watched separately.
    pub(in crate::workspace) fn drain_git_watchers(&mut self, cx: &mut Context<Self>) {
        let mut signals: HashMap<PathBuf, GitDirSignal> = HashMap::new();
        let mut errors = Vec::new();
        for (dir, watcher) in &self.git_watchers {
            while let Ok(event) = watcher.events_rx.try_recv() {
                match event {
                    GitDirEvent::Changed(signal) => {
                        signals.entry(dir.clone()).or_default().merge(signal);
                    }
                    GitDirEvent::Error(error) => errors.push((dir.clone(), error)),
                }
            }
        }
        for (dir, error) in errors {
            let report = ErrorReport::new(crate::surface::strings::error_git_watcher_error())
                .severity(ErrorSeverity::Warning)
                .with_context("path", redact_home(&dir))
                .with_context("error", error)
                .at(file!(), line!())
                .dedup("git.watcher_event")
                .build();
            self.report_error(report, cx);
        }

        for (dir, signal) in signals {
            let (mut own, mut shared) = (Vec::new(), Vec::new());
            for (target, state) in &self.lane_scoped {
                let GitDirsState::Known(dirs) = &state.git.dirs else {
                    continue;
                };
                if dirs.git_dir == dir {
                    own.push(*target);
                } else if dirs.common_dir == dir {
                    shared.push(*target);
                }
            }
            for target in own {
                if signal.refs {
                    self.refresh_tracking(target, cx);
                }
                if signal.worktree {
                    self.refresh_worktree_status(target, cx);
                }
            }
            if signal.refs {
                for target in shared {
                    self.refresh_tracking(target, cx);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;

    fn run_git(dir: &std::path::Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .current_dir(dir)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?} failed in {dir:?}");
    }

    /// The watch set is what decides whether an outside commit or fetch ever
    /// reaches the UI. A repo with a linked worktree is the shape that gets
    /// it wrong: watch only the lane dirs and a fetch is invisible, watch
    /// only the common dir and staging in the linked lane is.
    #[gpui::test]
    fn every_lane_is_watched_through_its_own_dir_and_the_shared_one(cx: &mut TestAppContext) {
        if !crate::lane::git::has_git() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_path_buf();
        run_git(&root, &["init", "-q"]);
        run_git(&root, &["config", "user.email", "daruda@test"]);
        run_git(&root, &["config", "user.name", "daruda"]);
        run_git(&root, &["commit", "-qm", "initial", "--allow-empty"]);
        let linked = root.join("side-wt");
        run_git(
            &root,
            &[
                "worktree",
                "add",
                "-q",
                linked.to_str().unwrap(),
                "-b",
                "side",
            ],
        );

        let (_wh, ws) = crate::workspace::tests::build_workspace_with(
            cx,
            &daruda_config::Config::default(),
            Some(daruda_store::project::Project::from_path(&root)),
        );
        ws.update(cx, |ws, cx| ws.reconcile_bootstrapped_lanes(cx));
        cx.run_until_parked();

        let target = ws.read_with(cx, |ws, _| ws.active);
        ws.update(cx, |ws, cx| ws.refresh_git_status(target, cx));
        cx.run_until_parked();

        ws.read_with(cx, |ws, _| {
            assert_eq!(
                ws.projects[0].lanes.len(),
                2,
                "fixture must expose both the main and the linked lane"
            );
            let mut watched: Vec<_> = ws.git_watchers.keys().cloned().collect();
            watched.sort();

            // The lanes carry canonical paths, so the expectations must too
            // — on macOS the raw tempdir is the `/var` alias of `/private/var`.
            let canonical = daruda_core::path::canonicalize(&root).unwrap();
            let common = crate::lane::git::git_dirs(&canonical).unwrap().common_dir;
            let side = crate::lane::git::git_dirs(&linked).unwrap().git_dir;
            let mut expected = vec![common, side];
            expected.sort();
            assert_eq!(
                watched, expected,
                "one watcher per git dir: the shared one plus the linked lane's own"
            );
        });
    }
}
