use super::*;
use daruda_store::inbox::{ChannelKey, InboxLease};

fn token(value: &str) -> TokenSnapshot {
    TokenSnapshot {
        token: value.into(),
        revision: keychain::revision(),
    }
}

fn release(root: &Path, credential: &str) {
    let lease = crate::test_support::once_released(
        || InboxLease::try_acquire(root, &lock::digest_of(credential)),
        |lease| matches!(lease, Ok(None)),
    )
    .unwrap()
    .unwrap();
    drop(lease);
}

#[test]
fn an_existing_shared_inbox_still_consumes_each_profiles_legacy_source_once() {
    let locks = tempfile::tempdir().unwrap();
    let first_profile = tempfile::tempdir().unwrap();
    let second_profile = tempfile::tempdir().unwrap();
    let source = second_profile.path().join("telegram.json");
    std::fs::write(&source, b"{\"update_offset\":200}").unwrap();
    let bytes = std::fs::read(&source).unwrap();
    let modified = std::fs::metadata(&source).unwrap().modified().unwrap();
    let first = open(locks.path(), first_profile.path(), token("bot-a")).unwrap();
    assert!(matches!(
        open(locks.path(), second_profile.path(), token("bot-a")).unwrap(),
        Ownership::HeldElsewhere
    ));
    drop(first);
    release(locks.path(), "bot-a");
    let Ownership::Ready(second) =
        open(locks.path(), second_profile.path(), token("bot-a")).unwrap()
    else {
        panic!("bot-a is free");
    };
    assert_eq!(
        second
            .inbox
            .snapshot()
            .maximum(&ChannelKey::new("telegram")),
        Some(-1)
    );
    drop(second);
    release(locks.path(), "bot-a");
    for _ in 0..2 {
        let Ownership::Ready(changed) =
            open(locks.path(), second_profile.path(), token("bot-b")).unwrap()
        else {
            panic!("bot-b is free");
        };
        assert_eq!(
            changed
                .inbox
                .snapshot()
                .maximum(&ChannelKey::new("telegram")),
            None
        );
        drop(changed);
        release(locks.path(), "bot-b");
    }
    assert_eq!(std::fs::read(&source).unwrap(), bytes);
    assert_eq!(
        std::fs::metadata(&source).unwrap().modified().unwrap(),
        modified
    );
}
