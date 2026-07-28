//! Codex plan-limit windows from ChatGPT's `backend-api/codex/usage`.
//!
//! Up to two windows (`primary_window` / `secondary_window`), each reporting
//! its own length — a monthly one on a Team plan, shorter ones elsewhere — so
//! nothing here assumes a fixed set. Credentials are the plaintext
//! `auth.json` inside the account's `CODEX_HOME`, the same file
//! `accounts::codex` reads identity from; a missing or tokenless one collapses
//! to [`FetchError::NoToken`].
//!
//! The adapter cannot supply this: `codex-acp` forwards `token_count` events
//! but drops their `rate_limits`, so polling is the only path.

use std::path::Path;
use std::time::{Duration, SystemTime};

use daruda_store::accounts::AccountRecipeId;
use serde_json::Value;

use super::{ProviderUsage, UsageSource, UsageWindow, WindowScope};
use crate::accounts::PlanInfo;
use crate::accounts::codex::system_codex_home;
use crate::http::{FetchError, Header, get_json};

const USAGE_URL: &str = "https://chatgpt.com/backend-api/codex/usage";
const STATUS_URL: &str = "https://status.openai.com/api/v2/status.json";

/// Plaintext credential file codex writes inside its `CODEX_HOME`.
const AUTH_FILE: &str = "auth.json";

pub struct CodexUsage;

impl UsageSource for CodexUsage {
    fn fetch(&self, config_dir: Option<&Path>) -> Result<ProviderUsage, FetchError> {
        let home = match config_dir {
            Some(dir) => dir.to_path_buf(),
            None => system_codex_home().ok_or(FetchError::NoToken)?,
        };
        let credentials = read_credentials(&home)?;
        let auth = format!("Bearer {}", credentials.access_token);
        // The account header scopes the response to the right workspace on a
        // login that belongs to several; omitted when the file has no id.
        let mut headers: Vec<Header<'_>> = vec![("Authorization", &auth)];
        if let Some(account_id) = credentials.account_id.as_deref() {
            headers.push(("ChatGPT-Account-Id", account_id));
        }
        let body = get_json(USAGE_URL, &headers)?;
        Ok(parse_usage(&body))
    }

    fn status_url(&self) -> &'static str {
        STATUS_URL
    }
}

/// Deliberately not `Debug`: it carries a bearer token, and a derived impl
/// would print it into any panic message that touches it.
struct Credentials {
    access_token: String,
    account_id: Option<String>,
}

/// Read the ChatGPT OAuth token out of `<home>/auth.json`. Every failure —
/// no file, malformed JSON, no token — is [`FetchError::NoToken`], which is
/// what tells the caller the domain is signed out rather than broken.
fn read_credentials(home: &Path) -> Result<Credentials, FetchError> {
    let raw = std::fs::read_to_string(home.join(AUTH_FILE)).map_err(|_| FetchError::NoToken)?;
    let auth: Value = serde_json::from_str(&raw).map_err(|_| FetchError::NoToken)?;
    let tokens = &auth["tokens"];
    Ok(Credentials {
        access_token: tokens["access_token"]
            .as_str()
            .map(str::to_string)
            .ok_or(FetchError::NoToken)?,
        account_id: tokens["account_id"].as_str().map(str::to_string),
    })
}

/// Parse the JSON body returned by `backend-api/codex/usage`. Split out so it
/// can be unit-tested against fixtures without a network mock.
///
/// Unlike Anthropic, the plan tier rides in the usage response rather than the
/// credential store, so it is read here. There is no equivalent of Anthropic's
/// rate-limit multiplier, so the qualifier stays empty.
pub fn parse_usage(value: &Value) -> ProviderUsage {
    let rate_limit = &value["rate_limit"];
    let windows = [
        &rate_limit["primary_window"],
        &rate_limit["secondary_window"],
    ]
    .into_iter()
    .filter_map(parse_window)
    .collect();
    let plan = value["plan_type"].as_str().map(|tier| PlanInfo {
        tier: Some(tier.to_string()),
        qualifier: None,
    });
    ProviderUsage::new(AccountRecipeId::Codex, windows, plan)
}

/// `used_percent` arrives as an integer on some plans and a float on others,
/// so it is read as a number rather than either. A window without one is not
/// a window — the field is the whole point.
fn parse_window(value: &Value) -> Option<UsageWindow> {
    if value.is_null() {
        return None;
    }
    let utilization = value["used_percent"].as_f64()? as f32;
    Some(UsageWindow {
        window: Duration::from_secs(value["limit_window_seconds"].as_u64()?),
        utilization: utilization.clamp(0.0, 100.0),
        resets_at: value["reset_at"].as_u64().map(|epoch| {
            // Unix seconds, where Anthropic sends RFC 3339.
            SystemTime::UNIX_EPOCH + Duration::from_secs(epoch)
        }),
        // Codex meters every model against one budget.
        scope: WindowScope::Overall,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The shape a live Team-plan account returned on 2026-07-28: one monthly
    /// window, no secondary.
    fn team_plan_response() -> Value {
        json!({
            "user_id": "user-abc",
            "account_id": "ce62a47c-2346-4c67-a552-e94a60e0a946",
            "email": "someone@example.com",
            "plan_type": "team",
            "rate_limit": {
                "allowed": true,
                "limit_reached": false,
                "primary_window": {
                    "used_percent": 2,
                    "limit_window_seconds": 2_628_000,
                    "reset_after_seconds": 2_621_510,
                    "reset_at": 1_787_849_085_u64,
                },
                "secondary_window": Value::Null,
            },
            "credits": { "has_credits": true, "unlimited": false },
        })
    }

    #[test]
    fn a_team_plan_reports_one_monthly_window() {
        let usage = parse_usage(&team_plan_response());
        assert_eq!(usage.recipe, AccountRecipeId::Codex);
        assert_eq!(usage.windows.len(), 1);
        let window = &usage.windows[0];
        assert_eq!(window.window, Duration::from_secs(2_628_000));
        assert_eq!(window.utilization, 2.0);
        assert_eq!(window.scope, WindowScope::Overall);
        assert_eq!(
            window.resets_at,
            Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1_787_849_085))
        );
        assert_eq!(
            usage.plan.as_ref().and_then(|p| p.tier.as_deref()),
            Some("team")
        );
        // No Anthropic-style multiplier exists to qualify the tier with.
        assert!(usage.plan.as_ref().is_some_and(|p| p.qualifier.is_none()));
    }

    #[test]
    fn both_windows_are_reported_shortest_first() {
        let mut body = team_plan_response();
        body["rate_limit"]["secondary_window"] = json!({
            "used_percent": 40.5,
            "limit_window_seconds": 18_000,
            "reset_at": 1_787_800_000_u64,
        });
        let usage = parse_usage(&body);
        let lengths: Vec<u64> = usage.windows.iter().map(|w| w.window.as_secs()).collect();
        assert_eq!(lengths, [18_000, 2_628_000]);
        assert_eq!(usage.windows[0].utilization, 40.5);
    }

    #[test]
    fn a_response_without_rate_limits_yields_an_empty_snapshot() {
        let usage = parse_usage(&json!({ "plan_type": "plus" }));
        assert!(usage.windows.is_empty());
        // Still a real answer — the plan is known even with no window metered.
        assert_eq!(
            usage.plan.as_ref().and_then(|p| p.tier.as_deref()),
            Some("plus")
        );
    }

    #[test]
    fn a_window_missing_its_length_or_percent_is_dropped() {
        for window in [
            json!({ "used_percent": 5 }),
            json!({ "limit_window_seconds": 18_000 }),
        ] {
            let mut body = team_plan_response();
            body["rate_limit"]["primary_window"] = window;
            body["rate_limit"]["secondary_window"] = Value::Null;
            assert!(parse_usage(&body).windows.is_empty());
        }
    }

    #[test]
    fn a_window_without_a_reset_time_still_counts() {
        let mut body = team_plan_response();
        body["rate_limit"]["primary_window"] =
            json!({ "used_percent": 7, "limit_window_seconds": 18_000 });
        let usage = parse_usage(&body);
        assert_eq!(usage.windows.len(), 1);
        assert!(usage.windows[0].resets_at.is_none());
    }

    #[test]
    fn out_of_range_percentages_are_clamped() {
        let mut body = team_plan_response();
        body["rate_limit"]["primary_window"] =
            json!({ "used_percent": 140, "limit_window_seconds": 18_000 });
        assert_eq!(parse_usage(&body).windows[0].utilization, 100.0);
    }

    #[test]
    fn an_unknown_plan_leaves_the_badge_empty_rather_than_guessing() {
        let mut body = team_plan_response();
        body["plan_type"] = Value::Null;
        assert!(parse_usage(&body).plan.is_none());
    }

    #[test]
    fn a_home_without_auth_json_is_signed_out() {
        let tmp = tempfile::tempdir().expect("tempdir");
        assert!(matches!(
            read_credentials(tmp.path()),
            Err(FetchError::NoToken)
        ));
        assert!(matches!(
            CodexUsage.fetch(Some(tmp.path())),
            Err(FetchError::NoToken)
        ));
    }

    #[test]
    fn a_malformed_or_tokenless_auth_json_is_signed_out() {
        let tmp = tempfile::tempdir().expect("tempdir");
        for contents in [
            "{not json".to_string(),
            json!({ "tokens": {} }).to_string(),
            json!({ "auth_mode": "apikey" }).to_string(),
        ] {
            std::fs::write(tmp.path().join(AUTH_FILE), &contents).expect("write auth.json");
            assert!(
                matches!(read_credentials(tmp.path()), Err(FetchError::NoToken)),
                "accepted: {contents}"
            );
        }
    }

    /// Live round-trip against the real endpoint with whatever codex login is
    /// on this machine. `#[ignore]`d: it needs network and a signed-in
    /// `~/.codex`, so it is a hand-run check that the URL, headers and parse
    /// still match what ChatGPT serves — the one thing fixtures cannot prove.
    /// Run with `cargo test -p daruda_claude -- --ignored codex_live`.
    #[test]
    #[ignore = "requires network and a signed-in codex home"]
    fn codex_live_usage_round_trip() {
        match CodexUsage.fetch(None) {
            Ok(usage) => {
                assert_eq!(usage.recipe, AccountRecipeId::Codex);
                assert!(
                    !usage.windows.is_empty(),
                    "a signed-in account should meter at least one window; got {usage:?}"
                );
                for window in &usage.windows {
                    assert!(window.window > Duration::ZERO);
                    assert!((0.0..=100.0).contains(&window.utilization));
                }
            }
            Err(FetchError::NoToken) => {
                panic!("no codex login on this machine — sign in before running this check")
            }
            Err(e) => panic!("live fetch failed: {e}"),
        }
    }

    #[test]
    fn credentials_carry_the_account_id_when_present() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let auth = json!({
            "tokens": { "access_token": "tok", "account_id": "acct-1", "id_token": "x" },
        });
        std::fs::write(tmp.path().join(AUTH_FILE), auth.to_string()).expect("write");
        let credentials = read_credentials(tmp.path()).expect("readable");
        assert_eq!(credentials.access_token, "tok");
        assert_eq!(credentials.account_id.as_deref(), Some("acct-1"));

        // A login with no account id still authenticates.
        let auth = json!({ "tokens": { "access_token": "tok" } });
        std::fs::write(tmp.path().join(AUTH_FILE), auth.to_string()).expect("write");
        assert_eq!(
            read_credentials(tmp.path()).expect("readable").account_id,
            None
        );
    }
}
