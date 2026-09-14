//! Registry and read boundary for environment variables owned by daruda.
//!
//! A [`Key`] can only be constructed in this module, so first-party Rust code
//! cannot introduce another `DARUDA_*` name without extending this registry.
//! Reading stays uncached: callers that need snapshot semantics own that
//! decision, and bootstrap code that writes defaults remains at the
//! single-threaded process boundary.
//!
//! `DARUDA_BIN` is the one shell-only exception. The installed hook reads it
//! in `crates/app/src/hooks/notify.sh`; Rust neither reads nor writes it.

use std::env::VarError;
use std::ffi::OsString;
use std::time::Duration;

/// A registered daruda-owned process-environment name.
///
/// The field and constructor are private so consumers must choose one of the
/// declared keys below instead of creating an arbitrary name.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Key(&'static str);

impl Key {
    const fn new(name: &'static str) -> Self {
        Self(name)
    }

    /// Return the exact name for child-process injection or bootstrap writes.
    pub const fn name(self) -> &'static str {
        self.0
    }

    /// Read a Unicode value, preserving `std::env::var` error semantics.
    pub fn read_utf8(self) -> Result<String, VarError> {
        std::env::var(self.0)
    }

    /// Read a platform-native value, including an explicitly empty value.
    pub fn read_os(self) -> Option<OsString> {
        std::env::var_os(self.0)
    }

    /// Whether the key exists. An explicitly empty value counts as present.
    pub fn is_present(self) -> bool {
        self.read_os().is_some()
    }
}

/// Full data-directory override used by persistence and isolated runs.
pub const DATA_DIR: Key = Key::new("DARUDA_DATA_DIR");

/// Runtime profile override used to partition persistent state and logs.
pub const PROFILE: Key = Key::new("DARUDA_PROFILE");

/// ACP wire-log base path. Unset leaves the tap disabled.
pub const ACP_WIRE_LOG: Key = Key::new("DARUDA_ACP_WIRE_LOG");

/// ACP wire-log payload-sidecar switch. False-like values disable the sidecar.
pub const ACP_WIRE_LOG_PAYLOADS: Key = Key::new("DARUDA_ACP_WIRE_LOG_PAYLOADS");

/// ACP wire-log field spill threshold in bytes. Zero disables elision.
pub const ACP_WIRE_LOG_MAX_FIELD: Key = Key::new("DARUDA_ACP_WIRE_LOG_MAX_FIELD");

/// Telegram diagnostic trace path. Unset leaves the trace disabled.
pub const TELEGRAM_LOG: Key = Key::new("DARUDA_TELEGRAM_LOG");

/// Screenshot post-launch settle delay in milliseconds.
pub const SCREENSHOT_SETTLE_MS: Key = Key::new("DARUDA_SCREENSHOT_SETTLE_MS");

/// ACP replay post-launch settle delay in milliseconds.
pub const REPLAY_SETTLE_MS: Key = Key::new("DARUDA_REPLAY_SETTLE_MS");

/// Presence-only diagnostic switch that disables terminal seam deduplication.
pub const NO_SEAM_DEDUP: Key = Key::new("DARUDA_NO_SEAM_DEDUP");

/// Presence-only diagnostic switch for agent-list measurement tracing.
pub const DEBUG_AGENT_LIST: Key = Key::new("DARUDA_DEBUG_AGENT_LIST");

/// Optional ACP capture path for the ignored live row-census test.
pub const CENSUS_LOG: Key = Key::new("DARUDA_CENSUS_LOG");

/// Optional adapter id paired with [`CENSUS_LOG`].
pub const CENSUS_AGENT: Key = Key::new("DARUDA_CENSUS_AGENT");

/// Adapter command override used by the standalone flow example.
pub const FLOW_AGENT: Key = Key::new("DARUDA_FLOW_AGENT");

/// Run directory injected into every flow command node.
pub const FLOW_RUN_DIR: Key = Key::new("DARUDA_FLOW_RUN_DIR");

/// Node id injected into every flow command node.
pub const FLOW_NODE_ID: Key = Key::new("DARUDA_FLOW_NODE_ID");

/// One-based attempt number injected into every flow command node.
pub const FLOW_ATTEMPT: Key = Key::new("DARUDA_FLOW_ATTEMPT");

/// Per-session MCP authentication channel. Its value must never be logged.
pub const CONTROL_TOKEN: Key = Key::new("DARUDA_CONTROL_TOKEN");

/// Read a registered millisecond value, falling back when absent or invalid.
pub fn read_millis_or(key: Key, default: Duration) -> Duration {
    millis_or(key.read_utf8().ok().as_deref(), default)
}

fn millis_or(value: Option<&str>, default: Duration) -> Duration {
    value
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEFAULT: Duration = Duration::from_millis(2_000);

    #[test]
    fn milliseconds_accept_whitespace_and_zero() {
        assert_eq!(
            millis_or(Some(" 750 "), DEFAULT),
            Duration::from_millis(750)
        );
        assert_eq!(millis_or(Some("0"), DEFAULT), Duration::ZERO);
    }

    #[test]
    fn milliseconds_fall_back_when_absent_or_invalid() {
        assert_eq!(millis_or(None, DEFAULT), DEFAULT);
        assert_eq!(millis_or(Some(""), DEFAULT), DEFAULT);
        assert_eq!(millis_or(Some("soon"), DEFAULT), DEFAULT);
        assert_eq!(millis_or(Some("-1"), DEFAULT), DEFAULT);
        assert_eq!(millis_or(Some("18446744073709551616"), DEFAULT), DEFAULT);
    }
}
