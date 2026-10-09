//! Claude Code's credential files on Linux and Windows.
use super::{AccountError, FetchError, PlanInfo, parse_credentials, read_credentials_file};
use std::path::Path;

pub fn read_system_credentials() -> Result<(String, PlanInfo), FetchError> {
    // The ambient login is the user's own; under test it reads as absent, as
    // the Keychain does on macOS.
    if cfg!(test) {
        return Err(FetchError::NoToken);
    }
    let path = credentials_path().ok_or(FetchError::NoToken)?;
    let raw = std::fs::read_to_string(path).map_err(|_| FetchError::NoToken)?;
    parse_credentials(&raw)
}

fn credentials_path() -> Option<std::path::PathBuf> {
    credentials_path_from(std::env::var_os("CLAUDE_CONFIG_DIR"), dirs::home_dir())
}

/// Pure core of [`credentials_path`], split out so the `$CLAUDE_CONFIG_DIR`
/// override and the `~/.claude` default can be unit-tested without mutating
/// the real process environment (parallel `cargo test` runs share one).
pub(super) fn credentials_path_from(
    config_dir: Option<std::ffi::OsString>,
    home: Option<std::path::PathBuf>,
) -> Option<std::path::PathBuf> {
    if let Some(dir) = config_dir {
        return Some(std::path::PathBuf::from(dir).join(".credentials.json"));
    }
    Some(home?.join(".claude").join(".credentials.json"))
}

pub fn read_scoped_credentials(config_dir: &Path) -> Result<(String, PlanInfo), AccountError> {
    read_credentials_file(config_dir)
}

/// Non-macOS no-op: the scoped credential lives in `.credentials.json`
/// inside `config_dir`, already removed by the caller's directory
/// cleanup (`std::fs::remove_dir_all`) — there is no separate OS
/// credential store to touch.
pub fn delete_scoped_credentials(_config_dir: &Path) {}

pub(super) fn system_credentials_digest() -> Option<String> {
    None
}
