use super::*;
use std::sync::mpsc;

const BOT: &str = "0123456789abcdef0123456789abcdef";

/// Block until the writer behind `released` has let go of its lease, failing
/// rather than hanging when it never does.
fn wait_released(released: InboxReleased) {
    let (freed, notified) = mpsc::channel();
    std::thread::spawn(move || {
        smol::block_on(released.wait());
        // SILENT-OK: the receiver is gone only after the timeout already failed the test.
        let _ = freed.send(());
    });
    notified
        .recv_timeout(Duration::from_secs(5))
        .expect("the writer never released its lease");
}

#[test]
fn actual_write_failures_close_the_writer_and_replay_preserves_acknowledged_claims() {
    use daruda_runlog::testing::{Fault, FaultBackend};

    for fault in [
        Fault::TornAppend { k: 3, keep: 5 },
        Fault::FailSyncAfter { k: 3 },
    ] {
        let directory = tempfile::tempdir().unwrap();
        let lease = InboxLease::try_acquire(directory.path(), BOT)
            .unwrap()
            .unwrap();
        let backend = FaultBackend::new().with_fault(fault);
        let mut log = RunLog::<InboxMachine, _>::open(backend.clone(), "test").unwrap();
        log.commit(InboxEvent::Initialized {
            seed: InboxSeed::default(),
        })
        .unwrap();
        log.commit(InboxEvent::Claimed {
            channel: ChannelKey::new("telegram"),
            id: InboxId::Monotonic(41),
        })
        .unwrap();
        let snapshot = Arc::new(RwLock::new(log.state().clone()));
        let committer = Committer::spawn(log, "failing-inbox", Box::new(|_, _| {})).unwrap();
        let store = InboxStore::assemble(committer, lease, snapshot);
        assert!(matches!(
            smol::block_on(store.claim(ChannelKey::new("telegram"), InboxId::Monotonic(42))),
            Err(CommitError::Io(_))
        ));
        assert!(matches!(
            smol::block_on(store.claim(ChannelKey::new("telegram"), InboxId::Monotonic(43))),
            Err(CommitError::Closed)
        ));
        assert_eq!(
            store.snapshot().maximum(&ChannelKey::new("telegram")),
            Some(41)
        );
        assert!(
            InboxLease::try_acquire(directory.path(), BOT)
                .unwrap()
                .is_none()
        );
        let released = store.released();
        drop(store);
        wait_released(released);
        let _lease = InboxLease::try_acquire(directory.path(), BOT)
            .unwrap()
            .expect("a released writer frees its lease");
        let mut reopened = RunLog::<InboxMachine, _>::open(backend.restart(), "test").unwrap();
        assert!(matches!(
            reopened.commit(InboxEvent::Claimed {
                channel: ChannelKey::new("telegram"),
                id: InboxId::Monotonic(41)
            }),
            Err(CommitError::Rejected(InboxRejected::Duplicate { .. }))
        ));
        reopened
            .commit(InboxEvent::Claimed {
                channel: ChannelKey::new("telegram"),
                id: InboxId::Monotonic(43),
            })
            .unwrap();
    }
}

#[test]
fn dropping_a_canceled_waiter_keeps_the_lease_until_queued_commits_drain() {
    let directory = tempfile::tempdir().unwrap();
    let lease = InboxLease::try_acquire(directory.path(), BOT)
        .unwrap()
        .unwrap();
    daruda_core::path::create_owner_only_dir(lease.directory()).unwrap();
    let mut log =
        RunLog::<InboxMachine, _>::open(FileBackend::new(lease.directory()), "test").unwrap();
    log.commit(InboxEvent::Initialized {
        seed: InboxSeed::default(),
    })
    .unwrap();
    let snapshot = Arc::new(RwLock::new(log.state().clone()));
    let (entered, inside_hook) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let committer = Committer::spawn(
        log,
        "blocked-inbox",
        Box::new(move |_, _| {
            entered.send(()).unwrap();
            resume.recv().unwrap();
        }),
    )
    .unwrap();
    let store = InboxStore::assemble(committer, lease, snapshot);
    let waiter = store.claim(ChannelKey::new("slack"), InboxId::Opaque("pending".into()));
    inside_hook.recv_timeout(Duration::from_secs(5)).unwrap();
    drop(waiter);
    let released = store.released();
    let (dropped, notified) = mpsc::channel();
    let dropping = std::thread::spawn(move || {
        drop(store);
        dropped.send(()).unwrap();
    });
    let nonblocking = notified.recv_timeout(Duration::from_secs(1)).is_ok();
    assert!(
        InboxLease::try_acquire(directory.path(), BOT)
            .unwrap()
            .is_none()
    );
    assert!(!released.is_released());
    release.send(()).unwrap();
    dropping.join().unwrap();
    assert!(
        nonblocking,
        "dropping an inbox must not join on the foreground thread"
    );
    wait_released(released);
    let lease = InboxLease::try_acquire(directory.path(), BOT)
        .unwrap()
        .expect("a drained writer frees its lease");
    let reopened = InboxStore::open(lease, "test", || panic!("already initialized")).unwrap();
    assert!(matches!(
        smol::block_on(reopened.claim(ChannelKey::new("slack"), InboxId::Opaque("pending".into()))),
        Err(CommitError::Rejected(InboxRejected::Duplicate { .. }))
    ));
}
