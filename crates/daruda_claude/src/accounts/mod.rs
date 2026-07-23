//! Per-account runtime isolation for Claude: config-dir layout and
//! config-dir-scoped Keychain reads. Pure/GPUI-free.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::http::FetchError;
use crate::limits::{PlanInfo, parse_keychain_credentials};
use daruda_store::accounts::AccountId;

#[derive(Debug, thiserror::Error)]
pub enum AccountError {
    #[error("credential store read failed: {0}")]
    Credentials(#[from] FetchError),
    #[error("keychain command failed")]
    Keychain,
}

/// Root that holds every managed account's isolated config dir.
pub fn accounts_root(data_dir: &Path) -> PathBuf {
    data_dir.join("claude-accounts")
}

/// The `CLAUDE_CONFIG_DIR` for one account.
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

/// Read the OAuth token + plan for a specific account's config dir.
#[cfg(target_os = "macos")]
pub fn read_scoped_credentials(config_dir: &Path) -> Result<(String, PlanInfo), AccountError> {
    use std::process::Command;
    let service = scoped_keychain_service(config_dir);
    let out = Command::new("security")
        .args(["find-generic-password", "-s", &service, "-w"])
        .output()
        .map_err(|_| AccountError::Keychain)?;
    if !out.status.success() {
        return Err(AccountError::Credentials(FetchError::NoToken));
    }
    let raw = String::from_utf8(out.stdout).map_err(|_| AccountError::Keychain)?;
    Ok(parse_keychain_credentials(&raw)?)
}

#[cfg(not(target_os = "macos"))]
pub fn read_scoped_credentials(config_dir: &Path) -> Result<(String, PlanInfo), AccountError> {
    let path = config_dir.join(".credentials.json");
    let raw = std::fs::read_to_string(path)
        .map_err(|_| AccountError::Credentials(FetchError::NoToken))?;
    Ok(parse_keychain_credentials(&raw)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn scoped_service_name_is_stable_and_suffixed() {
        let a = scoped_keychain_service(Path::new("/tmp/acc/alice"));
        let a2 = scoped_keychain_service(Path::new("/tmp/acc/alice"));
        let b = scoped_keychain_service(Path::new("/tmp/acc/bob"));
        assert_eq!(a, a2, "same dir → same service name");
        assert_ne!(a, b, "different dir → different service name");
        assert!(a.starts_with("Claude Code-credentials-"));
        // sha256 앞 8 hex
        assert_eq!(a.rsplit('-').next().unwrap().len(), 8);
    }

    #[test]
    fn config_dir_is_under_data_dir_and_per_account() {
        let data = Path::new("/data");
        let id1 = daruda_store::accounts::AccountId::new();
        let id2 = daruda_store::accounts::AccountId::new();
        let d1 = account_config_dir(data, id1);
        let d2 = account_config_dir(data, id2);
        assert!(d1.starts_with("/data"));
        assert_ne!(d1, d2);
    }
}
