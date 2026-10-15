//! Read-only Telegram cursor migration inputs.

use daruda_store::telegram::*;

#[test]
fn an_absent_file_starts_from_zero() {
    let dir = std::env::temp_dir().join(format!("daruda_tg_absent_{}", std::process::id()));
    assert_eq!(load_telegram_state_in(&dir), TelegramState::default());
    assert_eq!(load_telegram_state_in(&dir).update_offset, 0);
}

#[test]
fn a_legacy_next_fetch_cursor_is_read_without_rewriting_it() {
    let dir = tempfile::tempdir().unwrap();
    let state = TelegramState {
        update_offset: 4_242,
    };
    let file = telegram_state_path_in(dir.path());
    let bytes = b"{\"update_offset\":4242}\n";
    std::fs::write(&file, bytes).unwrap();
    let modified = std::fs::metadata(&file).unwrap().modified().unwrap();
    assert_eq!(load_telegram_state_in(dir.path()), state);
    assert_eq!(std::fs::read(&file).unwrap(), bytes);
    assert_eq!(
        std::fs::metadata(&file).unwrap().modified().unwrap(),
        modified
    );
}

/// A file written by a future daruda that grew a field must not reset the
/// offset — `#[serde(default)]` on the container is what keeps an unknown
/// key from failing the whole parse.
#[test]
fn an_unknown_key_does_not_discard_the_offset() {
    let dir = std::env::temp_dir().join(format!("daruda_tg_fwd_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create dir");
    std::fs::write(
        telegram_state_path_in(&dir),
        br#"{"update_offset":7,"something_new":true}"#,
    )
    .expect("write");
    assert_eq!(load_telegram_state_in(&dir).update_offset, 7);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_backup_does_not_recover_missing_or_corrupt_legacy_input() {
    for present in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let file = telegram_state_path_in(dir.path());
        if present {
            std::fs::write(&file, b"{corrupt").unwrap();
        }
        std::fs::write(
            dir.path().join("telegram.json.bak"),
            b"{\"update_offset\":42}",
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
        assert_eq!(load_telegram_state_in(dir.path()), TelegramState::default());
        assert_eq!(artifacts(), before);
    }
}
