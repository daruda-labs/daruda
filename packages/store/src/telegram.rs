//! Read-only `telegram.json` migration input for the shared bot inbox.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TelegramState {
    /// Legacy next-fetch cursor: the largest processed `update_id` plus one.
    /// Inbox migration seeds the last claim with `update_offset - 1`.
    pub update_offset: i64,
}

/// `telegram.json` path under `data_dir`.
pub fn telegram_state_path_in(data_dir: &Path) -> PathBuf {
    data_dir.join("telegram.json")
}

/// Read only the canonical legacy input, never restoring recovery sidecars.
/// Inbox migration validates it strictly instead of using this default fallback.
pub fn load_telegram_state_in(data_dir: &Path) -> TelegramState {
    crate::inbox::migrate::read_legacy(&telegram_state_path_in(data_dir))
}
