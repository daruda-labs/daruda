//! Telegram bridge state persistence.

use daruda_store::telegram::*;

#[test]
fn an_absent_file_starts_from_zero() {
    let dir = std::env::temp_dir().join(format!("daruda_tg_absent_{}", std::process::id()));
    assert_eq!(load_telegram_state_in(&dir), TelegramState::default());
    assert_eq!(load_telegram_state_in(&dir).update_offset, 0);
}

#[test]
fn the_offset_survives_a_round_trip() {
    let dir = std::env::temp_dir().join(format!("daruda_tg_rt_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create dir");
    let state = TelegramState {
        update_offset: 4_242,
    };
    save_telegram_state_in(&dir, &state).expect("save");
    assert_eq!(load_telegram_state_in(&dir), state);
    std::fs::remove_dir_all(&dir).ok();
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
