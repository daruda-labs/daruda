//! Read-only remote-channel history migration inputs.

use daruda_store::remote_channels::*;

#[test]
fn history_survives_restart_in_its_own_data_directory() {
    let dir = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let bytes = b"{\"processed\":{\"work\":[\"event-1\"]}}\n";
    let file = dir.path().join("remote_channels.json");
    std::fs::write(&file, bytes).unwrap();
    let modified = std::fs::metadata(&file).unwrap().modified().unwrap();
    let state = RemoteChannelState::load_in(dir.path());
    assert!(state.contains("work", "event-1"));
    assert_eq!(RemoteChannelState::load_in(dir.path()), state);
    assert_eq!(
        RemoteChannelState::load_in(other.path()),
        RemoteChannelState::default()
    );
    assert_eq!(std::fs::read(&file).unwrap(), bytes);
    assert_eq!(
        std::fs::metadata(&file).unwrap().modified().unwrap(),
        modified
    );
}

#[test]
fn a_backup_does_not_recover_missing_or_corrupt_legacy_input() {
    for present in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("remote_channels.json");
        if present {
            std::fs::write(&file, b"{corrupt").unwrap();
        }
        std::fs::write(
            dir.path().join("remote_channels.json.bak"),
            b"{\"processed\":{\"work\":[\"event\"]}}",
        )
        .unwrap();
        let artifacts = || {
            let mut files: Vec<_> = std::fs::read_dir(dir.path())
                .unwrap()
                .map(|entry| {
                    let entry = entry.unwrap();
                    (
                        entry.file_name(),
                        std::fs::read(entry.path()).unwrap(),
                        entry.metadata().unwrap().modified().unwrap(),
                    )
                })
                .collect();
            files.sort_by(|left, right| left.0.cmp(&right.0));
            files
        };
        let before = artifacts();
        assert_eq!(
            RemoteChannelState::load_in(dir.path()),
            RemoteChannelState::default()
        );
        assert_eq!(artifacts(), before);
    }
}
