//! Anthropic plan-limit windows from `/api/oauth/usage`.
//!
//! Two windows: a 5-hour rolling token-usage window and a 7-day
//! window. Each carries a utilization percentage (0–100) and a
//! reset timestamp. The Usage tab renders these as gauge bars so
//! the user can see when they will throttle.
//!
//! Token discovery is macOS-only — the Anthropic OAuth token lives
//! in the macOS Keychain under `Claude Code-credentials`. On every
//! other platform `read_keychain_token` returns
//! [`FetchError::NoToken`] and the Usage tab falls back to placeholder
//! gauges. This is intentional: the Linux / Windows ports of Claude
//! Code use a different storage scheme and we'd rather not silently
//! pretend to fetch from a non-existent token than half-implement a
//! cross-platform reader.

use std::time::SystemTime;

use chrono::DateTime;
use serde_json::Value;

use crate::http::{FetchError, Header, get_json};

/// Snapshot of both plan-rate windows as returned by Anthropic's
/// `/api/oauth/usage`. Either window is `None` when Anthropic omits
/// it from the response (e.g. the account hasn't accumulated any
/// usage yet); `fetched_at` is the wall-clock the response landed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlanLimits {
    pub five_hour: Option<LimitWindow>,
    pub seven_day: Option<LimitWindow>,
    /// Wall-clock when the response was successfully decoded. `None`
    /// before the first fetch lands. The renderer uses this to
    /// dim the gauges with a "stale" hint after a long period
    /// without successful refreshes.
    pub fetched_at: Option<SystemTime>,
}

/// A single plan-rate window. Utilization is clamped at the parser
/// boundary so the renderer can trust `0.0..=100.0` and skip its own
/// guards.
#[derive(Clone, Debug, PartialEq)]
pub struct LimitWindow {
    /// `0.0 ..= 100.0` — utilization percentage. Out-of-range values
    /// from the API (defensive: the contract is 0–100) are clamped
    /// during parse so callers never see negatives or >100.
    pub utilization: f32,
    /// Wall-clock when the rolling window resets. `None` when the
    /// API omits the field.
    pub resets_at: Option<SystemTime>,
}

/// Severity bucket for a utilization value. Drives the bar color in
/// the Usage tab and the optional "you're about to throttle" warning.
/// Thresholds match the Übersicht widget the Usage tab is modelled
/// on (`getBarColor`): green below 50%, yellow at 50–80%, red ≥ 80%.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LimitSeverity {
    /// `utilization < 50.0`
    Low,
    /// `50.0 <= utilization < 80.0`
    Medium,
    /// `utilization >= 80.0`
    High,
}

impl LimitSeverity {
    /// Map a utilization percentage (0–100) to a severity bucket.
    /// Treats NaN / negatives as `Low` so a malformed API value
    /// never trips a bogus High alert.
    pub fn from_utilization(pct: f32) -> Self {
        if pct >= LIMIT_HIGH_THRESHOLD {
            Self::High
        } else if pct >= LIMIT_MEDIUM_THRESHOLD {
            Self::Medium
        } else {
            Self::Low
        }
    }
}

/// Boundary at which the bar flips from green to yellow.
pub const LIMIT_MEDIUM_THRESHOLD: f32 = 50.0;
/// Boundary at which the bar flips from yellow to red.
pub const LIMIT_HIGH_THRESHOLD: f32 = 80.0;

/// Fetch and parse the plan-rate response. Synchronous — callers
/// (`limits_pump`) wrap it in `cx.background_executor().spawn(...)`
/// so the GPUI thread never blocks on `ureq`.
pub fn fetch_plan_limits() -> Result<PlanLimits, FetchError> {
    let token = read_keychain_token()?;
    let auth = format!("Bearer {token}");
    let headers: [Header<'_>; 3] = [
        ("Authorization", &auth),
        ("anthropic-beta", "oauth-2025-04-20"),
        ("Content-Type", "application/json"),
    ];
    let body = get_json("https://api.anthropic.com/api/oauth/usage", &headers)?;
    parse_plan_limits(&body)
}

/// Parse the JSON body returned by `/api/oauth/usage`. Split out so
/// it can be unit-tested with mock fixtures without standing up a
/// network mock.
pub fn parse_plan_limits(value: &Value) -> Result<PlanLimits, FetchError> {
    Ok(PlanLimits {
        five_hour: parse_window(value.get("five_hour")),
        seven_day: parse_window(value.get("seven_day")),
        fetched_at: Some(SystemTime::now()),
    })
}

fn parse_window(value: Option<&Value>) -> Option<LimitWindow> {
    let v = value?;
    if v.is_null() {
        return None;
    }
    let utilization = v.get("utilization").and_then(Value::as_f64)? as f32;
    Some(LimitWindow {
        utilization: utilization.clamp(0.0, 100.0),
        resets_at: v
            .get("resets_at")
            .and_then(Value::as_str)
            .and_then(parse_rfc3339),
    })
}

/// Anthropic emits `resets_at` as RFC 3339 (`"2026-05-07T15:00:00Z"`),
/// the strict subset of ISO 8601. Returns `None` on any parse
/// failure so the caller skips `resets_at` instead of swallowing
/// parser bugs into a present-but-bogus `SystemTime`.
fn parse_rfc3339(s: &str) -> Option<SystemTime> {
    DateTime::parse_from_rfc3339(s).ok().map(|dt| dt.into())
}

/// Read the Anthropic OAuth access token from the macOS Keychain.
/// Returns `Err(FetchError::NoToken)` on any failure — missing
/// keychain item, non-macOS build, malformed JSON — so the caller
/// can render a single "measurement unavailable" placeholder
/// without distinguishing root causes.
#[cfg(target_os = "macos")]
fn read_keychain_token() -> Result<String, FetchError> {
    use std::process::Command;
    let out = Command::new("security")
        .args([
            "find-generic-password",
            "-s",
            "Claude Code-credentials",
            "-w",
        ])
        .output()
        .map_err(|_| FetchError::NoToken)?;
    if !out.status.success() {
        return Err(FetchError::NoToken);
    }
    let raw = String::from_utf8(out.stdout).map_err(|_| FetchError::NoToken)?;
    let v: serde_json::Value = serde_json::from_str(raw.trim()).map_err(|_| FetchError::NoToken)?;
    v["claudeAiOauth"]["accessToken"]
        .as_str()
        .map(str::to_string)
        .ok_or(FetchError::NoToken)
}

#[cfg(not(target_os = "macos"))]
fn read_keychain_token() -> Result<String, FetchError> {
    Err(FetchError::NoToken)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_full_response_extracts_both_windows() {
        let body = json!({
            "five_hour": { "utilization": 68.2, "resets_at": "2026-05-07T15:00:00Z" },
            "seven_day": { "utilization": 31.4, "resets_at": "2026-05-10T00:00:00Z" },
        });
        let parsed = parse_plan_limits(&body).unwrap();

        let five = parsed.five_hour.unwrap();
        assert!((five.utilization - 68.2).abs() < 1e-3);
        assert!(five.resets_at.is_some());

        let seven = parsed.seven_day.unwrap();
        assert!((seven.utilization - 31.4).abs() < 1e-3);
        assert!(seven.resets_at.is_some());
    }

    #[test]
    fn parse_missing_windows_yields_none() {
        let body = json!({});
        let parsed = parse_plan_limits(&body).unwrap();
        assert!(parsed.five_hour.is_none());
        assert!(parsed.seven_day.is_none());
        assert!(parsed.fetched_at.is_some());
    }

    #[test]
    fn parse_null_window_yields_none() {
        let body = json!({ "five_hour": null, "seven_day": { "utilization": 10.0 } });
        let parsed = parse_plan_limits(&body).unwrap();
        assert!(parsed.five_hour.is_none());
        assert!(parsed.seven_day.is_some());
    }

    #[test]
    fn parse_window_without_resets_at() {
        let body = json!({ "five_hour": { "utilization": 50.0 } });
        let parsed = parse_plan_limits(&body).unwrap();
        let win = parsed.five_hour.unwrap();
        assert!((win.utilization - 50.0).abs() < 1e-3);
        assert!(win.resets_at.is_none());
    }

    #[test]
    fn parse_clamps_out_of_range_utilization() {
        let body = json!({
            "five_hour": { "utilization": -5.0 },
            "seven_day": { "utilization": 150.0 },
        });
        let parsed = parse_plan_limits(&body).unwrap();
        assert_eq!(parsed.five_hour.unwrap().utilization, 0.0);
        assert_eq!(parsed.seven_day.unwrap().utilization, 100.0);
    }

    #[test]
    fn parse_skips_window_when_utilization_missing() {
        let body = json!({ "five_hour": { "resets_at": "2026-05-07T15:00:00Z" } });
        let parsed = parse_plan_limits(&body).unwrap();
        assert!(parsed.five_hour.is_none());
    }

    #[test]
    fn severity_thresholds_match_uebersicht() {
        // Under 50% → low.
        assert_eq!(LimitSeverity::from_utilization(0.0), LimitSeverity::Low);
        assert_eq!(LimitSeverity::from_utilization(49.999), LimitSeverity::Low);
        // 50% inclusive → medium.
        assert_eq!(LimitSeverity::from_utilization(50.0), LimitSeverity::Medium);
        assert_eq!(
            LimitSeverity::from_utilization(79.999),
            LimitSeverity::Medium
        );
        // 80% inclusive → high.
        assert_eq!(LimitSeverity::from_utilization(80.0), LimitSeverity::High);
        assert_eq!(LimitSeverity::from_utilization(100.0), LimitSeverity::High);
    }

    #[test]
    fn severity_treats_negatives_and_nan_as_low() {
        assert_eq!(LimitSeverity::from_utilization(-1.0), LimitSeverity::Low);
        assert_eq!(
            LimitSeverity::from_utilization(f32::NAN),
            LimitSeverity::Low
        );
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn read_keychain_token_returns_no_token_off_macos() {
        let err = read_keychain_token().unwrap_err();
        assert!(matches!(err, FetchError::NoToken));
    }
}
