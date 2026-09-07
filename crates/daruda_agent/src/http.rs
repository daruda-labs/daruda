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
use std::thread;
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

/// Per-attempt request budget. Both endpoints respond in well under a
/// second on a normal connection; capping each try at 10s keeps the
/// pump responsive when DNS or TLS hangs.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Retry only short transient failures. Longer `Retry-After` values are
/// left for the caller's regular poll cadence so this synchronous helper
/// does not park a background worker for minutes.
const MAX_RETRY_DELAY: Duration = Duration::from_secs(30);

/// Total attempts for retryable status / transport failures.
const MAX_JSON_ATTEMPTS: usize = 3;

/// Maximum response body size we'll buffer (1 MiB). Both endpoints
/// reply with a few hundred bytes; the cap protects against a
/// pathological proxy returning an unbounded stream.
const MAX_BODY_BYTES: usize = 1024 * 1024;

/// `(name, value)` pair passed to [`get_json`]. Aliasing the tuple
/// keeps the call sites readable.
pub type Header<'a> = (&'a str, &'a str);

/// Issue a GET against `url` with the supplied `headers` and parse
/// the response body as JSON. Caps the body at 1 MiB and the total
/// per-attempt wall-clock at 10 s. Short retryable failures (`429` and
/// `5xx`, plus transport misses) are retried before returning
/// `FetchError::Http`; JSON-decode failures return `FetchError::Parse`.
pub fn get_json(url: &str, headers: &[Header<'_>]) -> Result<serde_json::Value, FetchError> {
    let agent = ureq::AgentBuilder::new().timeout(REQUEST_TIMEOUT).build();

    let mut attempt = 0;
    let response = loop {
        let mut req = agent.get(url);
        for (name, value) in headers {
            req = req.set(name, value);
        }

        match req.call() {
            Ok(response) => break response,
            Err(error) => {
                if let Some(delay) = retry_delay_for_error(&error, attempt) {
                    thread::sleep(delay);
                    attempt += 1;
                    continue;
                }
                return Err(http_error(error));
            }
        }
    };

    let mut body = String::new();
    response
        .into_reader()
        .take(MAX_BODY_BYTES as u64)
        .read_to_string(&mut body)
        .map_err(|e| FetchError::Http(e.to_string()))?;

    serde_json::from_str(&body).map_err(|e| FetchError::Parse(e.to_string()))
}

fn retry_delay_for_error(error: &ureq::Error, attempt: usize) -> Option<Duration> {
    if attempt + 1 >= MAX_JSON_ATTEMPTS {
        return None;
    }
    match error {
        ureq::Error::Status(status, response) if retryable_status(*status) => {
            retry_delay(response.header("retry-after"), attempt, chrono::Utc::now())
        }
        ureq::Error::Transport(_) => fallback_retry_delay(attempt),
        _ => None,
    }
}

fn retryable_status(status: u16) -> bool {
    status == 429 || (500..=599).contains(&status)
}

fn retry_delay(
    retry_after: Option<&str>,
    attempt: usize,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<Duration> {
    let fallback = fallback_retry_delay(attempt)?;
    let delay = retry_after
        .and_then(|value| parse_retry_after(value, now))
        .map(|server_delay| server_delay.max(fallback))
        .unwrap_or(fallback);
    (delay <= MAX_RETRY_DELAY).then_some(delay)
}

fn fallback_retry_delay(attempt: usize) -> Option<Duration> {
    match attempt {
        0 => Some(Duration::from_secs(1)),
        1 => Some(Duration::from_secs(3)),
        _ => None,
    }
}

fn parse_retry_after(value: &str, now: chrono::DateTime<chrono::Utc>) -> Option<Duration> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if let Ok(secs) = value.parse::<u64>() {
        return Some(Duration::from_secs(secs));
    }

    let at = chrono::DateTime::parse_from_rfc2822(value)
        .ok()?
        .with_timezone(&chrono::Utc);
    if at <= now {
        Some(Duration::ZERO)
    } else {
        (at - now).to_std().ok()
    }
}

fn http_error(error: ureq::Error) -> FetchError {
    match error {
        ureq::Error::Status(status, response) => {
            FetchError::Http(status_error_message(status, &response))
        }
        ureq::Error::Transport(transport) => FetchError::Http(transport.to_string()),
    }
}

fn status_error_message(status: u16, response: &ureq::Response) -> String {
    let mut message = format!(
        "{}: status code {} {}",
        response.get_url(),
        status,
        response.status_text()
    );
    if let Some(retry_after) = response.header("retry-after") {
        message.push_str(&format!("; retry-after: {retry_after}"));
    }
    if let Some(content_type) = response.header("content-type") {
        message.push_str(&format!("; content-type: {content_type}"));
    }
    message
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn fetch_error_display_includes_inner() {
        let e = FetchError::Http("connection refused".to_string());
        assert!(e.to_string().contains("connection refused"));
        let e = FetchError::Parse("missing field".to_string());
        assert!(e.to_string().contains("missing field"));
        let e = FetchError::NoToken;
        assert!(e.to_string().contains("Keychain"));
    }

    #[test]
    fn retry_after_accepts_delta_seconds_and_http_date() {
        let now = chrono::Utc.with_ymd_and_hms(2026, 9, 7, 8, 0, 0).unwrap();
        assert_eq!(parse_retry_after("5", now), Some(Duration::from_secs(5)));
        assert_eq!(
            parse_retry_after("Mon, 07 Sep 2026 08:00:10 GMT", now),
            Some(Duration::from_secs(10))
        );
    }

    #[test]
    fn retry_delay_uses_short_backoff_floor_and_declines_long_waits() {
        let now = chrono::Utc.with_ymd_and_hms(2026, 9, 7, 8, 0, 0).unwrap();
        assert_eq!(retry_delay(Some("0"), 0, now), Some(Duration::from_secs(1)));
        assert_eq!(retry_delay(Some("5"), 0, now), Some(Duration::from_secs(5)));
        assert_eq!(retry_delay(Some("120"), 0, now), None);
    }
}
