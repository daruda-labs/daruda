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

    /// Claim `lock`, run `bg` on the background executor, release the lock, then
    /// run `on_result` on the foreground. A silent no-op when the lock is
    /// already held — that is the duplicate-click refusal.
    ///
    /// The release happens *before* `on_result`, which is where the pre-refactor
    /// code put it (`flag = false` was every continuation's first statement).
    /// Returns nothing on purpose: the task is detached here, so no caller can
    /// drop it and strand the lock held.
    pub(in crate::workspace) fn spawn_locked_git_work<R, F, G>(
        &mut self,
        lock: GitLock,
        cx: &mut Context<Self>,
        bg: F,
        on_result: G,
    ) where
        R: Send + 'static,
        F: FnOnce() -> R + Send + 'static,
        G: FnOnce(&mut Self, R, &mut Context<Self>) + 'static,
    {
        if self.git_lock_held(lock) {
            return;
        }
        self.set_git_lock(lock, true, cx);
        // The buttons' disabled state is part of this frame's dock snapshot.
        cx.notify();
        crate::workspace::spawn_helpers::spawn_bg_work_and_mutate(cx, bg, move |ws, result, cx| {
            ws.set_git_lock(lock, false, cx);
            on_result(ws, result, cx);
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use gpui::TestAppContext;

    use super::GitLock;

    /// The invariant the whole module exists for, in one pass: the lock is held
    /// for exactly one op, a second claim while it is held is refused outright
    /// (not queued), the release lands before the continuation runs, and the two
    /// locks do not interfere.
    #[gpui::test]
    fn one_op_holds_the_lock_and_releases_it_before_its_continuation(cx: &mut TestAppContext) {
        let (_wh, ws) = crate::workspace::tests::build_workspace(cx);

        // Each op adds its own weight, so the total names exactly which ran.
        let ran = Arc::new(AtomicUsize::new(0));
        // Recorded rather than asserted inside the continuation: a panic there
        // unwinds through a gpui task instead of failing the test.
        let released_first = Arc::new(AtomicUsize::new(0));

        ws.update(cx, |ws, cx| {
            let (ran_a, released) = (ran.clone(), released_first.clone());
            ws.spawn_locked_git_work(
                GitLock::Index,
                cx,
                || (),
                move |ws, (), _cx| {
                    if !ws.git_lock_held(GitLock::Index) {
                        released.store(1, Ordering::SeqCst);
                    }
                    ran_a.fetch_add(1, Ordering::SeqCst);
                },
            );
            assert!(
                ws.git_lock_held(GitLock::Index),
                "the claim stands until the op finishes"
            );
            assert!(
                !ws.git_lock_held(GitLock::Repo),
                "staging and repo locks are independent"
            );

            let ran_b = ran.clone();
            ws.spawn_locked_git_work(
                GitLock::Index,
                cx,
                || (),
                move |_, (), _| {
                    ran_b.fetch_add(10, Ordering::SeqCst);
                },
            );
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
}
