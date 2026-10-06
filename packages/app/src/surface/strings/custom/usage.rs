use crate::surface::strings as s;

/// Body shown when the focused pane's own domain isn't signed in — even
/// though another domain might be. Names `recipe` specifically rather than
/// the generic [`s::usage::no_provider`] notice, which would misleadingly claim
/// nobody is signed into anything.
pub(crate) fn no_domain_provider(recipe: daruda_store::accounts::AccountRecipeId) -> String {
    rust_i18n::t!("usage.no_domain_provider", domain => s::settings::account_recipe_label(recipe))
        .into_owned()
}

/// Weekday abbreviation for a chart bar. `idx` is 0=Sunday .. 6=Saturday
/// (matching `chrono::Weekday::num_days_from_sunday`). Out-of-range
/// values fall back to an empty label rather than panicking.
pub(crate) fn weekday_label(idx: u8) -> String {
    match idx {
        0 => s::usage::weekday_sun(),
        1 => s::usage::weekday_mon(),
        2 => s::usage::weekday_tue(),
        3 => s::usage::weekday_wed(),
        4 => s::usage::weekday_thu(),
        5 => s::usage::weekday_fri(),
        6 => s::usage::weekday_sat(),
        _ => String::new(),
    }
}

/// Format a "resets in …" countdown for the gauge subtitle. Lives
/// here (not on `Duration`) so the precise rounding behaviour is
/// covered by tests next to the rest of the surface strings.
///
/// Buckets:
/// - `≥ 1 day` → `"<d>d <h>h"`
/// - `≥ 1 hour` → `"<h>h <m>m"`
/// - `≥ 1 minute` → `"<m>m"`
/// - `1..60 seconds` → `"<1m"` (avoids the awkward "Resets in 0m"
///   that would linger for a whole minute before the reset)
/// - `0` → `"now"`
///
/// Always prefixed with `"Resets in "` so callers concatenate a
/// single string and don't have to reason about pluralization.
pub(crate) fn format_reset_countdown(remaining: std::time::Duration) -> String {
    if remaining.as_secs() == 0 {
        return s::usage::reset_now();
    }
    std::borrow::Cow::<str>::Owned(s::usage::reset_in(s::usage::format_reset_short(remaining)))
        .into_owned()
}

/// The same buckets as [`s::usage::format_reset_countdown`] without the "Resets"
/// prefix — for the status-bar usage chip, where the surrounding chip
/// already says what the number is and the sentence form would not fit.
/// `0` renders as `"now"`.
pub(crate) fn format_reset_short(remaining: std::time::Duration) -> String {
    let secs = remaining.as_secs();
    if secs == 0 {
        return s::usage::reset_short_now();
    }
    let mins_total = secs / 60;
    let hours_total = mins_total / 60;
    let days_total = hours_total / 24;
    if days_total >= 1 {
        std::borrow::Cow::<str>::Owned(s::usage::reset_short_days(days_total, hours_total % 24))
            .into_owned()
    } else if hours_total >= 1 {
        std::borrow::Cow::<str>::Owned(s::usage::reset_short_hours(hours_total, mins_total % 60))
            .into_owned()
    } else if mins_total >= 1 {
        s::usage::reset_short_minutes(mins_total)
    } else {
        s::usage::reset_short_under_minute()
    }
}
