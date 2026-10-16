//! Public inbox claims survive restarts and bot handoffs without reply replay.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};

use daruda_runlog::CommitError;
use daruda_store::inbox::{
    ChannelKey, InboxId, InboxLease, InboxRejected, InboxSeed, InboxStore, migrate,
};

const BOT: &str = "0123456789abcdef0123456789abcdef";

fn open(root: &std::path::Path, seed: impl FnOnce() -> std::io::Result<InboxSeed>) -> InboxStore {
    let until = Instant::now() + Duration::from_secs(5);
    let lease = loop {
        if let Some(lease) = InboxLease::try_acquire(root, BOT).unwrap() {
            break lease;
        }
        assert!(
            Instant::now() < until,
            "previous writer did not release the bot"
        );
        std::thread::yield_now();
    };
    InboxStore::open(lease, "inbox-test", seed).unwrap()
}

fn close(store: InboxStore, root: &std::path::Path) {
    drop(store);
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        if InboxLease::try_acquire(root, BOT).unwrap().is_some() {
            return;
        }
        assert!(
            Instant::now() < until,
            "inbox did not drain before fixture cleanup"
        );
        std::thread::yield_now();
    }
}

#[test]
fn duplicate_telegram_updates_and_slack_events_execute_once() {
    let root = tempfile::tempdir().unwrap();
    let store = open(root.path(), || Ok(InboxSeed::default()));
    for (channel, id) in [
        ("telegram", InboxId::Monotonic(42)),
        ("slack", InboxId::Opaque("event".into())),
    ] {
        let effects = AtomicUsize::new(0);
        for _ in 0..2 {
            match smol::block_on(store.claim(ChannelKey::new(channel), id.clone())) {
                Ok(_) => {
                    effects.fetch_add(1, Ordering::Relaxed);
                }
                Err(CommitError::Rejected(InboxRejected::Duplicate { .. })) => {}
                other => panic!("unexpected claim {other:?}"),
            }
        }
        assert_eq!(effects.load(Ordering::Relaxed), 1);
    }
    close(store, root.path());
}

#[test]
fn a_claim_without_its_effect_is_not_executed_after_bot_handoff() {
    let root = tempfile::tempdir().unwrap();
    let first = open(root.path(), || Ok(InboxSeed::default()));
    smol::block_on(first.claim(ChannelKey::new("telegram"), InboxId::Monotonic(42))).unwrap();
    drop(first);
    let second = open(root.path(), || panic!("handoff must not reimport"));
    assert!(matches!(
        smol::block_on(second.claim(ChannelKey::new("telegram"), InboxId::Monotonic(42))),
        Err(CommitError::Rejected(InboxRejected::Duplicate { .. }))
    ));
    assert_eq!(
        second.snapshot().maximum(&ChannelKey::new("telegram")),
        Some(42)
    );
    close(second, root.path());
}

#[test]
fn initialization_runs_once_even_when_empty() {
    let root = tempfile::tempdir().unwrap();
    let initialized = Arc::new(AtomicUsize::new(0));
    let first = open(root.path(), || {
        initialized.fetch_add(1, Ordering::Relaxed);
        Ok(InboxSeed::default())
    });
    drop(first);
    let second = open(root.path(), || {
        initialized.fetch_add(1, Ordering::Relaxed);
        Ok(InboxSeed::default())
    });
    assert_eq!(initialized.load(Ordering::Relaxed), 1);
    close(second, root.path());
}

#[test]
fn the_lease_refuses_competitors_and_invalid_bot_paths() {
    let root = tempfile::tempdir().unwrap();
    let held = InboxLease::try_acquire(root.path(), BOT).unwrap().unwrap();
    assert!(InboxLease::try_acquire(root.path(), BOT).unwrap().is_none());
    for invalid in [
        "../escape",
        "/absolute",
        "ABCDEF0123456789abcdef0123456789ab",
        "credential:secret",
    ] {
        assert!(InboxLease::try_acquire(root.path(), invalid).is_err());
    }
    drop(held);
    assert!(InboxLease::try_acquire(root.path(), BOT).unwrap().is_some());
}

#[test]
fn legacy_inputs_import_once_without_changing_source_bytes_or_mtime() {
    let root = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let telegram = data.path().join("telegram.json");
    let remote = data.path().join("remote_channels.json");
    std::fs::write(&telegram, b"{\n  \"update_offset\": 42\n}\n").unwrap();
    std::fs::write(
        &remote,
        b"{\"processed\":{\"old-profile-uuid\":[\"event-1\",\"event-2\"]}}\n",
    )
    .unwrap();
    let original = [&telegram, &remote].map(|file| {
        (
            std::fs::read(file).unwrap(),
            std::fs::metadata(file).unwrap().modified().unwrap(),
        )
    });
    let first = open(root.path(), || {
        let mut seed = migrate::telegram_for_bot(data.path(), BOT, "inbox-test")?;
        seed.rings =
            migrate::remote(data.path(), "old-profile-uuid", ChannelKey::new("slack"))?.rings;
        Ok(seed)
    });
    assert_eq!(
        first.snapshot().maximum(&ChannelKey::new("telegram")),
        Some(41)
    );
    for (channel, id) in [
        ("telegram", InboxId::Monotonic(41)),
        ("slack", InboxId::Opaque("event-2".into())),
    ] {
        assert!(matches!(
            smol::block_on(first.claim(ChannelKey::new(channel), id)),
            Err(CommitError::Rejected(InboxRejected::Duplicate { .. }))
        ));
    }
    smol::block_on(first.claim(ChannelKey::new("telegram"), InboxId::Monotonic(42))).unwrap();
    drop(first);
    let second = open(root.path(), || {
        panic!("legacy input must not be imported twice")
    });
    assert_eq!(
        second.snapshot().maximum(&ChannelKey::new("telegram")),
        Some(42)
    );
    for (file, expected) in [&telegram, &remote].into_iter().zip(original) {
        assert_eq!(
            (
                std::fs::read(file).unwrap(),
                std::fs::metadata(file).unwrap().modified().unwrap()
            ),
            expected
        );
    }
    close(second, root.path());
}

#[test]
fn legacy_rings_use_the_same_bound_and_corrupt_inputs_fail_closed() {
    let data = tempfile::tempdir().unwrap();
    let events: Vec<_> = (0..=256).map(|id| id.to_string()).collect();
    let file = data.path().join("remote_channels.json");
    std::fs::write(
        &file,
        serde_json::to_vec(&serde_json::json!({"processed": {"old": events}})).unwrap(),
    )
    .unwrap();
    let seed = migrate::remote(data.path(), "old", ChannelKey::new("slack")).unwrap();
    let ring = &seed.rings[&ChannelKey::new("slack")];
    assert_eq!(ring.len(), 256);
    assert_eq!(ring.front().unwrap(), "1");
    std::fs::write(&file, b"{corrupt").unwrap();
    assert!(migrate::remote(data.path(), "old", ChannelKey::new("slack")).is_err());
}

#[test]
fn oversized_legacy_inputs_are_refused_before_parsing_without_modification() {
    let data = tempfile::tempdir().unwrap();
    for name in ["telegram.json", "remote_channels.json"] {
        let path = data.path().join(name);
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(64 * 1024 * 1024 + 1).unwrap();
        drop(file);
        let before = std::fs::metadata(&path).unwrap();
        let error = if name == "telegram.json" {
            migrate::telegram_for_bot(data.path(), BOT, "test").unwrap_err()
        } else {
            migrate::remote(data.path(), "old", ChannelKey::new("slack")).unwrap_err()
        };
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert_eq!(error.to_string(), "State record is too large");
        let after = std::fs::metadata(&path).unwrap();
        assert_eq!(after.len(), before.len());
        assert_eq!(after.modified().unwrap(), before.modified().unwrap());
        assert!(!data.path().join(format!("{name}.bak")).exists());
    }
}
