//! Public API tests for usage claude.

use daruda_agent::usage::claude::*;
use serde_json::json;

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
fn parse_keeps_a_window_without_a_reset_time() {
    let usage = parse_usage(&json!({ "five_hour": { "utilization": 50.0 } }), None);
    assert_eq!(usage.windows.len(), 1);
    assert!(usage.windows[0].resets_at.is_none());
}

#[test]
fn parse_skips_a_window_missing_its_utilization() {
    let body = json!({ "five_hour": { "resets_at": "2026-05-07T15:00:00Z" } });
    assert!(parse_usage(&body, None).windows.is_empty());
}

use daruda_agent::{FetchError, usage::UsageSource};

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
