//! Watcher for a repository's git directory.
//!
//! The file-tree watcher deliberately drops everything under `.git/`: a
//! status read used to rewrite `.git/index`, and forwarding that write
//! would have re-triggered the read it came from. That left the ops which
//! touch *only* the git dir — commit, fetch, push, branch switch, whether
//! run by daruda or by a terminal beside it — with no way to reach the UI.
//!
//! This watcher covers exactly that gap. It is safe now because
//! `lane::git::git_command` pins `--no-optional-locks`, so daruda's own
//! reads no longer write; what remains is a precise ignore list for the
//! paths git churns without changing what daruda shows.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};

/// Coalescing window for git-dir events. Wider than the file tree's, because
/// one user action rewrites several files here (a commit moves `index`, a ref
/// and a log) and the refresh is worth doing once. Anchored on the first event
/// of a burst so a steady stream — a rebase, a big fetch — cannot starve it.
const GIT_DIR_DEBOUNCE_WINDOW: Duration = Duration::from_millis(1000);

/// Which cached axis a git-dir change invalidates. Both can be true: a
/// branch switch moves `HEAD`, which changes the tracked branch *and*
/// what the working tree is compared against.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GitDirSignal {
    /// Branch, upstream, ahead/behind.
    pub refs: bool,
    /// Staged / unstaged file sets.
    pub worktree: bool,
}

/// Debounced git-dir event delivered to the workspace.
pub enum GitDirEvent {
    Changed(GitDirSignal),
    Error(String),
}

impl GitDirSignal {
    const REFS: Self = Self {
        refs: true,
        worktree: false,
    };
    const WORKTREE: Self = Self {
        refs: false,
        worktree: true,
    };
    const BOTH: Self = Self {
        refs: true,
        worktree: true,
    };

    pub fn is_empty(self) -> bool {
        !self.refs && !self.worktree
    }

    pub fn merge(&mut self, other: Self) {
        self.refs |= other.refs;
        self.worktree |= other.worktree;
    }
}

/// Top-level git-dir directories whose churn never changes what daruda
/// shows: object storage (the bulk of fetch and gc traffic), the reflog,
/// the LFS cache, and the fsmonitor daemon's socket. `worktrees` is here
/// for a different reason — a linked lane's own watcher covers it, and
/// reading it again from the common dir would fan one lane's staging out
/// across every lane of the repository.
const IGNORED_DIRS: [&str; 5] = ["objects", "logs", "lfs", "fsmonitor--daemon", "worktrees"];

/// Git-dir files rewritten by ops whose real effect lands elsewhere:
/// `FETCH_HEAD` on every fetch (the ref updates go to `refs/`), and the
/// editor scratch files a commit leaves behind.
const IGNORED_FILES: [&str; 3] = ["FETCH_HEAD", "COMMIT_EDITMSG", "MERGE_MSG"];

/// What a git-dir-relative path implies for the caches, or `None` when it
/// implies nothing.
///
/// Unknown names return [`GitDirSignal::BOTH`] on purpose: a stale panel is
/// worse than a spare refresh, and git gains files faster than this list does.
pub fn classify_git_path(rel: &Path) -> Option<GitDirSignal> {
    let mut components = rel.components();
    let first = components.next()?.as_os_str().to_str()?;
    let is_nested = components.next().is_some();

    if IGNORED_DIRS.contains(&first) {
        return None;
    }
    if !is_nested {
        if IGNORED_FILES.contains(&first) {
            return None;
        }
        // `index.lock`, `HEAD.lock`, … — the write that matters lands when
        // the lock is replaced by the real file.
        if first.ends_with(".lock") {
            return None;
        }
    }

    Some(match first {
        "refs" | "packed-refs" => GitDirSignal::REFS,
        "index" => GitDirSignal::WORKTREE,
        // Conflict and replay state: the panel renders conflicts from it.
        "MERGE_HEAD" | "CHERRY_PICK_HEAD" | "REVERT_HEAD" | "rebase-merge" | "rebase-apply" => {
            GitDirSignal::WORKTREE
        }
        // `HEAD` moves on checkout and commit: a different branch and a
        // different baseline to diff the tree against.
        _ => GitDirSignal::BOTH,
    })
}

/// Owns the `notify::Watcher` over one git directory plus its debounce
/// thread. The watch stops when this struct is dropped.
pub struct GitDirWatcher {
    _watcher: RecommendedWatcher,
    /// Receiver of coalesced signals. The `Workspace` polling task drains
    /// this on each tick.
    pub events_rx: mpsc::Receiver<GitDirEvent>,
}

impl GitDirWatcher {
    pub fn new(git_dir: PathBuf) -> Result<Self, notify::Error> {
        let (raw_tx, raw_rx) = mpsc::channel::<Result<Event, String>>();
        let mut watcher: RecommendedWatcher =
            notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
                let event = res.map_err(|error| error.to_string());
                let _ = raw_tx.send(event);
            })?;
        watcher.watch(&git_dir, RecursiveMode::Recursive)?;

        let (out_tx, out_rx) = mpsc::channel::<GitDirEvent>();
        std::thread::spawn(move || debounce_loop(&git_dir, raw_rx, out_tx));
        Ok(Self {
            _watcher: watcher,
            events_rx: out_rx,
        })
    }
}

fn debounce_loop(
    git_dir: &Path,
    raw_rx: mpsc::Receiver<Result<Event, String>>,
    out_tx: mpsc::Sender<GitDirEvent>,
) {
    loop {
        // Block until the first event arrives — the thread parks here at
        // zero CPU while nothing touches the repository.
        let first = match raw_rx.recv() {
            Ok(Ok(event)) => event,
            Ok(Err(error)) => {
                let _ = out_tx.send(GitDirEvent::Error(error));
                continue;
            }
            Err(_) => return,
        };
        let mut signal = signal_for(git_dir, &first);
        let deadline = Instant::now() + GIT_DIR_DEBOUNCE_WINDOW;

        loop {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            match raw_rx.recv_timeout(deadline - now) {
                Ok(Ok(event)) => signal.merge(signal_for(git_dir, &event)),
                Ok(Err(error)) => {
                    flush_signal(&mut signal, &out_tx);
                    let _ = out_tx.send(GitDirEvent::Error(error));
                }
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    flush_signal(&mut signal, &out_tx);
                    return;
                }
            }
        }
        flush_signal(&mut signal, &out_tx);
    }
}

fn flush_signal(signal: &mut GitDirSignal, out_tx: &mpsc::Sender<GitDirEvent>) {
    if signal.is_empty() {
        return;
    }
    let _ = out_tx.send(GitDirEvent::Changed(*signal));
    *signal = GitDirSignal::default();
}

/// Merge every path in one event into a single signal. Paths outside
/// `git_dir` cannot be classified against it and are skipped.
fn signal_for(git_dir: &Path, ev: &Event) -> GitDirSignal {
    let mut signal = GitDirSignal::default();
    for path in &ev.paths {
        if let Ok(rel) = path.strip_prefix(git_dir)
            && let Some(one) = classify_git_path(rel)
        {
            signal.merge(one);
        }
    }
    signal
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classify(rel: &str) -> Option<GitDirSignal> {
        classify_git_path(Path::new(rel))
    }

    /// The table this watcher exists for. Getting a row wrong costs either a
    /// stale panel (dropping something that mattered) or the idle-CPU burn
    /// that made the old code drop `.git` wholesale (keeping churn).
    #[test]
    fn git_paths_classify_by_what_they_invalidate() {
        // Ref movement — fetch, push, branch create/delete.
        assert_eq!(
            classify("refs/remotes/origin/main"),
            Some(GitDirSignal::REFS)
        );
        assert_eq!(classify("refs/heads/side"), Some(GitDirSignal::REFS));
        assert_eq!(classify("packed-refs"), Some(GitDirSignal::REFS));

        // Staging and conflict state.
        assert_eq!(classify("index"), Some(GitDirSignal::WORKTREE));
        assert_eq!(classify("MERGE_HEAD"), Some(GitDirSignal::WORKTREE));
        assert_eq!(classify("rebase-merge/done"), Some(GitDirSignal::WORKTREE));

        // A checkout moves both.
        assert_eq!(classify("HEAD"), Some(GitDirSignal::BOTH));

        // Churn that changes nothing daruda shows.
        assert_eq!(classify("objects/ab/cdef0123"), None);
        assert_eq!(classify("logs/refs/heads/main"), None);
        assert_eq!(classify("lfs/objects/aa/bb"), None);
        assert_eq!(classify("fsmonitor--daemon/cookies/x"), None);
        // Seen from the common dir; the linked lane watches this itself.
        assert_eq!(classify("worktrees/side/index"), None);
        assert_eq!(classify("FETCH_HEAD"), None);
        assert_eq!(classify("COMMIT_EDITMSG"), None);
        assert_eq!(classify("index.lock"), None);
        assert_eq!(classify(""), None);

        // An unknown name refreshes rather than risks a stale panel.
        assert_eq!(classify("SOME_NEW_GIT_FILE"), Some(GitDirSignal::BOTH));
    }

    /// A fetch writes objects and a reflog entry beside the one ref update.
    /// The window has to collapse that to a single refs-only signal, or the
    /// object traffic alone would drive a refresh per file.
    #[test]
    fn a_fetch_shaped_burst_collapses_to_one_refs_signal() {
        let git_dir = Path::new("/repo/.git");
        let (raw_tx, raw_rx) = mpsc::channel::<Result<Event, String>>();
        let (out_tx, out_rx) = mpsc::channel::<GitDirEvent>();
        for path in [
            "/repo/.git/objects/ad/a51f69",
            "/repo/.git/objects/b7/bb7093",
            "/repo/.git/FETCH_HEAD",
            "/repo/.git/refs/remotes/origin/main",
            "/repo/.git/logs/refs/remotes/origin/main",
        ] {
            raw_tx
                .send(Ok(Event {
                    kind: notify::EventKind::Any,
                    paths: vec![PathBuf::from(path)],
                    attrs: notify::event::EventAttributes::new(),
                }))
                .unwrap();
        }
        drop(raw_tx);
        debounce_loop(git_dir, raw_rx, out_tx);

        assert!(matches!(
            out_rx.try_recv(),
            Ok(GitDirEvent::Changed(GitDirSignal::REFS))
        ));
        assert!(
            out_rx.try_recv().is_err(),
            "the burst must produce exactly one signal"
        );
    }

    /// A burst of pure noise must not wake the caches at all — this is the
    /// property whose absence pinned idle CPU before `.git` was dropped.
    #[test]
    fn a_pure_noise_burst_emits_nothing() {
        let git_dir = Path::new("/repo/.git");
        let (raw_tx, raw_rx) = mpsc::channel::<Result<Event, String>>();
        let (out_tx, out_rx) = mpsc::channel::<GitDirEvent>();
        for path in ["/repo/.git/objects/00/11", "/repo/.git/index.lock"] {
            raw_tx
                .send(Ok(Event {
                    kind: notify::EventKind::Any,
                    paths: vec![PathBuf::from(path)],
                    attrs: notify::event::EventAttributes::new(),
                }))
                .unwrap();
        }
        drop(raw_tx);
        debounce_loop(git_dir, raw_rx, out_tx);
        assert!(out_rx.try_recv().is_err());
    }

    #[test]
    fn watcher_errors_are_forwarded_after_pending_changes() {
        let git_dir = Path::new("/repo/.git");
        let (raw_tx, raw_rx) = mpsc::channel::<Result<Event, String>>();
        let (out_tx, out_rx) = mpsc::channel::<GitDirEvent>();
        raw_tx
            .send(Ok(Event {
                kind: notify::EventKind::Any,
                paths: vec![PathBuf::from("/repo/.git/HEAD")],
                attrs: notify::event::EventAttributes::new(),
            }))
            .unwrap();
        raw_tx.send(Err("watch failed".into())).unwrap();
        drop(raw_tx);
        debounce_loop(git_dir, raw_rx, out_tx);

        assert!(matches!(
            out_rx.try_recv(),
            Ok(GitDirEvent::Changed(GitDirSignal::BOTH))
        ));
        assert!(matches!(
            out_rx.try_recv(),
            Ok(GitDirEvent::Error(error)) if error == "watch failed"
        ));
    }
}
