//! Claude Code's macOS Keychain entries and file fallback.
use super::{AccountError, FetchError, PlanInfo, parse_credentials, read_credentials_file};
use crate::accounts::layout::scoped_keychain_service;
use std::path::Path;

/// Keychain service holding the ambient Claude login — the entry the CLI
/// writes when no per-account dir scopes it. Shared by every daruda profile
/// and by the user's own terminal usage.
const SYSTEM_KEYCHAIN_SERVICE: &str = "Claude Code-credentials";

/// System-wide Claude Code login: the macOS Keychain item, or the
/// `.credentials.json` file (`$CLAUDE_CONFIG_DIR`-aware) elsewhere. Every
/// failure — no login yet, non-macOS build, malformed contents — collapses to
/// [`FetchError::NoToken`] so callers render one "unavailable" state.
pub fn read_system_credentials() -> Result<(String, PlanInfo), FetchError> {
    let KeychainLookup::Found(secret) = find_keychain_secret(SYSTEM_KEYCHAIN_SERVICE) else {
        return Err(FetchError::NoToken);
    };
    let raw = String::from_utf8(secret).map_err(|_| FetchError::NoToken)?;
    parse_credentials(&raw)
}

/// What one Keychain lookup found.
pub(super) enum KeychainLookup {
    Found(Vec<u8>),
    Missing,
    /// `security` itself could not run.
    Failed,
}

/// The one place this module reads the Keychain. A test never reaches the
/// user's login keychain: under test the store answers as an empty one.
pub(super) fn find_keychain_secret(service: &str) -> KeychainLookup {
    if cfg!(test) {
        return KeychainLookup::Missing;
    }
    match std::process::Command::new("security")
        .args(["find-generic-password", "-s", service, "-w"])
        .output()
    {
        Ok(out) if out.status.success() => KeychainLookup::Found(out.stdout),
        Ok(_) => KeychainLookup::Missing,
        Err(_) => KeychainLookup::Failed,
    }
}

/// Read the OAuth token + plan for a specific account's config dir.
///
/// On macOS the Keychain is where the CLI normally puts them, but not the only
/// place it ever has: a build that writes `.credentials.json` into the config
/// dir instead would otherwise read as "no credentials" — and a *successful*
/// login reported that way is not merely a wrong label, it makes the add flow
/// discard the directory it just created. Trying the file after the Keychain
/// costs one `stat` on the normal path.
pub fn read_scoped_credentials(config_dir: &Path) -> Result<(String, PlanInfo), AccountError> {
    match read_scoped_keychain_credentials(config_dir) {
        Ok(found) => Ok(found),
        Err(keychain_error) => read_credentials_file(config_dir).map_err(|_| keychain_error),
    }
}

fn read_scoped_keychain_credentials(config_dir: &Path) -> Result<(String, PlanInfo), AccountError> {
    let secret = match find_keychain_secret(&scoped_keychain_service(config_dir)) {
        KeychainLookup::Found(secret) => secret,
        KeychainLookup::Missing => return Err(AccountError::Credentials(FetchError::NoToken)),
        KeychainLookup::Failed => return Err(AccountError::Keychain),
    };
    let raw = String::from_utf8(secret).map_err(|_| AccountError::Keychain)?;
    Ok(parse_credentials(&raw)?)
}

/// Best-effort delete of the scoped macOS Keychain item a Claude Code
/// login writes into `config_dir`'s isolated `CLAUDE_CONFIG_DIR` (see
/// [`crate::accounts::layout::scoped_keychain_service`]). Called by every `app`-side cleanup path
/// that discards a login attempt without keeping it (dedup hit, denied,
/// timed out, failed, or cancelled) so a discarded login never leaves an
/// orphaned OS credential behind. Mirrors the `security
/// delete-generic-password` invocation in
/// `app/src/telegram/keychain.rs::delete_token`. "Item not found" is the
/// expected common case (the login never got far enough to write
/// credentials) and is silently ignored; any other failure is logged,
/// not surfaced — same "no functional impact" call as the config-dir
/// removal this runs alongside.
pub fn delete_scoped_credentials(config_dir: &Path) {
    use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
    use daruda_store::observability::log_writer::LogWriter;
    use std::process::{Command, Stdio};

    // Same rule as `find_keychain_secret`: a test never touches the keychain.
    if cfg!(test) {
        return;
    }
    let service = scoped_keychain_service(config_dir);
    let output = Command::new("security")
        .args(["delete-generic-password", "-s", &service])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output();
    match output {
        Ok(output) if output.status.success() => {}
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            if !stderr.contains("could not be found") {
                LogWriter::log(
                    ErrorReport::new("Failed to delete scoped account Keychain item")
                        .message(stderr)
                        .severity(ErrorSeverity::Warning)
                        .at(file!(), line!())
                        .dedup("account.add.cleanup_keychain_failed")
                        .build(),
                );
            }
        }
        Err(e) => {
            LogWriter::log(
                ErrorReport::new("Failed to delete scoped account Keychain item")
                    .from_error(&e)
                    .severity(ErrorSeverity::Warning)
                    .at(file!(), line!())
                    .dedup("account.add.cleanup_keychain_failed")
                    .build(),
            );
        }
    }
}

pub(super) fn system_credentials_digest() -> Option<String> {
    match find_keychain_secret(SYSTEM_KEYCHAIN_SERVICE) {
        KeychainLookup::Found(secret) => Some(super::secret_digest(&secret)),
        KeychainLookup::Missing | KeychainLookup::Failed => None,
    }
}
