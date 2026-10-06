//! Anthropic plan-limit windows from `/api/oauth/usage`.
//!
//! Three windows: a 5-hour rolling one, a 7-day one, and (on plans with a
//! separate Opus budget) a 7-day one scoped to Opus-class models. Missing
//! credentials collapse to [`FetchError::NoToken`] — the caller does not
//! distinguish "not logged in" from "malformed" from "not on this OS".

use std::path::Path;
use std::time::{Duration, SystemTime};

use chrono::DateTime;
use daruda_store::accounts::AccountRecipeId;
use serde_json::Value;

use super::{ProviderUsage, UsageSource, UsageWindow, WindowScope};
use crate::accounts::{PlanInfo, read_scoped_credentials, read_system_credentials};
use crate::http::{FetchError, Header, get_json};

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const STATUS_URL: &str = "https://status.claude.com/api/v2/status.json";

/// Lengths of the windows Anthropic reports. The API names them by JSON key
/// rather than by duration, so the mapping to a neutral length lives here.
const FIVE_HOURS: Duration = Duration::from_secs(5 * 60 * 60);
const SEVEN_DAYS: Duration = Duration::from_secs(7 * 24 * 60 * 60);

pub struct ClaudeUsage;

impl UsageSource for ClaudeUsage {
    fn fetch(&self, config_dir: Option<&Path>) -> Result<ProviderUsage, FetchError> {
        let (token, plan) = match config_dir {
            Some(dir) => read_scoped_credentials(dir).map_err(|_| FetchError::NoToken)?,
            None => read_system_credentials()?,
        };
        let auth = format!("Bearer {token}");
        let headers: [Header<'_>; 3] = [
            ("Authorization", &auth),
            ("anthropic-beta", "oauth-2025-04-20"),
            ("Content-Type", "application/json"),
        ];
        let body = get_json(USAGE_URL, &headers)?;
        Ok(parse_usage(&body, Some(plan)))
    }

    fn status_url(&self) -> &'static str {
        STATUS_URL
    }
}

/// Parse the JSON body returned by `/api/oauth/usage`. Split out so it can be
/// unit-tested against fixtures without a network mock. `plan` comes from the
/// local credential store, never from this response.
pub fn parse_usage(value: &Value, plan: Option<PlanInfo>) -> ProviderUsage {
    let windows = [
        (value.get("five_hour"), FIVE_HOURS, WindowScope::Overall),
        (value.get("seven_day"), SEVEN_DAYS, WindowScope::Overall),
        (value.get("seven_day_opus"), SEVEN_DAYS, WindowScope::Opus),
    ]
    .into_iter()
    .filter_map(|(v, window, scope)| parse_window(v, window, scope))
    .collect();
    ProviderUsage::new(AccountRecipeId::Claude, windows, plan)
}

fn parse_window(
    value: Option<&Value>,
    window: Duration,
    scope: WindowScope,
) -> Option<UsageWindow> {
    let v = value?;
    if v.is_null() {
        return None;
    }
    let utilization = v.get("utilization").and_then(Value::as_f64)? as f32;
    Some(UsageWindow {
        window,
        utilization: utilization.clamp(0.0, 100.0),
        resets_at: v
            .get("resets_at")
            .and_then(Value::as_str)
            .and_then(parse_rfc3339),
        scope,
    })
}

/// Anthropic emits `resets_at` as RFC 3339 (`"2026-05-07T15:00:00Z"`).
/// Returns `None` on any parse failure so the caller drops the reset time
/// rather than swallowing a parser bug into a present-but-bogus `SystemTime`.
fn parse_rfc3339(s: &str) -> Option<SystemTime> {
    DateTime::parse_from_rfc3339(s).ok().map(|dt| dt.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn windows_of(value: &Value) -> Vec<(u64, WindowScope, f32)> {
        parse_usage(value, None)
            .windows
            .into_iter()
            .map(|w| (w.window.as_secs(), w.scope, w.utilization))
            .collect()
    }

    #[test]
    fn parse_maps_both_windows_to_their_lengths() {
        let body = json!({
            "five_hour": { "utilization": 68.2, "resets_at": "2026-05-07T15:00:00Z" },
            "seven_day": { "utilization": 31.4, "resets_at": "2026-05-10T00:00:00Z" },
        });
        let usage = parse_usage(&body, None);
        assert_eq!(usage.recipe, AccountRecipeId::Claude);
        assert_eq!(usage.windows.len(), 2);
        assert_eq!(usage.windows[0].window, FIVE_HOURS);
        assert!((usage.windows[0].utilization - 68.2).abs() < 1e-3);
        assert!(usage.windows[0].resets_at.is_some());
        assert_eq!(usage.windows[1].window, SEVEN_DAYS);
        assert!(usage.fetched_at.is_some());
    }

    #[test]
    fn parse_marks_the_opus_window_by_scope_since_it_shares_the_weekly_length() {
        let body = json!({
            "seven_day": { "utilization": 31.4 },
            "seven_day_opus": { "utilization": 12.5 },
        });
        assert_eq!(
            windows_of(&body),
            [
                (SEVEN_DAYS.as_secs(), WindowScope::Overall, 31.4),
                (SEVEN_DAYS.as_secs(), WindowScope::Opus, 12.5),
            ]
        );
    }

    #[test]
    fn parse_never_fills_plan_from_the_response() {
        // Plan info comes from the credential store — a response carrying
        // plan-ish keys must not populate it.
        let body = json!({ "five_hour": { "utilization": 1.0 }, "subscriptionType": "max" });
        assert_eq!(parse_usage(&body, None).plan, None);
    }

    #[test]
    fn parse_missing_windows_yields_an_empty_snapshot() {
        let usage = parse_usage(&json!({}), None);
        assert!(usage.windows.is_empty());
        assert!(usage.plan.is_none());
        assert!(usage.fetched_at.is_some());
    }

    #[test]
    fn parse_skips_a_null_window() {
        let body = json!({ "five_hour": null, "seven_day": { "utilization": 10.0 } });
        assert_eq!(
            windows_of(&body),
            [(SEVEN_DAYS.as_secs(), WindowScope::Overall, 10.0)]
        );
    }

    #[test]
    fn parse_keeps_a_window_without_a_reset_time() {
        let usage = parse_usage(&json!({ "five_hour": { "utilization": 50.0 } }), None);
        assert_eq!(usage.windows.len(), 1);
        assert!(usage.windows[0].resets_at.is_none());
    }

    #[test]
    fn parse_clamps_out_of_range_utilization() {
        let body = json!({
            "five_hour": { "utilization": -5.0 },
            "seven_day": { "utilization": 150.0 },
        });
        assert_eq!(
            windows_of(&body),
            [
                (FIVE_HOURS.as_secs(), WindowScope::Overall, 0.0),
                (SEVEN_DAYS.as_secs(), WindowScope::Overall, 100.0),
            ]
        );
    }

    #[test]
    fn parse_skips_a_window_missing_its_utilization() {
        let body = json!({ "five_hour": { "resets_at": "2026-05-07T15:00:00Z" } });
        assert!(parse_usage(&body, None).windows.is_empty());
    }

    #[test]
    fn fetch_for_a_config_dir_that_never_logged_in_is_no_token() {
        // No account ever logged in against this dir, so its scoped Keychain
        // item / `.credentials.json` was never written — the failure must map
        // to `NoToken`, not panic or leak an `AccountError`.
        let tmp = tempfile::tempdir().expect("tempdir");
        let err = ClaudeUsage
            .fetch(Some(&tmp.path().join("never-logged-in")))
            .unwrap_err();
        assert!(matches!(err, FetchError::NoToken));
    }
}
