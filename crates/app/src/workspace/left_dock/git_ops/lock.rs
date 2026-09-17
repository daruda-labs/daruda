//! The two git operation locks, and the only way to hold one.
//!
//! Every *mutating* git op is exclusive: a second click while one is running is
//! refused, not queued. Claim, release and spawn live together in
//! [`Workspace::spawn_locked_git_work`] so they cannot drift — a call path that
//! claimed by hand and then hit an early return before spawning would leave the
//! Git view's buttons inert for the rest of the run, with nothing left to
//! release the flag.
//!
//! Read-only refreshes are outside this: `git status` (`status`) and
//! `git log -1` (`history::enter_amend_mode`) run unlocked and guard themselves
//! where they need to, so holding a lock here never blocks the view catching up
//! with what a running op changed.

use daruda_store::project::LaneRef;
use gpui::Context;

use crate::workspace::Workspace;

/// Which class of git work a caller is claiming. Two locks rather than one
/// because a staging click and a fetch touch different git state, and the view
/// enables their buttons independently — sharing a lock would make either one
/// disable the other's affordance.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::workspace) enum GitLock {
    /// `git add` / `restore` / `clean` — the index and the working tree.
    Index,
    /// commit / amend / push / pull / fetch / `init` — the repository.
    Repo,
}

impl Workspace {
    /// Whether `lock` is held right now. For the callers that must refuse
    /// *before* doing something the claim cannot cover: every path that opens a
    /// confirm dialog, since its git work only starts a user decision later and
    /// a dialog for work that will be refused is worse than no dialog.
    pub(in crate::workspace) fn git_lock_held(&self, lock: GitLock) -> bool {
        match lock {
            GitLock::Index => self.git_stage_in_flight,
            GitLock::Repo => self.git_op_in_flight,
        }
    }

    fn set_git_lock(&mut self, lock: GitLock, held: bool, cx: &mut Context<Self>) {
        match lock {
            GitLock::Index => self.git_stage_in_flight = held,
            GitLock::Repo => {
                self.git_op_in_flight = held;
                // The commit button's disabled state mirrors this flag. Syncing
                // it here is what keeps the mirror to one update site — every
                // flip of the repo lock goes through this arm.
                self.sync_commit_buttons(cx);
            }
        }
    }

    /// Claim `lock`, run `bg` on the background executor, release the lock, run
    /// `on_result` on the foreground, then re-read whatever the op could have
    /// moved. A silent no-op when the lock is already held — that is the
    /// duplicate-click refusal.
    ///
    /// Invalidation rides on the claim rather than on each caller remembering:
    /// holding a lock *is* the declaration that this op writes to the
    /// repository, so which axes went stale follows from `lock` alone. It runs
    /// after `on_result` because an op can be what makes the lane git-backed in
    /// the first place (`git init`), and a refresh before that lands is a no-op.
    ///
    /// The release happens *before* `on_result`, which is where the pre-refactor
    /// code put it (`flag = false` was every continuation's first statement).
    /// Returns nothing on purpose: the task is detached here, so no caller can
    /// drop it and strand the lock held.
    pub(in crate::workspace) fn spawn_locked_git_work<T, E, F, G>(
        &mut self,
        lock: GitLock,
        target: LaneRef,
        cx: &mut Context<Self>,
        bg: F,
        on_result: G,
    ) where
        T: Send + 'static,
        E: Send + 'static,
        F: FnOnce() -> Result<T, E> + Send + 'static,
        G: FnOnce(&mut Self, Result<T, E>, &mut Context<Self>) + 'static,
    {
        if self.git_lock_held(lock) {
            return;
        }
        self.set_git_lock(lock, true, cx);
        // The buttons' disabled state is part of this frame's dock snapshot.
        cx.notify();
        crate::workspace::spawn_helpers::spawn_bg_work_and_mutate(cx, bg, move |ws, result, cx| {
            ws.set_git_lock(lock, false, cx);
            let succeeded = result.is_ok();
            on_result(ws, result, cx);
            if succeeded {
                ws.invalidate_after_git_work(lock, target, cx);
            }
        })
        .detach();
    }

    /// Re-read the axes `lock` covers. An index op touches one lane's working
    /// tree; a repo op moves refs the whole repository shares, so every lane of
    /// it re-reads its tracking info — that is what keeps a sibling lane's
    /// ahead/behind badge honest after a fetch.
    fn invalidate_after_git_work(
        &mut self,
        lock: GitLock,
        target: LaneRef,
        cx: &mut Context<Self>,
    ) {
        match lock {
            GitLock::Index => self.refresh_worktree_status(target, cx),
            GitLock::Repo => {
                self.refresh_tracking_across_repo(target.project, cx);
                self.refresh_worktree_status(target, cx);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use daruda_store::project::LaneRef;
    use gpui::TestAppContext;

    use super::GitLock;
    use crate::lane::git::GitError;

    /// Every locked op reports success or failure, so the helpers below can
    /// stand in for real git work without hiding the shape the lock depends on.
    fn ok() -> Result<(), GitError> {
        Ok(())
    }

    fn failed() -> Result<(), GitError> {
        Err(GitError::NotFound)
    }

    /// The invariant the whole module exists for, in one pass: the lock is held
    /// for exactly one op, a second claim while it is held is refused outright
    /// (not queued), the release lands before the continuation runs, and the two
    /// locks do not interfere.
    #[gpui::test]
    fn one_op_holds_the_lock_and_releases_it_before_its_continuation(cx: &mut TestAppContext) {
        let (_wh, ws) = crate::workspace::tests::build_workspace(cx);
        let target = ws.read_with(cx, |ws, _| ws.active);

        // Each op adds its own weight, so the total names exactly which ran.
        let ran = Arc::new(AtomicUsize::new(0));
        // Recorded rather than asserted inside the continuation: a panic there
        // unwinds through a gpui task instead of failing the test.
        let released_first = Arc::new(AtomicUsize::new(0));

        ws.update(cx, |ws, cx| {
            let (ran_a, released) = (ran.clone(), released_first.clone());
            ws.spawn_locked_git_work(GitLock::Index, target, cx, ok, move |ws, _, _cx| {
                if !ws.git_lock_held(GitLock::Index) {
                    released.store(1, Ordering::SeqCst);
                }
                ran_a.fetch_add(1, Ordering::SeqCst);
            });
            assert!(
                ws.git_lock_held(GitLock::Index),
                "the claim stands until the op finishes"
            );
            assert!(
                !ws.git_lock_held(GitLock::Repo),
                "staging and repo locks are independent"
            );

            let ran_b = ran.clone();
            ws.spawn_locked_git_work(GitLock::Index, target, cx, ok, move |_, _, _| {
                ran_b.fetch_add(10, Ordering::SeqCst);
            });
        });
        cx.run_until_parked();

        assert_eq!(
            ran.load(Ordering::SeqCst),
            1,
            "the second claim must be refused, not queued behind the first"
        );
        assert_eq!(
            released_first.load(Ordering::SeqCst),
            1,
            "the continuation must be free to start the next op"
        );
        ws.read_with(cx, |ws, _| {
            assert!(!ws.git_lock_held(GitLock::Index), "the lock cannot leak");
        });
    }

    /// A workspace over a real repo with a second lane pointed at the same
    /// checkout — enough for both lanes to answer a tracking read, which is
    /// what the repo lock's fan-out has to reach.
    fn build_two_lane_git_workspace(
        cx: &mut TestAppContext,
    ) -> (gpui::Entity<crate::workspace::Workspace>, LaneRef, LaneRef) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_path_buf();
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.email", "daruda@test"],
            vec!["config", "user.name", "daruda"],
            vec!["commit", "-qm", "initial", "--allow-empty"],
        ] {
            assert!(
                std::process::Command::new("git")
                    .current_dir(&root)
                    .args(&args)
                    .status()
                    .unwrap()
                    .success()
            );
        }
        // The tempdir must outlive the workspace; leaking the handle keeps the
        // checkout alive for the whole test.
        std::mem::forget(temp);

        let (_wh, ws) = crate::workspace::tests::build_workspace_with(
            cx,
            &daruda_config::Config::default(),
            Some(daruda_store::project::Project::from_path(&root)),
        );
        ws.update(cx, |ws, cx| ws.reconcile_bootstrapped_lanes(cx));
        cx.run_until_parked();

        let (first, second) = ws.update(cx, |ws, _| {
            let first = ws.active;
            let project = ws.active_project_mut().unwrap();
            let lane = project.lanes.iter().map(|lane| lane.id).max().unwrap() + 1;
            let path = project.lanes[0].path.clone();
            project.lanes.push(crate::lane::Lane::git(
                lane,
                path.clone(),
                Some("main".into()),
                path.clone(),
                path,
                1,
            ));
            (
                first,
                LaneRef {
                    project: project.id,
                    lane,
                },
            )
        });
        (ws, first, second)
    }

    /// What the lock claim buys: the caller never re-reads git state by hand,
    /// and the reach follows from which lock it took. A repo op moves refs the
    /// whole repository shares, so a sibling lane's tracking has to come back
    /// fresh too — that is the ahead/behind badge nobody was updating.
    #[gpui::test]
    fn a_repo_op_refreshes_every_lane_and_an_index_op_only_its_own(cx: &mut TestAppContext) {
        if !crate::lane::git::has_git() {
            return;
        }
        let (ws, first, second) = build_two_lane_git_workspace(cx);
        ws.update(cx, |ws, _| {
            for target in [first, second] {
                let git = &mut ws.lane_scoped_mut(target).git;
                git.tracking = None;
                git.worktree = None;
            }
        });

        ws.update(cx, |ws, cx| {
            ws.spawn_locked_git_work(GitLock::Index, first, cx, ok, |_, _, _| {});
        });
        cx.run_until_parked();
        ws.read_with(cx, |ws, _| {
            assert!(
                ws.lane_git_worktree(first).is_some(),
                "an index op must re-read the tree it staged into"
            );
            assert!(
                ws.lane_scoped[&first].git.tracking.is_none(),
                "an index op moves no ref, so tracking must stay untouched"
            );
            assert!(
                ws.lane_git_worktree(second).is_none(),
                "an index op must not reach a sibling lane"
            );
        });

        ws.update(cx, |ws, cx| {
            ws.spawn_locked_git_work(GitLock::Repo, first, cx, ok, |_, _, _| {});
        });
        cx.run_until_parked();
        ws.read_with(cx, |ws, _| {
            for target in [first, second] {
                assert!(
                    ws.lane_scoped[&target].git.tracking.is_some(),
                    "a repo op moves refs every lane shares"
                );
            }
        });
    }

    /// A failed op leaves the repository as it was, so re-reading it would only
    /// spend a subprocess to learn nothing.
    #[gpui::test]
    fn a_failed_op_invalidates_nothing(cx: &mut TestAppContext) {
        if !crate::lane::git::has_git() {
            return;
        }
        let (ws, first, _second) = build_two_lane_git_workspace(cx);
        ws.update(cx, |ws, _| {
            let git = &mut ws.lane_scoped_mut(first).git;
            git.tracking = None;
            git.worktree = None;
        });

        ws.update(cx, |ws, cx| {
            ws.spawn_locked_git_work(GitLock::Repo, first, cx, failed, |_, _, _| {});
        });
        cx.run_until_parked();
        ws.read_with(cx, |ws, _| {
            let git = &ws.lane_scoped[&first].git;
            assert!(git.tracking.is_none() && git.worktree.is_none());
            assert!(!ws.git_lock_held(GitLock::Repo), "the lock still releases");
        });
    }
}
