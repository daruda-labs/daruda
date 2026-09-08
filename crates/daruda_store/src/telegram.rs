//! The Telegram bridge's cross-restart runtime state.
//!
//! Storage layout:
//! ```text
//! ~/.config/daruda/
//! └── telegram.json   # the getUpdates high-water mark — daruda is sole writer
//! ```
//!
//! Deliberately **not** in `config.toml`, even though the rest of the bridge's
//! settings are. Two reasons, either sufficient: this is a counter daruda
//! writes about once per inbound message, and `config.toml` is a file the user
//! hand-edits — a save from their editor would silently rewind the mark and
//! make Telegram re-deliver, and re-run, commands that already ran. Writing it
//! here also keeps the write off `SettingsStore`, whose every mutation notifies
//! app-wide observers that rebuild the native menu bar and refresh every window
//! (see the `cx.refresh_windows()` ban in the root `CLAUDE.md`, pitfall 10).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::persistence::{LoadOutcome, load_json_file, save_json_atomic};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TelegramState {
    /// Highest `update_id` already routed. Seeds the next `getUpdates` after a
    /// restart.
    ///
    /// The guarantee is at-least-once, not exactly-once: the mark is written
    /// after an update is acted on, so a crash in between re-delivers that one
    /// update. Bounding the replay to a single update is the point — the
    /// alternative, writing before acting, would silently drop a command.
    pub update_offset: i64,
}

/// `telegram.json` path under `data_dir`.
pub fn telegram_state_path_in(data_dir: &Path) -> PathBuf {
    data_dir.join("telegram.json")
}

/// Load the bridge's state. A missing or corrupt file starts from zero, which
/// is safe — Telegram simply re-sends whatever is still queued.
pub fn load_telegram_state_in(data_dir: &Path) -> TelegramState {
    match load_json_file::<TelegramState>("telegram", &telegram_state_path_in(data_dir)) {
        LoadOutcome::Parsed(state) => state,
        LoadOutcome::Missing | LoadOutcome::Corrupt => TelegramState::default(),
    }
}

/// Save the bridge's state atomically — same-FS tempfile + rename.
pub fn save_telegram_state_in(data_dir: &Path, state: &TelegramState) -> std::io::Result<()> {
    save_json_atomic(data_dir, &telegram_state_path_in(data_dir), state)
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
