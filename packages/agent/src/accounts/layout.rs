//! Config-dir layout + Keychain service naming for managed accounts.
//! Pure/GPUI-free — no I/O, just path and identifier derivation.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use daruda_store::accounts::AccountId;

/// Root holding every managed account's isolated config dir, for every auth
/// domain. The `claude-accounts` name is retained for on-disk compatibility:
/// existing accounts live there and must keep working without a re-login.
pub fn accounts_root(data_dir: &Path) -> PathBuf {
    data_dir.join("claude-accounts")
}

/// The isolated config dir for one account, whatever variable its auth domain
/// carries it in (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`, …).
pub fn account_config_dir(data_dir: &Path, id: AccountId) -> PathBuf {
    accounts_root(data_dir).join(id.0.to_string())
}

/// Claude Code 2.1+ scopes the macOS Keychain credential item by
/// `sha256(CLAUDE_CONFIG_DIR)` (first 8 hex). Mirrors orca `keychain.ts:85`.
pub fn scoped_keychain_service(config_dir: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(config_dir.to_string_lossy().as_bytes());
    let digest = hasher.finalize();
    let suffix: String = digest.iter().take(4).map(|b| format!("{b:02x}")).collect();
    format!("Claude Code-credentials-{suffix}")
}
