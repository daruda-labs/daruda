//! Tests for the public `daruda_agent::usage` API.

use daruda_agent::usage::*;
use daruda_agent::{FetchError, PlanInfo};
use daruda_store::accounts::AccountRecipeId;
use std::time::Duration;

fn window(secs: u64, utilization: f32, scope: WindowScope) -> UsageWindow {
    UsageWindow {
        window: Duration::from_secs(secs),
        utilization,
        resets_at: None,
        scope,
    }
}

const FIVE_HOURS: u64 = 5 * 3600;
const SEVEN_DAYS: u64 = 7 * 24 * 3600;
const ONE_MONTH: u64 = 2_628_000;

#[test]
fn new_sorts_windows_shortest_first() {
    let usage = ProviderUsage::new(
        AccountRecipeId::Claude,
        vec![
            window(SEVEN_DAYS, 30.0, WindowScope::Overall),
            window(FIVE_HOURS, 10.0, WindowScope::Overall),
        ],
        None,
    );
    let lengths: Vec<u64> = usage.windows.iter().map(|w| w.window.as_secs()).collect();
    assert_eq!(lengths, [FIVE_HOURS, SEVEN_DAYS]);
}

#[test]
fn new_orders_the_overall_window_before_a_model_scoped_one_of_equal_length() {
    let usage = ProviderUsage::new(
        AccountRecipeId::Claude,
        vec![
            window(SEVEN_DAYS, 12.0, WindowScope::Opus),
            window(SEVEN_DAYS, 30.0, WindowScope::Overall),
        ],
        None,
    );
    let scopes: Vec<WindowScope> = usage.windows.iter().map(|w| w.scope).collect();
    assert_eq!(scopes, [WindowScope::Overall, WindowScope::Opus]);
}

#[test]
fn headline_is_the_shortest_window_while_none_is_spent() {
    // 99.9% weekly still leaves work possible, so the meter does not swap.
    let usage = ProviderUsage::new(
        AccountRecipeId::Claude,
        vec![
            window(FIVE_HOURS, 12.0, WindowScope::Overall),
            window(SEVEN_DAYS, 99.9, WindowScope::Overall),
            window(SEVEN_DAYS, 98.0, WindowScope::Opus),
        ],
        None,
    );
    let headline = usage.headline_window().expect("windows are present");
    assert_eq!(headline.window.as_secs(), FIVE_HOURS);
    assert_eq!(headline.utilization, 12.0);
}

#[test]
fn a_spent_window_takes_over_the_headline() {
    let usage = ProviderUsage::new(
        AccountRecipeId::Claude,
        vec![
            window(FIVE_HOURS, 3.0, WindowScope::Overall),
            window(SEVEN_DAYS, 100.0, WindowScope::Overall),
        ],
        None,
    );
    assert_eq!(
        usage.headline_window().unwrap().window.as_secs(),
        SEVEN_DAYS
    );
}

#[test]
fn a_spent_scoped_window_wins_only_while_the_overall_one_has_room() {
    let scoped_only = ProviderUsage::new(
        AccountRecipeId::Claude,
        vec![
            window(FIVE_HOURS, 3.0, WindowScope::Overall),
            window(SEVEN_DAYS, 40.0, WindowScope::Overall),
            window(SEVEN_DAYS, 100.0, WindowScope::Opus),
        ],
        None,
    );
    assert_eq!(
        scoped_only.headline_window().unwrap().scope,
        WindowScope::Opus
    );

    // Both spent — the broader block is the one worth naming.
    let both = ProviderUsage::new(
        AccountRecipeId::Claude,
        vec![
            window(FIVE_HOURS, 3.0, WindowScope::Overall),
            window(SEVEN_DAYS, 100.0, WindowScope::Overall),
            window(SEVEN_DAYS, 100.0, WindowScope::Opus),
        ],
        None,
    );
    assert_eq!(both.headline_window().unwrap().scope, WindowScope::Overall);
}

/// A provider metering one long window (Codex bills monthly) still gets a
/// headline: the rule keys on window length, not on a particular window
/// existing, so such a provider is not left without a chip.
#[test]
fn a_lone_long_window_is_still_the_headline() {
    let usage = ProviderUsage::new(
        AccountRecipeId::Codex,
        vec![window(ONE_MONTH, 2.0, WindowScope::Overall)],
        None,
    );
    assert_eq!(usage.headline_window().unwrap().window.as_secs(), ONE_MONTH);
}

#[test]
fn no_windows_means_no_headline() {
    let usage = ProviderUsage::new(AccountRecipeId::Claude, Vec::new(), None);
    assert!(usage.headline_window().is_none());
}

fn snapshot() -> ProviderUsage {
    ProviderUsage::new(
        AccountRecipeId::Claude,
        vec![window(FIVE_HOURS, 42.0, WindowScope::Overall)],
        None,
    )
}

#[test]
fn a_missing_credential_means_signed_out() {
    let outcome = UsageOutcome::Pending.advance(Err(FetchError::NoToken));
    assert!(outcome.is_signed_out());
    assert!(!outcome.has_numbers());
}

/// The user signed out after a good poll: the cached numbers belong to an
/// account that is no longer there, so they must go rather than linger.
#[test]
fn signing_out_discards_the_cached_snapshot() {
    let outcome = UsageOutcome::Ok(snapshot()).advance(Err(FetchError::NoToken));
    assert_eq!(outcome, UsageOutcome::SignedOut);
    assert!(outcome.snapshot().is_none());
}

/// An expired token arrives as an HTTP 401, not a missing credential — the
/// user is still signed in, so the numbers stay put and get marked stale.
#[test]
fn a_failed_refresh_keeps_the_last_good_snapshot_and_marks_it_stale() {
    let good = snapshot();
    let outcome = UsageOutcome::Ok(good.clone()).advance(Err(FetchError::Http("401".into())));
    assert!(outcome.has_numbers());
    assert!(outcome.is_stale());
    assert_eq!(outcome.snapshot(), Some(&good));
}

#[test]
fn a_failure_before_any_success_shows_nothing_and_is_not_stale() {
    let outcome = UsageOutcome::Pending.advance(Err(FetchError::Parse("bad".into())));
    assert_eq!(outcome, UsageOutcome::Failed { last: None });
    assert!(!outcome.has_numbers());
    assert!(!outcome.is_stale());
    assert!(!outcome.is_signed_out());
}

#[test]
fn repeated_failures_do_not_lose_the_snapshot() {
    let good = snapshot();
    let outcome = UsageOutcome::Ok(good.clone())
        .advance(Err(FetchError::Http("500".into())))
        .advance(Err(FetchError::Http("500".into())));
    assert_eq!(outcome.snapshot(), Some(&good));
}

#[test]
fn a_success_clears_staleness() {
    let outcome = UsageOutcome::Failed {
        last: Some(snapshot()),
    }
    .advance(Ok(snapshot()));
    assert!(!outcome.is_stale());
    assert!(outcome.has_numbers());
}

#[test]
fn pending_shows_nothing_without_claiming_a_reason() {
    let pending = UsageOutcome::default();
    assert_eq!(pending, UsageOutcome::Pending);
    assert!(!pending.has_numbers());
    assert!(!pending.is_signed_out());
    assert!(!pending.is_stale());
}

/// The plan badge is on screen next to the gauges, so a tier change with
/// identical windows still has to reach it — the repaint gate compares this
/// key, and leaving `plan` out of it left the badge stale.
#[test]
fn the_rendered_key_notices_a_plan_change_with_unchanged_windows() {
    let windows = vec![window(FIVE_HOURS, 42.0, WindowScope::Overall)];
    let plan = |tier: &str| {
        Some(PlanInfo {
            tier: Some(tier.to_string()),
            qualifier: None,
        })
    };
    let pro = UsageOutcome::Ok(ProviderUsage::new(
        AccountRecipeId::Claude,
        windows.clone(),
        plan("pro"),
    ));
    let team = UsageOutcome::Ok(ProviderUsage::new(
        AccountRecipeId::Claude,
        windows.clone(),
        plan("team"),
    ));
    let same = UsageOutcome::Ok(ProviderUsage::new(
        AccountRecipeId::Claude,
        windows,
        plan("pro"),
    ));
    assert_ne!(pro.rendered_key(), team.rendered_key());
    // A re-poll that changed nothing visible must not repaint, even though
    // `fetched_at` moved.
    assert_eq!(pro.rendered_key(), same.rendered_key());
}

#[test]
fn severity_thresholds_bucket_at_fifty_and_eighty() {
    assert_eq!(LimitSeverity::from_utilization(0.0), LimitSeverity::Low);
    assert_eq!(LimitSeverity::from_utilization(49.999), LimitSeverity::Low);
    assert_eq!(LimitSeverity::from_utilization(50.0), LimitSeverity::Medium);
    assert_eq!(
        LimitSeverity::from_utilization(79.999),
        LimitSeverity::Medium
    );
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
fn is_spent_only_at_a_hundred_percent() {
    assert!(!window(FIVE_HOURS, 99.9, WindowScope::Overall).is_spent());
    assert!(window(FIVE_HOURS, 100.0, WindowScope::Overall).is_spent());
}
