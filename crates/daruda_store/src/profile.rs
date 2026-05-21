//! Active profile resolution — env override + build-cfg fallback.
//! Used by both `persistence::default_data_dir()` and
//! `observability::log_writer::log_profile()` so data and logs stay
//! in sync. Resolved once per process (OnceLock); env changes after
//! startup are ignored on purpose to keep all subsystems consistent.

use std::sync::OnceLock;

/// Environment variable that overrides the build-cfg profile.
pub(crate) const DARUDA_PROFILE_ENV: &str = "DARUDA_PROFILE";

/// Profile name that keeps the legacy (un-suffixed) data path.
/// `persistence::default_data_dir_from` pattern-matches against this
/// when deciding whether to append a `-<profile>` suffix to the
/// platform config dir.
pub(crate) const RELEASE_PROFILE: &str = "release";

/// Returns the active profile name. Order of precedence:
/// 1. `DARUDA_PROFILE` env var (any non-empty string).
/// 2. `cfg!(debug_assertions)` → `"debug"` else `"release"`.
pub fn active_profile() -> &'static str {
    static PROFILE: OnceLock<&'static str> = OnceLock::new();
    PROFILE.get_or_init(resolve_profile)
}

fn resolve_profile() -> &'static str {
    resolve_profile_from(std::env::var(DARUDA_PROFILE_ENV).ok().as_deref())
}

/// Pure resolver — does not read the environment. Public to the
/// crate so tests can exercise every branch without mutating global
/// state (avoids `unsafe { set_var }` + thread-races under parallel
/// `cargo test`).
pub(crate) fn resolve_profile_from(env_val: Option<&str>) -> &'static str {
    if let Some(v) = env_val {
        let trimmed = v.trim();
        if !trimmed.is_empty() {
            // Exactly one allocation per process when called from
            // `resolve_profile()` (OnceLock ensures single init).
            // In tests this leaks per call, which is harmless.
            return Box::leak(trimmed.to_string().into_boxed_str());
        }
    }
    if cfg!(debug_assertions) {
        "debug"
    } else {
        RELEASE_PROFILE
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_profile_from;

    // Tests use a pure resolver instead of mutating process env, so
    // they're safe under parallel `cargo test` and need no `unsafe`.

    #[test]
    fn whitespace_env_falls_back_to_cfg() {
        let expected = if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        };
        assert_eq!(resolve_profile_from(Some("   ")), expected);
    }

    #[test]
    fn env_override_wins() {
        assert_eq!(resolve_profile_from(Some("staging")), "staging");
    }

    #[test]
    fn env_override_trims_whitespace() {
        assert_eq!(resolve_profile_from(Some("  preview  ")), "preview");
    }
}
