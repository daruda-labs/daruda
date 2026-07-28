//! Display labels for plan-rate windows.
//!
//! `daruda_claude` reports a window as a [`Duration`], not a name, so the
//! label is derived here — the one place both the status-bar chip and the
//! Usage tab read it from, so the two can't drift.

use std::time::Duration;

use daruda_claude::{UsageWindow, WindowScope};

use crate::surface::strings;

const MINUTE: u64 = 60;
const HOUR: u64 = 60 * MINUTE;
const DAY: u64 = 24 * HOUR;
/// Mean Gregorian month. Codex's monthly window reports 2_628_000 s, which is
/// this to within a rounding error, so it labels as "1mo" rather than "730h".
const MONTH: u64 = 2_629_746;
/// Longest window still labelled in days. Above this, months read better —
/// four weeks is the last length a user thinks of as "N days".
const MAX_DAYS: u64 = 28 * DAY;

/// Short label for a window, e.g. `"5h"`, `"7d"`, `"1mo"`, and `"7d · Opus"`
/// for a model-scoped one (which shares its length with the overall window,
/// so the duration alone would be ambiguous).
pub(in crate::workspace) fn window_label(window: &UsageWindow) -> String {
    let base = duration_label(window.window);
    match window.scope {
        WindowScope::Overall => base,
        WindowScope::Opus => format!("{base}{}", strings::usage_limit_opus_suffix()),
    }
}

/// Coarsest unit that divides `window` evenly, so a 5-hour window reads "5h"
/// rather than "300m". A length that divides no unit evenly rounds to the
/// nearest whole unit at its own magnitude.
fn duration_label(window: Duration) -> String {
    let secs = window.as_secs();
    if secs < HOUR {
        return strings::usage_limit_window_minutes(div_round(secs, MINUTE));
    }
    if secs < DAY {
        return strings::usage_limit_window_hours(div_round(secs, HOUR));
    }
    if secs <= MAX_DAYS {
        return strings::usage_limit_window_days(div_round(secs, DAY));
    }
    strings::usage_limit_window_months(div_round(secs, MONTH))
}

/// Nearest whole multiple, floored at 1 so a sub-unit window never labels as
/// "0h".
fn div_round(secs: u64, unit: u64) -> u64 {
    ((secs + unit / 2) / unit).max(1)
}

/// Utilization as the whole percent every surface displays.
pub(in crate::workspace) fn percent(utilization: f32) -> u32 {
    utilization.clamp(0.0, 100.0).round() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(secs: u64, scope: WindowScope) -> UsageWindow {
        UsageWindow {
            window: Duration::from_secs(secs),
            utilization: 0.0,
            resets_at: None,
            scope,
        }
    }

    #[test]
    fn anthropic_windows_keep_the_labels_they_had_before_the_duration_switch() {
        assert_eq!(window_label(&window(5 * HOUR, WindowScope::Overall)), "5h");
        assert_eq!(window_label(&window(7 * DAY, WindowScope::Overall)), "7d");
        assert_eq!(
            window_label(&window(7 * DAY, WindowScope::Opus)),
            "7d · Opus"
        );
    }

    /// Codex reports its monthly window as 2_628_000 s — a hair under a mean
    /// month, and not a whole number of days.
    #[test]
    fn the_codex_monthly_window_labels_as_one_month() {
        assert_eq!(
            window_label(&window(2_628_000, WindowScope::Overall)),
            "1mo"
        );
    }

    #[test]
    fn sub_hour_windows_label_in_minutes() {
        assert_eq!(
            window_label(&window(30 * MINUTE, WindowScope::Overall)),
            "30m"
        );
        // Floored at one unit rather than reading "0m".
        assert_eq!(window_label(&window(5, WindowScope::Overall)), "1m");
    }

    #[test]
    fn four_weeks_is_the_last_length_counted_in_days() {
        assert_eq!(window_label(&window(28 * DAY, WindowScope::Overall)), "28d");
        assert_eq!(window_label(&window(29 * DAY, WindowScope::Overall)), "1mo");
    }

    #[test]
    fn percent_rounds_to_a_whole_number_and_clamps() {
        assert_eq!(percent(41.6), 42);
        assert_eq!(percent(0.0), 0);
        assert_eq!(percent(100.0), 100);
        assert_eq!(percent(-3.0), 0);
        assert_eq!(percent(140.0), 100);
    }
}
