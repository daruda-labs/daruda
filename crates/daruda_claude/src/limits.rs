//! Anthropic plan-limit windows from `/api/oauth/usage`.
//!
//! Three windows: a 5-hour rolling token-usage window, a 7-day
//! window, and (on plans with a separate Opus budget) a 7-day window
//! scoped to Opus-class models. Each carries a utilization percentage
//! (0–100) and a reset timestamp. The Usage tab renders these as
//! gauge bars so the user can see when they will throttle.
//!
//! Token discovery is macOS-only — the Anthropic OAuth token lives
//! in the macOS Keychain under `Claude Code-credentials`. On every
//! other platform `read_keychain_credentials` returns
//! [`FetchError::NoToken`] and the Usage tab falls back to placeholder
//! gauges. This is intentional: the Linux / Windows ports of Claude
//! Code use a different storage scheme and we'd rather not silently
//! pretend to fetch from a non-existent token than half-implement a
//! cross-platform reader. The same Keychain JSON also carries the
//! subscription metadata surfaced as [`PlanInfo`].

use std::time::SystemTime;

use chrono::DateTime;
use serde_json::Value;

use crate::http::{FetchError, Header, get_json};

/// Snapshot of the plan-rate windows as returned by Anthropic's
/// `/api/oauth/usage`. Any window is `None` when Anthropic omits
/// it from the response (e.g. the account hasn't accumulated any
/// usage yet, or the plan has no separate Opus budget); `fetched_at`
/// is the wall-clock the response landed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlanLimits {
    pub five_hour: Option<LimitWindow>,
    pub seven_day: Option<LimitWindow>,
    /// 7-day window scoped to Opus-class models. Present only on
    /// plans that meter Opus separately from the overall budget.
    pub seven_day_opus: Option<LimitWindow>,
    /// Subscription metadata read from the local Keychain credentials,
    /// not from the usage response. Filled by [`fetch_plan_limits`];
    /// always `None` from [`parse_plan_limits`], which is
    /// API-response-only.
    pub plan: Option<PlanInfo>,
    /// Wall-clock when the response was successfully decoded. `None`
    /// before the first fetch lands. The renderer uses this to
    /// dim the gauges with a "stale" hint after a long period
    /// without successful refreshes.
    pub fetched_at: Option<SystemTime>,
}

/// Subscription metadata from the Keychain `claudeAiOauth` object.
/// Both fields are pass-through strings — Anthropic adds tiers and
/// plan names without notice, so daruda displays them verbatim
/// instead of mapping to an enum that would go stale.
#[derive(Clone, Debug, PartialEq)]
pub struct PlanInfo {
    /// e.g. "team", "max", "pro".
    pub subscription_type: Option<String>,
    /// e.g. "default_claude_ai_5x".
    pub rate_limit_tier: Option<String>,
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
    let (token, plan) = read_keychain_credentials()?;
    let auth = format!("Bearer {token}");
    let headers: [Header<'_>; 3] = [
        ("Authorization", &auth),
        ("anthropic-beta", "oauth-2025-04-20"),
        ("Content-Type", "application/json"),
    ];
    let body = get_json("https://api.anthropic.com/api/oauth/usage", &headers)?;
    let mut limits = parse_plan_limits(&body)?;
    limits.plan = Some(plan);
    Ok(limits)
}

/// Parse the JSON body returned by `/api/oauth/usage`. Split out so
/// it can be unit-tested with mock fixtures without standing up a
/// network mock. `plan` stays `None` here — it comes from the local
/// Keychain, not from the API response.
pub fn parse_plan_limits(value: &Value) -> Result<PlanLimits, FetchError> {
    Ok(PlanLimits {
        five_hour: parse_window(value.get("five_hour")),
        seven_day: parse_window(value.get("seven_day")),
        seven_day_opus: parse_window(value.get("seven_day_opus")),
        plan: None,
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

/// Read the Anthropic OAuth access token plus subscription metadata
/// from the macOS Keychain. One Keychain read serves both — the
/// `claudeAiOauth` JSON object carries `accessToken` alongside
/// `subscriptionType` / `rateLimitTier`. Returns
/// `Err(FetchError::NoToken)` on any failure — missing keychain item,
/// non-macOS build, malformed JSON — so the caller can render a
/// single "measurement unavailable" placeholder without
/// distinguishing root causes.
#[cfg(target_os = "macos")]
fn read_keychain_credentials() -> Result<(String, PlanInfo), FetchError> {
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
    parse_keychain_credentials(&raw)
}

#[cfg(not(target_os = "macos"))]
fn read_keychain_credentials() -> Result<(String, PlanInfo), FetchError> {
    Err(FetchError::NoToken)
}

/// Extract `(access token, plan info)` from the Keychain credentials
/// JSON. Pure — split from the `security` subprocess call so it can
/// be unit-tested with fixtures. A missing token is fatal
/// (`FetchError::NoToken`); missing subscription fields are not —
/// they just leave the corresponding [`PlanInfo`] slots `None`.
fn parse_keychain_credentials(raw: &str) -> Result<(String, PlanInfo), FetchError> {
    let v: serde_json::Value = serde_json::from_str(raw.trim()).map_err(|_| FetchError::NoToken)?;
    let oauth = &v["claudeAiOauth"];
    let token = oauth["accessToken"]
        .as_str()
        .map(str::to_string)
        .ok_or(FetchError::NoToken)?;
    let plan = PlanInfo {
        subscription_type: oauth["subscriptionType"].as_str().map(str::to_string),
        rate_limit_tier: oauth["rateLimitTier"].as_str().map(str::to_string),
    };
    Ok((token, plan))
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

        // No Opus window in the response → None, not an error.
        assert!(parsed.seven_day_opus.is_none());
    }

    #[test]
    fn parse_extracts_seven_day_opus() {
        let body = json!({
            "five_hour": { "utilization": 68.2, "resets_at": "2026-05-07T15:00:00Z" },
            "seven_day": { "utilization": 31.4, "resets_at": "2026-05-10T00:00:00Z" },
            "seven_day_opus": { "utilization": 12.5, "resets_at": "2026-05-10T00:00:00Z" },
        });
        let parsed = parse_plan_limits(&body).unwrap();
        assert!(parsed.five_hour.is_some());
        assert!(parsed.seven_day.is_some());
        let opus = parsed.seven_day_opus.unwrap();
        assert!((opus.utilization - 12.5).abs() < 1e-3);
        assert!(opus.resets_at.is_some());
    }

    #[test]
    fn parse_plan_limits_never_fills_plan_info() {
        // Plan info comes from the Keychain, not the usage response —
        // even a response that happens to carry plan-ish keys must not
        // populate it.
        let body = json!({
            "five_hour": { "utilization": 1.0 },
            "subscriptionType": "max",
        });
        let parsed = parse_plan_limits(&body).unwrap();
        assert_eq!(parsed.plan, None);
    }

    #[test]
    fn parse_missing_windows_yields_none() {
        let body = json!({});
        let parsed = parse_plan_limits(&body).unwrap();
        assert!(parsed.five_hour.is_none());
        assert!(parsed.seven_day.is_none());
        assert!(parsed.seven_day_opus.is_none());
        assert!(parsed.plan.is_none());
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

    #[test]
    fn keychain_credentials_extract_token_and_plan_info() {
        let raw = r#"{
            "claudeAiOauth": {
                "accessToken": "sk-ant-oat01-abc",
                "refreshToken": "sk-ant-ort01-def",
                "expiresAt": 1778112000000,
                "scopes": ["user:inference", "user:profile"],
                "subscriptionType": "team",
                "rateLimitTier": "default_claude_ai_5x"
            }
        }"#;
        let (token, plan) = parse_keychain_credentials(raw).unwrap();
        assert_eq!(token, "sk-ant-oat01-abc");
        assert_eq!(plan.subscription_type.as_deref(), Some("team"));
        assert_eq!(
            plan.rate_limit_tier.as_deref(),
            Some("default_claude_ai_5x")
        );
    }

    #[test]
    fn keychain_credentials_tolerate_missing_plan_fields() {
        // Older credential payloads carry only the token — the fetch
        // must still proceed, with empty plan slots.
        let raw = r#"{ "claudeAiOauth": { "accessToken": "tok" } }"#;
        let (token, plan) = parse_keychain_credentials(raw).unwrap();
        assert_eq!(token, "tok");
        assert_eq!(plan.subscription_type, None);
        assert_eq!(plan.rate_limit_tier, None);
    }

    #[test]
    fn keychain_credentials_without_token_is_no_token() {
        let raw = r#"{ "claudeAiOauth": { "subscriptionType": "pro" } }"#;
        let err = parse_keychain_credentials(raw).unwrap_err();
        assert!(matches!(err, FetchError::NoToken));
    }

    #[test]
    fn keychain_credentials_malformed_json_is_no_token() {
        let err = parse_keychain_credentials("{ not json").unwrap_err();
        assert!(matches!(err, FetchError::NoToken));
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn read_keychain_credentials_returns_no_token_off_macos() {
        let err = read_keychain_credentials().unwrap_err();
        assert!(matches!(err, FetchError::NoToken));
    }
}
