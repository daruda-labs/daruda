//! Public API tests for usage codex.

use daruda_agent::usage::codex::*;
use serde_json::Value;
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
fn both_windows_are_reported_shortest_first() {
    let mut body = team_plan_response();
    body["rate_limit"]["secondary_window"] = json!({
        "used_percent": 40.5,
        "limit_window_seconds": 18_000,
        "reset_at": 1_787_800_000_u64,
    });
    let usage = parse_usage(&body).expect("two valid windows parse");
    let lengths: Vec<u64> = usage.windows.iter().map(|w| w.window.as_secs()).collect();
    assert_eq!(lengths, [18_000, 2_628_000]);
    assert_eq!(usage.windows[0].utilization, 40.5);
}

/// Both windows null is a real answer — this account meters nothing right
/// now — so it parses, unlike the missing-object case above.
#[test]
fn null_windows_parse_as_an_account_metering_nothing() {
    let body = json!({
        "plan_type": "plus",
        "rate_limit": { "primary_window": Value::Null, "secondary_window": Value::Null },
    });
    let usage = parse_usage(&body).expect("null windows are a real answer");
    assert!(usage.windows.is_empty());
    assert_eq!(
        usage.plan.as_ref().and_then(|p| p.tier.as_deref()),
        Some("plus")
    );
}

#[test]
fn a_window_without_a_reset_time_still_counts() {
    let mut body = team_plan_response();
    body["rate_limit"]["primary_window"] =
        json!({ "used_percent": 7, "limit_window_seconds": 18_000 });
    let usage = parse_usage(&body).expect("a reset time is optional");
    assert_eq!(usage.windows.len(), 1);
    assert!(usage.windows[0].resets_at.is_none());
}

#[test]
fn out_of_range_percentages_are_clamped() {
    let mut body = team_plan_response();
    body["rate_limit"]["primary_window"] =
        json!({ "used_percent": 140, "limit_window_seconds": 18_000 });
    let usage = parse_usage(&body).expect("an out-of-range percent still parses");
    assert_eq!(usage.windows[0].utilization, 100.0);
}

#[test]
fn an_unknown_plan_leaves_the_badge_empty_rather_than_guessing() {
    let mut body = team_plan_response();
    body["plan_type"] = Value::Null;
    assert!(
        parse_usage(&body)
            .expect("an absent plan is not a parse failure")
            .plan
            .is_none()
    );
}

use daruda_agent::{FetchError, WindowScope};
use daruda_store::accounts::AccountRecipeId;
use std::time::{Duration, SystemTime};

#[test]
fn a_team_plan_reports_one_monthly_window() {
    let usage = parse_usage(&team_plan_response()).expect("the live shape parses");
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

/// A response with no `rate_limit` at all is the schema having moved, not an
/// idle account: surfacing it keeps the tab from replacing real numbers with
/// "Unavailable" as though the refresh had succeeded.
#[test]
fn a_response_without_rate_limits_is_a_parse_error() {
    assert!(matches!(
        parse_usage(&json!({ "plan_type": "plus" })),
        Err(FetchError::Parse(_))
    ));
}

#[test]
fn a_window_missing_its_length_or_percent_is_a_parse_error() {
    for window in [
        json!({ "used_percent": 5 }),
        json!({ "limit_window_seconds": 18_000 }),
        json!({ "used_percent": "2%", "limit_window_seconds": 18_000 }),
    ] {
        let mut body = team_plan_response();
        body["rate_limit"]["primary_window"] = window.clone();
        body["rate_limit"]["secondary_window"] = Value::Null;
        assert!(
            matches!(parse_usage(&body), Err(FetchError::Parse(_))),
            "accepted a malformed window: {window}"
        );
    }
}

/// A malformed *secondary* window is as much a schema break as a primary
/// one — it must not be skipped while the primary still parses.
#[test]
fn a_malformed_secondary_window_is_not_silently_skipped() {
    let mut body = team_plan_response();
    body["rate_limit"]["secondary_window"] = json!({ "used_percent": 40 });
    assert!(matches!(parse_usage(&body), Err(FetchError::Parse(_))));
}
