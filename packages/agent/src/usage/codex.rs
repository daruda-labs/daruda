//! Codex plan-limit windows from ChatGPT's `backend-api/wham/usage`.
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

const USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
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
        parse_usage(&body)
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

/// Parse the JSON body returned by `backend-api/wham/usage`. Split out so it
/// can be unit-tested against fixtures without a network mock.
///
/// Unlike Anthropic, the plan tier rides in the usage response rather than the
/// credential store, so it is read here. There is no equivalent of Anthropic's
/// rate-limit multiplier, so the qualifier stays empty.
///
/// Errors on a response this code cannot read — a missing `rate_limit` object,
/// or a window that is present but malformed. That is the line between "the
/// schema moved under us" (which must surface, so the numbers aren't quietly
/// replaced by "Unavailable") and "this account meters nothing right now"
/// (a null window, which is a real answer).
pub fn parse_usage(value: &Value) -> Result<ProviderUsage, FetchError> {
    let rate_limit = value
        .get("rate_limit")
        .filter(|v| v.is_object())
        .ok_or_else(|| FetchError::Parse("missing `rate_limit` object".into()))?;
    let mut windows = Vec::new();
    for name in ["primary_window", "secondary_window"] {
        if let Some(window) = parse_window(&rate_limit[name], name)? {
            windows.push(window);
        }
    }
    let plan = value["plan_type"].as_str().map(|tier| PlanInfo {
        tier: Some(tier.to_string()),
        qualifier: None,
    });
    Ok(ProviderUsage::new(AccountRecipeId::Codex, windows, plan))
}

/// `Ok(None)` for a window the response says isn't metered; an error for one it
/// reports but this code can't read.
///
/// `used_percent` arrives as an integer on some plans and a float on others, so
/// it is read as a number rather than either.
fn parse_window(value: &Value, name: &str) -> Result<Option<UsageWindow>, FetchError> {
    if value.is_null() {
        return Ok(None);
    }
    let field = |field: &str| FetchError::Parse(format!("{name}: unreadable `{field}`"));
    let utilization = value["used_percent"]
        .as_f64()
        .ok_or_else(|| field("used_percent"))? as f32;
    let window = value["limit_window_seconds"]
        .as_u64()
        .ok_or_else(|| field("limit_window_seconds"))?;
    Ok(Some(UsageWindow {
        window: Duration::from_secs(window),
        utilization: utilization.clamp(0.0, 100.0),
        resets_at: value["reset_at"].as_u64().map(|epoch| {
            // Unix seconds, where Anthropic sends RFC 3339.
            SystemTime::UNIX_EPOCH + Duration::from_secs(epoch)
        }),
        // Codex meters every model against one budget.
        scope: WindowScope::Overall,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn usage_url_matches_the_chatgpt_rate_limit_route() {
        assert_eq!(USAGE_URL, "https://chatgpt.com/backend-api/wham/usage");
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
    /// Run with `cargo test -p daruda_agent -- --ignored codex_live`.
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
