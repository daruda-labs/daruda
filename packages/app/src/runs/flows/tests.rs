use std::path::PathBuf;

use super::*;

fn run_in(run_dir: &str) -> FlowRun {
    let run_dir = PathBuf::from(run_dir);
    let (snapshot, pointers) = flow_metadata(
        &run_dir,
        &FlowSource::Resumed {
            run_dir: run_dir.clone(),
        },
        None,
    );
    FlowRun {
        snapshot,
        pointers,
        started: std::time::Instant::now(),
        view: RunView::new(run_dir.clone(), FlowSource::Resumed { run_dir }, Vec::new()),
        cancel: CancelToken::default(),
        overage: OverageStop::default(),
        submitted_by: daruda_flow::request::StartedBy::Desktop,
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

/// Wait on `key` until its end is delivered.
async fn delivered_end(
    key: &super::super::result::RunKey,
    wait: (super::super::watch::WaitId, smol::channel::Receiver<()>),
    cx: &mut gpui::TestAppContext,
) -> super::super::result::RunSnapshot {
    let (id, wake) = wait;
    loop {
        if let Some(answer) = cx.update(|cx| RunsHost::poll_run_wait(id, false, cx))
            && let Some(end) = answer.ended.into_iter().find(|end| &end.key == key)
        {
            cx.update(|cx| RunsHost::finish_run_wait(id, true, cx));
            return end;
        }
        wake.recv().await.expect("the waiter stays open");
    }
}

/// A waiter open while a flow ends receives the end with its `run.md`
/// text: the end is held until the report has been read, never announced
/// bare and enriched after a waiter may already have taken it.
#[gpui::test]
async fn a_waiter_open_when_a_flow_ends_receives_its_report(cx: &mut gpui::TestAppContext) {
    use super::super::result::{RunKey, RunState};
    use daruda_flow::request::{Budget, StartedBy};
    // A real run thread wakes the app from outside the scheduler.
    cx.executor().allow_parking();
    let dir = tempfile::tempdir().expect("tempdir");
    let (events_tx, events) = smol::channel::unbounded();
    let (asks_tx, asks) = smol::channel::unbounded();
    let run_dir = dir.path().join("flow-report");
    let request = RunRequest {
        started_by: Some(StartedBy::Orchestrator),
        loaded: daruda_flow::load(
            "version: 1\nnodes:\n  - id: check\n    kind: command\n    run: \"echo done\"\n",
            None,
        )
        .expect("the flow loads"),
        cwd: dir.path().into(),
        run_dir: run_dir.clone(),
        lock_dir: dir.path().join("locks"),
        flow_dir: dir.path().into(),
        agents: Default::default(),
        node_install_dir: dir.path().join("node"),
        budget: Budget::unlimited(),
        is_alive: Box::new(|_| false),
        git_status: None,
        events: Some(events_tx),
        ask: Some(asks_tx),
        until: None,
        pinned: vec![],
        resume: None,
    };
    let key = RunKey::flow_of(&run_dir);
    let wait = cx.update(|cx| {
        RunsHost::ensure_installed(cx);
        let wait = RunsHost::open_run_wait(Some(vec![key.clone()]), cx).expect("a waiter");
        let run_id = RunsHost::start_flow(
            FlowStart {
                tree: tree(dir.path()),
                request,
                source: FlowSource::File(dir.path().join("flow.yaml")),
                runners: Box::new(|request| Runners {
                    agent: daruda_flow::runner::AcpRunner::new(
                        request.agents.clone(),
                        request.node_install_dir.clone(),
                    ),
                    command: daruda_flow::runner::ProcessRunner::new(vec![]),
                }),
                overage: OverageStop::default(),
                submitted_by: StartedBy::Orchestrator,
                events,
                asks,
            },
            cx,
        )
        .expect("the worktree is free");
        assert_eq!(RunKey::Flow(run_id), key, "the id is the key results use");
        wait
    });

    let end = delivered_end(&key, wait, cx).await;
    let report = std::fs::read_to_string(run_dir.join(daruda_flow::record::RUN_REPORT_FILE))
        .expect("the run wrote its report");
    match end.state {
        RunState::Ended { text, .. } => assert_eq!(
            text,
            daruda_control_types::agent_text::bound_agent_text(&report),
            "the end reached the waiter without its report"
        ),
        other => panic!("not an end: {other:?}"),
    }
}

/// A report that cannot be read still releases the end, with the process's
/// own facts and no text: a waiter is never left holding a run that ended.
#[gpui::test]
async fn an_unreadable_report_still_releases_the_end(cx: &mut gpui::TestAppContext) {
    use super::super::result::{EndedAs, RunKey, RunState};
    cx.executor().allow_parking();
    let dir = tempfile::tempdir().expect("tempdir");
    // No run directory at all, so there is no report to read.
    let run_dir = dir.path().join("never-written");
    let key = RunKey::flow_of(&run_dir);
    let (events_tx, events) = smol::channel::unbounded();
    let (asks_tx, asks) = smol::channel::unbounded::<PendingAsk>();
    drop(asks_tx);
    let wait = cx.update(|cx| {
        RunsHost::ensure_installed(cx);
        RunsHost::seed_flow_run(
            dir.path().to_path_buf(),
            RunView::new(
                run_dir.clone(),
                FlowSource::Resumed {
                    run_dir: run_dir.clone(),
                },
                Vec::new(),
            ),
            cx,
        );
        let wait = RunsHost::open_run_wait(Some(vec![key.clone()]), cx).expect("a waiter");
        let worktree = tree(dir.path());
        let pumped = run_dir.clone();
        cx.spawn(async move |cx| pump(worktree, pumped, events, asks, cx).await)
            .detach();
        wait
    });
    events_tx
        .try_send(RunNotice::Unrecorded(RunEnd::Done))
        .expect("the pump listens");

    let end = delivered_end(&key, wait, cx).await;
    assert!(
        matches!(
            end.state,
            RunState::Ended {
                ended_as: EndedAs::Completed,
                text: None,
                ..
            }
        ),
        "released with the process's facts: {:?}",
        end.state
    );
}
