use super::*;

#[test]
fn maintenance_and_handlers_share_the_lock_until_its_owner_drops() {
    let dir = tempfile::tempdir().unwrap();
    let handler = SessionLock::acquire(dir.path(), "session").unwrap();
    assert!(
        SessionLock::try_acquire(dir.path(), "session")
            .unwrap()
            .is_none()
    );
    drop(handler);
    let maintenance = SessionLock::try_acquire(dir.path(), "session")
        .unwrap()
        .unwrap();
    assert!(
        SessionLock::try_acquire(dir.path(), "session")
            .unwrap()
            .is_none()
    );
    drop(maintenance);
    assert!(
        SessionLock::try_acquire(dir.path(), "session")
            .unwrap()
            .is_some()
    );
    assert!(super::super::lock_path_for(dir.path(), "session").exists());
}
