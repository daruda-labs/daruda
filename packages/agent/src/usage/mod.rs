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
    /// Ascending by `UsageWindow::sort_key`. Empty when the provider
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
    crate::providers::integration_for(id).usage
}
