//! macOS Keychain storage for the Telegram bot token.
//!
//! Mirrors `daruda_claude::limits`'s Anthropic OAuth token read: shell
//! out to `/usr/bin/security` rather than link a Keychain crate. The
//! bot token is a secret and never touches `config.toml` — see
//! `daruda_config::TelegramConfig` for the non-secret settings that do
//! (master switch, paired chat id).
//!
//! `read_token` / `write_token` / `delete_token` are called from the
//! Settings window's Telegram section (`settings_window::sections`) —
//! "Save Token" / "Clear" — and `read_token` is also polled by the
//! bridge's poll/send loops (`telegram::global`).

use std::process::{Command, Stdio};

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;

const SERVICE: &str = "daruda-telegram-bot";
const ACCOUNT: &str = "token";

/// `SERVICE`, suffixed with the active build profile (mirrors
/// `daruda_store::persistence::default_data_dir`'s release/debug/named
/// split) — release keeps the bare name, so a debug or named-profile
/// daruda never shares the same Keychain-stored bot token as a real
/// release install. Sharing one token meant both processes long-polled
/// `getUpdates` simultaneously, and Telegram 409-conflicts whichever one
/// isn't already holding the poll — confirmed live (both logs showed
/// `status code 409` at the same timestamps while a debug and a release
/// build were open at once).
fn service_name() -> String {
    match daruda_store::persistence::profile_suffix() {
        Some(suffix) => format!("{SERVICE}-{suffix}"),
        None => SERVICE.to_string(),
    }
}

/// Read the bot token from the Keychain. Returns `None` when no token
/// is stored, the `security` CLI is unavailable or fails, or the
/// build is not macOS.
#[cfg(target_os = "macos")]
pub fn read_token() -> Option<String> {
    // Tests must never read the real Keychain: `cargo test` is a debug build
    // whose profile-suffixed `service_name()` equals a developer's real
    // `daruda-telegram-bot-debug` token, so an un-stubbed read would feed a
    // live token into the poll loop (real `getUpdates` → teardown hang, and a
    // 409 conflict with a running app). Stay hermetic — no `security` call.
    if cfg!(test) {
        return None;
    }
    let out = Command::new("security")
        .args([
            "find-generic-password",
            "-s",
            &service_name(),
            "-a",
            ACCOUNT,
            "-w",
        ])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    normalize_token(&out.stdout)
}

/// Decode raw Keychain output into a usable token: UTF-8 decode, trim
/// surrounding whitespace, and treat an empty (or whitespace-only)
/// result as "no token". Split from `read_token` so the parsing logic
/// is plain and unit-testable without shelling out to `security` —
/// mirrors `daruda_claude::limits::parse_keychain_credentials`.
fn normalize_token(raw: &[u8]) -> Option<String> {
    let token = std::str::from_utf8(raw).ok()?;
    let trimmed = token.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

#[cfg(not(target_os = "macos"))]
pub fn read_token() -> Option<String> {
    None
}

/// Write (or replace) the bot token in the Keychain. `-U` updates the
/// item in place if it already exists, so re-pairing from Settings
/// does not leave stale duplicate entries.
#[cfg(target_os = "macos")]
pub fn write_token(token: &str) -> std::io::Result<()> {
    let output = Command::new("security")
        .args([
            "add-generic-password",
            "-U",
            "-s",
            &service_name(),
            "-a",
            ACCOUNT,
            "-w",
            token,
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output();
    let output = match output {
        Ok(output) => output,
        Err(e) => {
            LogWriter::log(
                ErrorReport::new("Telegram token keychain write failed")
                    .severity(ErrorSeverity::Warning)
                    .from_error(&e)
                    .at(file!(), line!())
                    .dedup("telegram.keychain.write")
                    .build(),
            );
            return Err(e);
        }
    };
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let err = std::io::Error::other(format!("security add-generic-password failed: {stderr}"));
    LogWriter::log(
        ErrorReport::new("Telegram token keychain write failed")
            .severity(ErrorSeverity::Warning)
            .from_error(&err)
            .at(file!(), line!())
            .dedup("telegram.keychain.write")
            .build(),
    );
    Err(err)
}

#[cfg(not(target_os = "macos"))]
pub fn write_token(_token: &str) -> std::io::Result<()> {
    Ok(())
}

/// Delete the bot token from the Keychain (the Settings "clear"
/// affordance).
#[cfg(target_os = "macos")]
pub fn delete_token() -> std::io::Result<()> {
    let output = Command::new("security")
        .args([
            "delete-generic-password",
            "-s",
            &service_name(),
            "-a",
            ACCOUNT,
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output();
    let output = match output {
        Ok(output) => output,
        Err(e) => {
            LogWriter::log(
                ErrorReport::new("Telegram token keychain delete failed")
                    .severity(ErrorSeverity::Warning)
                    .from_error(&e)
                    .at(file!(), line!())
                    .dedup("telegram.keychain.delete")
                    .build(),
            );
            return Err(e);
        }
    };
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let err = std::io::Error::other(format!("security delete-generic-password failed: {stderr}"));
    LogWriter::log(
        ErrorReport::new("Telegram token keychain delete failed")
            .severity(ErrorSeverity::Warning)
            .from_error(&err)
            .at(file!(), line!())
            .dedup("telegram.keychain.delete")
            .build(),
    );
    Err(err)
}

#[cfg(not(target_os = "macos"))]
pub fn delete_token() -> std::io::Result<()> {
    Ok(())
}

// `normalize_token` tests run unconditionally (it's a plain function,
// no subprocess). The `read_token` / `write_token` / `delete_token`
// no-op tests exercise the non-macOS fallback path, so only those
// individual test functions are gated off macOS (matching
// `daruda_claude::limits`'s `read_keychain_credentials_returns_no_token_off_macos`
// pattern) rather than the whole module — keeping the module itself
// compiling and selectable by `cargo test` on macOS CI.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_name_is_suffixed_off_release() {
        // Test binaries compile with debug_assertions, so the active
        // profile here is "debug" (never "release") — asserts the
        // fix's whole point: a non-release build's Keychain service name
        // must differ from the bare `SERVICE`, or it would share (and
        // 409-conflict) the same stored token as a real release install.
        let name = service_name();
        assert_ne!(name, SERVICE);
        assert!(name.starts_with(SERVICE));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn read_token_is_hermetic_under_test() {
        // Tests must never read the real Keychain: `cargo test` runs a debug
        // build, whose profile-suffixed service name matches a developer's
        // real `daruda-telegram-bot-debug` token, so an un-stubbed read would
        // feed a live token into the poll loop — real network + teardown hang.
        // `is_none()` (not `assert_eq!`) so a regression never prints the
        // secret into test output.
        assert!(
            read_token().is_none(),
            "read_token must be hermetic under test"
        );
    }

    #[test]
    fn normalize_token_trims_and_accepts_valid_token() {
        assert_eq!(
            normalize_token(b"  abc123-token  \n"),
            Some("abc123-token".to_string())
        );
    }

    #[test]
    fn normalize_token_rejects_empty_after_trim() {
        assert_eq!(normalize_token(b"   \n\t  "), None);
    }

    #[test]
    fn normalize_token_rejects_fully_empty_input() {
        assert_eq!(normalize_token(b""), None);
    }

    #[test]
    fn normalize_token_rejects_invalid_utf8() {
        assert_eq!(normalize_token(&[0xff, 0xfe, 0xfd]), None);
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn read_token_returns_none_off_macos() {
        assert_eq!(read_token(), None);
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn write_token_is_noop_ok_off_macos() {
        assert!(write_token("fake-token").is_ok());
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn delete_token_is_noop_ok_off_macos() {
        assert!(delete_token().is_ok());
    }
}
