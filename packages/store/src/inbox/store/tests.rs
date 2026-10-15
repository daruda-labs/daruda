use super::*;
use std::sync::mpsc;

const BOT: &str = "0123456789abcdef0123456789abcdef";

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
    let store = InboxStore {
        owned: Some(OwnedInbox {
            committer,
            _lease: lease,
        }),
        snapshot,
    };
    let waiter = store.claim(ChannelKey::new("slack"), InboxId::Opaque("pending".into()));
    inside_hook.recv_timeout(Duration::from_secs(5)).unwrap();
    drop(waiter);
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
    release.send(()).unwrap();
    dropping.join().unwrap();
    assert!(
        nonblocking,
        "dropping an inbox must not join on the foreground thread"
    );
    let until = std::time::Instant::now() + Duration::from_secs(5);
    let lease = loop {
        if let Some(lease) = InboxLease::try_acquire(directory.path(), BOT).unwrap() {
            break lease;
        }
        assert!(std::time::Instant::now() < until);
        std::thread::yield_now();
    };
    let reopened = InboxStore::open(lease, "test", || panic!("already initialized")).unwrap();
    assert!(matches!(
        smol::block_on(reopened.claim(ChannelKey::new("slack"), InboxId::Opaque("pending".into()))),
        Err(CommitError::Rejected(InboxRejected::Duplicate { .. }))
    ));
}
