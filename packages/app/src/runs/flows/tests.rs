use std::path::PathBuf;

use super::*;

fn run_in(run_dir: &str) -> FlowRun {
    let run_dir = PathBuf::from(run_dir);
    FlowRun {
        view: RunView::new(run_dir.clone(), FlowSource::Resumed { run_dir }, Vec::new()),
        cancel: CancelToken::default(),
        overage: OverageStop::default(),
        caller: Caller::Desktop,
        finished: None,
    }
}

fn tree(path: &Path) -> CanonicalTree {
    CanonicalTree::resolve(path).expect("the folder resolves")
}

/// A worktree holds one run: a second never replaces the first, whose stop
/// switch would otherwise be lost with its entry.
#[test]
fn a_worktree_holds_one_run_at_a_time() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut runs = FlowRuns::default();
    assert_eq!(runs.insert(tree(dir.path()), run_in("/first")), Ok(()));
    assert_eq!(
        runs.insert(tree(dir.path()), run_in("/second")),
        Err(AlreadyRunning)
    );
    let held = runs.get(dir.path()).expect("the first run");
    assert_eq!(held.view.run_dir(), Path::new("/first"));
}

/// Any spelling of the worktree finds its run — the key as given, or a
/// path through a symlinked prefix — and a folder that is not it does not.
#[cfg(unix)]
#[test]
fn every_spelling_of_a_worktree_finds_its_run() {
    let dir = tempfile::tempdir().expect("tempdir");
    let links = tempfile::tempdir().expect("tempdir");
    let link = links.path().join("lane");
    std::os::unix::fs::symlink(dir.path(), &link).expect("symlink");
    let mut runs = FlowRuns::default();
    let key = tree(dir.path());
    runs.insert(key.clone(), run_in("/run")).expect("inserted");

    assert!(runs.get(key.as_path()).is_some());
    assert!(runs.get(dir.path()).is_some());
    assert!(runs.get(&link).is_some(), "the symlinked spelling missed");
    assert!(runs.get(links.path()).is_none());
    assert!(runs.remove(key.as_path()).is_some());
    assert!(runs.get(&link).is_none());
}

/// A quit waits for a run's thread with the time it has left and no longer:
/// a thread still going reads as not ended once the deadline passes, and one
/// that returned reads as ended at once.
#[test]
fn waiting_for_a_run_ends_at_its_thread_or_its_deadline() {
    let (finished_tx, finished) = std::sync::mpsc::channel();
    let run = FlowRun {
        finished: Some(finished),
        ..run_in("/run")
    };
    let now = std::time::Instant::now();
    assert!(!run.wait_ended(now + std::time::Duration::from_millis(20)));
    let thread = std::thread::spawn(move || drop(finished_tx));
    thread.join().expect("the thread returned");
    assert!(run.wait_ended(std::time::Instant::now()));
}
