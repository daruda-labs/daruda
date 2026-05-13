//! Cached, privacy-minimal system summary appended to every
//! [`ErrorReport`](super::error_report::ErrorReport) plain-text
//! rendering (D6).
//!
//! # Allowed in the summary
//!
//! - daruda binary version (set once at startup via
//!   [`set_app_version`]; defaults to `"?"` if unset),
//! - target OS (`std::env::consts::OS`, e.g. `"macos"`),
//! - target architecture (`std::env::consts::ARCH`, e.g. `"aarch64"`).
//!
//! # Deliberately excluded
//!
//! - hostname / username (no `whoami` / `gethostname` calls),
//! - OS build number / kernel version (no `uname -r` / sysctl),
//! - GPU / memory / CPU model,
//! - environment variables.
//!
//! Path-shaped context values must be redacted with
//! [`redact_home`] before they enter `ErrorReport.context`. The
//! summary itself never reads paths.

use std::path::Path;
use std::sync::OnceLock;

static APP_VERSION: OnceLock<String> = OnceLock::new();
static SUMMARY_CACHE: OnceLock<String> = OnceLock::new();

/// Set the daruda binary version. Idempotent — only the first call
/// wins; subsequent calls are silently ignored. The app crate calls
/// this once from `main()` with `env!("CARGO_PKG_VERSION")` so the
/// observability layer can stay in `daruda_store` without taking on
/// a circular dependency.
pub fn set_app_version(version: impl Into<String>) {
    let _ = APP_VERSION.set(version.into());
}

/// Cached privacy-minimal system summary string. Format:
/// `daruda <version>  ·  <os>  ·  <arch>`.
///
/// The cache is global and built on first call; later calls are
/// effectively free.
pub fn summary() -> &'static str {
    SUMMARY_CACHE.get_or_init(|| {
        let version = APP_VERSION.get().map(String::as_str).unwrap_or("?");
        format!(
            "daruda {}  ·  {}  ·  {}",
            version,
            std::env::consts::OS,
            std::env::consts::ARCH,
        )
    })
}

/// Replace the user's home directory prefix with `~` for safe
/// inclusion in [`ErrorReport.context`](super::error_report::ErrorReport).
/// Falls back to the lossy string form when the path is not under
/// `$HOME`.
///
/// Example: `/Users/alice/git/foo` → `~/git/foo`.
pub fn redact_home(path: impl AsRef<Path>) -> String {
    let path = path.as_ref();
    if let Some(home) = dirs::home_dir()
        && let Ok(rest) = path.strip_prefix(&home)
    {
        if rest.as_os_str().is_empty() {
            return "~".to_string();
        }
        return format!("~/{}", rest.to_string_lossy());
    }
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn summary_contains_os_and_arch() {
        // We cannot reliably test `set_app_version` here because the
        // OnceLock is process-global and other tests in this module
        // may have set it. Just assert the static parts are present.
        let s = summary();
        assert!(s.starts_with("daruda "));
        assert!(s.contains(std::env::consts::OS));
        assert!(s.contains(std::env::consts::ARCH));
    }

    #[test]
    fn summary_omits_privacy_sensitive_fields() {
        let s = summary();
        // No environment variables.
        if let Ok(home) = std::env::var("HOME") {
            assert!(!s.contains(&home), "summary leaked $HOME");
        }
        if let Ok(user) = std::env::var("USER") {
            // Allow short usernames that might collide with "macos" etc.
            // — assert only when the username is at least 4 chars.
            if user.len() >= 4 {
                assert!(!s.contains(&user), "summary leaked $USER");
            }
        }
    }

    #[test]
    fn redact_home_replaces_prefix() {
        let Some(home) = dirs::home_dir() else {
            return;
        };
        let nested = home.join("git").join("daruda");
        let redacted = redact_home(&nested);
        assert!(
            redacted.starts_with("~"),
            "expected leading ~, got {redacted:?}"
        );
        assert!(
            !redacted.contains(&home.to_string_lossy().into_owned()),
            "redact_home leaked the literal home prefix"
        );
    }

    #[test]
    fn redact_home_returns_path_unchanged_when_outside_home() {
        let p = PathBuf::from("/tmp/some/path");
        let r = redact_home(&p);
        assert_eq!(r, "/tmp/some/path");
    }

    #[test]
    fn redact_home_handles_exact_home() {
        let Some(home) = dirs::home_dir() else {
            return;
        };
        assert_eq!(redact_home(&home), "~");
    }
}
