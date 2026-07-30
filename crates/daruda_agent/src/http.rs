//! Internal HTTP helper shared by [`crate::limits`] and
//! [`crate::service_status`].
//!
//! Synchronous (`ureq`) on purpose — both endpoints poll on a 5-minute
//! cadence at most and the call stack runs from `BackgroundExecutor`
//! tasks, so an async client would only buy us complexity. Keeping it
//! sync also means `daruda_agent` does not have to pick an async
//! runtime, which would conflict with consumers that already use
//! GPUI's executor.
//!
//! All errors collapse to `FetchError` so the two endpoint modules can
//! treat them uniformly: `NoToken` for the OAuth keychain miss
//! (limits-only but kept here for the shared error surface), `Http`
//! for transport / 4xx / 5xx, and `Parse` for JSON or schema problems.

use std::io::Read;
use std::time::Duration;

/// Failure surface for the two daruda_agent HTTP endpoints. Each
/// variant carries the upstream error rendered as a string so the
/// renderer (which only needs to decide between "show data" and
/// "show placeholder") doesn't have to match against ureq / serde
/// types.
#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    /// Anthropic OAuth token is unavailable in the macOS Keychain
    /// (`security find-generic-password -s 'Claude Code-credentials'`
    /// returned non-zero) or this is a non-macOS build. Returned by
    /// `limits::fetch_plan_limits` only; the status endpoint is
    /// public and never produces this.
    #[error("OAuth token unavailable in Keychain")]
    NoToken,
    /// Network error, DNS failure, TLS handshake failure, non-2xx
    /// status, or response body read error — anything below the JSON
    /// layer. The wrapped string is for logging; the UI only cares
    /// that fetch failed.
    #[error("HTTP error: {0}")]
    Http(String),
    /// JSON could not be decoded, or the decoded shape didn't match
    /// the expected schema (missing required fields, wrong types).
    /// The wrapped string is for logging; surfaced to the UI as a
    /// placeholder.
    #[error("parse error: {0}")]
    Parse(String),
}

/// Total request budget. Both endpoints respond in well under a
/// second on a normal connection; capping at 10s keeps the pump
/// responsive when DNS or TLS hangs.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Maximum response body size we'll buffer (1 MiB). Both endpoints
/// reply with a few hundred bytes; the cap protects against a
/// pathological proxy returning an unbounded stream.
const MAX_BODY_BYTES: usize = 1024 * 1024;

/// `(name, value)` pair passed to [`get_json`]. Aliasing the tuple
/// keeps the call sites readable.
pub type Header<'a> = (&'a str, &'a str);

/// Issue a GET against `url` with the supplied `headers` and parse
/// the response body as JSON. Caps the body at 1 MiB and the total
/// wall-clock at 10 s. Returns `FetchError::Http` for transport / non
/// -2xx and `FetchError::Parse` for JSON-decode failures.
pub fn get_json(url: &str, headers: &[Header<'_>]) -> Result<serde_json::Value, FetchError> {
    let agent = ureq::AgentBuilder::new().timeout(REQUEST_TIMEOUT).build();

    let mut req = agent.get(url);
    for (name, value) in headers {
        req = req.set(name, value);
    }

    let response = req.call().map_err(|e| FetchError::Http(e.to_string()))?;

    let mut body = String::new();
    response
        .into_reader()
        .take(MAX_BODY_BYTES as u64)
        .read_to_string(&mut body)
        .map_err(|e| FetchError::Http(e.to_string()))?;

    serde_json::from_str(&body).map_err(|e| FetchError::Parse(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetch_error_display_includes_inner() {
        let e = FetchError::Http("connection refused".to_string());
        assert!(e.to_string().contains("connection refused"));
        let e = FetchError::Parse("missing field".to_string());
        assert!(e.to_string().contains("missing field"));
        let e = FetchError::NoToken;
        assert!(e.to_string().contains("Keychain"));
    }
}
