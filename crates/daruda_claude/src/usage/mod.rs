//! Plan-rate usage, modelled the same way for every auth domain.
//!
//! A provider reports some number of rolling windows, each with a length, a
//! utilization percentage, and a reset time. Nothing here names a specific
//! window: Anthropic's are 5-hour and 7-day, Codex's is monthly, and both are
//! just entries in [`ProviderUsage::windows`]. Display labels derive from
//! [`UsageWindow::window`] in the app layer, which owns i18n.

use std::time::{Duration, SystemTime};

use daruda_store::accounts::AccountRecipeId;

use crate::accounts::PlanInfo;
use crate::http::FetchError;

pub mod claude;
pub mod codex;

/// What a window meters, when its length alone doesn't say. Anthropic bills an
/// Opus-only 7-day budget alongside the overall 7-day one, so the two windows
/// are the same length and only this tells them apart.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum WindowScope {
    /// Every model on the plan.
    #[default]
    Overall,
    /// Opus-class models only.
    Opus,
}

/// One rolling rate-limit window.
#[derive(Clone, Debug, PartialEq)]
pub struct UsageWindow {
    /// Window length, e.g. 5 hours or 30 days. The display label derives from
    /// this rather than from a field name, so a provider can add a window of
    /// any length without a schema change.
    pub window: Duration,
    /// `0.0 ..= 100.0` — clamped at the parser boundary so renderers can skip
    /// their own guards.
    pub utilization: f32,
    /// Wall-clock when the window resets; `None` when the API omits it.
    pub resets_at: Option<SystemTime>,
    pub scope: WindowScope,
}

impl UsageWindow {
    /// Whether the window is fully spent and refusing further work. Callers
    /// promote such a window over a shorter one, since it blocks regardless
    /// of the short window's headroom.
    pub fn is_spent(&self) -> bool {
        self.utilization >= LIMIT_REACHED_PERCENT
    }

    /// Ordering key: shortest first, and for equal lengths the overall window
    /// before a model-scoped one — the broader limit is the one worth naming.
    fn sort_key(&self) -> (Duration, WindowScope) {
        (self.window, self.scope)
    }
}

/// One account's plan-rate snapshot in one auth domain.
#[derive(Clone, Debug, PartialEq)]
pub struct ProviderUsage {
    pub recipe: AccountRecipeId,
    /// Ascending by [`UsageWindow::sort_key`]. Empty when the provider
    /// reported no window at all — a valid answer, not an error.
    pub windows: Vec<UsageWindow>,
    /// Subscription metadata, from the local credential store rather than the
    /// usage response.
    pub plan: Option<PlanInfo>,
    /// Wall-clock when the response was decoded.
    pub fetched_at: Option<SystemTime>,
}

impl ProviderUsage {
    /// Build a snapshot, sorting `windows` into display order so no caller
    /// has to.
    pub fn new(
        recipe: AccountRecipeId,
        mut windows: Vec<UsageWindow>,
        plan: Option<PlanInfo>,
    ) -> Self {
        windows.sort_by_key(|w| w.sort_key());
        Self {
            recipe,
            windows,
            plan,
            fetched_at: Some(SystemTime::now()),
        }
    }

    /// The window whose utilization the caller should report when it has room
    /// for exactly one: the longest spent window if any is spent, else the
    /// shortest. Keeping to the shortest window by default means the reported
    /// meter doesn't change name as the numbers cross each other; a spent
    /// window overrides because it blocks work outright.
    pub fn headline_window(&self) -> Option<&UsageWindow> {
        self.windows
            .iter()
            .filter(|w| w.is_spent())
            .max_by(|a, b| {
                // Longest wins; on equal length prefer the overall window,
                // which `sort_key` already orders first.
                a.window.cmp(&b.window).then_with(|| b.scope.cmp(&a.scope))
            })
            .or_else(|| self.windows.first())
    }
}

/// What the latest poll established about one auth domain's usage. A domain
/// nobody is signed into must disappear from the UI rather than show empty
/// gauges, while a domain that *is* signed in and merely failed to refresh
/// must keep showing its last numbers — otherwise every network hiccup makes
/// the surface flicker out and back.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum UsageOutcome {
    /// No poll has landed yet. Renders nothing at all, not a placeholder:
    /// the first tick is close behind and a flash of empty chrome is worse
    /// than a beat of absence.
    #[default]
    Pending,
    /// The credential store holds nothing for this domain, so the user is not
    /// signed in. The chip and the tab section disappear.
    SignedOut,
    /// Signed in, but the fetch failed — an expired token (which comes back as
    /// an HTTP 401, not a missing credential), a network blip, a schema
    /// change. Carries the last good snapshot when there is one so the numbers
    /// stay on screen, marked stale.
    Failed {
        last: Option<ProviderUsage>,
    },
    Ok(ProviderUsage),
}

impl UsageOutcome {
    /// Fold a fetch result into the next outcome, carrying the current
    /// snapshot forward when the fetch failed. The single place the three
    /// states are assigned, so no caller re-derives the rules.
    pub fn advance(self, result: Result<ProviderUsage, FetchError>) -> Self {
        match result {
            Ok(usage) => Self::Ok(usage),
            // A vanished credential means signed out, whatever was cached
            // before: showing the previous account's numbers would be a lie.
            Err(FetchError::NoToken) => Self::SignedOut,
            Err(_) => Self::Failed {
                last: self.into_snapshot(),
            },
        }
    }

    /// Everything a surface draws from this outcome, as a comparable key.
    /// `fetched_at` is deliberately absent — it moves every poll, so including
    /// it would repaint on every tick. Whoever adds a rendered field to
    /// [`ProviderUsage`] adds it here, or the change won't reach the screen.
    pub fn rendered_key(&self) -> (Option<&[UsageWindow]>, Option<&PlanInfo>, bool, bool) {
        (
            self.snapshot().map(|u| u.windows.as_slice()),
            self.snapshot().and_then(|u| u.plan.as_ref()),
            self.is_stale(),
            self.is_signed_out(),
        )
    }

    /// The snapshot to render, if any.
    pub fn snapshot(&self) -> Option<&ProviderUsage> {
        match self {
            Self::Ok(usage) => Some(usage),
            Self::Failed { last } => last.as_ref(),
            Self::Pending | Self::SignedOut => None,
        }
    }

    fn into_snapshot(self) -> Option<ProviderUsage> {
        match self {
            Self::Ok(usage) => Some(usage),
            Self::Failed { last } => last,
            Self::Pending | Self::SignedOut => None,
        }
    }

    /// Whether there are numbers to render. Deliberately not named for
    /// visibility: each surface decides that for itself — the status-bar pill
    /// needs numbers to be worth its width, while the Usage tab keeps a section
    /// for a `Pending` domain so the layout doesn't shift when the first poll
    /// lands. The one rule both share is [`Self::is_signed_out`].
    pub fn has_numbers(&self) -> bool {
        self.snapshot().is_some()
    }

    /// Whether what is on screen is known to be out of date, so the surface
    /// can mark it.
    pub fn is_stale(&self) -> bool {
        matches!(self, Self::Failed { last: Some(_) })
    }

    /// Whether the last poll proved nobody is signed into this domain.
    pub fn is_signed_out(&self) -> bool {
        matches!(self, Self::SignedOut)
    }
}

/// Utilization at which a window is spent and further prompts are refused.
pub const LIMIT_REACHED_PERCENT: f32 = 100.0;
/// Boundary at which a gauge flips from green to yellow.
pub const LIMIT_MEDIUM_THRESHOLD: f32 = 50.0;
/// Boundary at which a gauge flips from yellow to red.
pub const LIMIT_HIGH_THRESHOLD: f32 = 80.0;

/// Severity bucket for a utilization value, driving gauge and chip colour.
/// Thresholds match the Übersicht widget these surfaces are modelled on.
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
    /// Map a utilization percentage (0–100) to a bucket. NaN and negatives
    /// fall to `Low` so a malformed API value never trips a bogus alert.
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

/// Where one auth domain's usage comes from. One impl per domain, resolved
/// through [`source_for`] so call sites stay free of per-domain branching.
pub trait UsageSource: Send + Sync {
    /// Fetch and parse this domain's plan-rate snapshot. `config_dir` is a
    /// managed account's isolated home, `None` the ambient system login.
    /// Blocking (HTTP + credential-store read) — call from a background
    /// thread.
    fn fetch(&self, config_dir: Option<&std::path::Path>) -> Result<ProviderUsage, FetchError>;

    /// Public status page for this domain's provider. Account-independent,
    /// and every one so far serves the Statuspage v2 schema that
    /// `service_status::parse_service_status` reads.
    fn status_url(&self) -> &'static str;
}

/// The usage source for `id`. Total, like `accounts::recipe_for`.
pub fn source_for(id: AccountRecipeId) -> &'static dyn UsageSource {
    match id {
        AccountRecipeId::Claude => &claude::ClaudeUsage,
        AccountRecipeId::Codex => &codex::CodexUsage,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
