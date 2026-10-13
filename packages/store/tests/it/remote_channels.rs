//! Remote-channel event deduplication state.

use daruda_store::remote_channels::*;

#[test]
fn history_survives_restart_in_its_own_data_directory() {
    let dir = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let mut state = RemoteChannelState::default();
    state.record("work", "event-1".into());
    state.save_in(dir.path()).unwrap();
    assert_eq!(RemoteChannelState::load_in(dir.path()), state);
    assert_eq!(
        RemoteChannelState::load_in(other.path()),
        RemoteChannelState::default()
    );
}
