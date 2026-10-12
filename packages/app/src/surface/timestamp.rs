//! Wall-clock text for a list whose year the UI implies — the one shape the
//! app shell renders itself. The chat pane keeps its own stamps
//! (`daruda_chat::stamp`), since it is also the one that writes them down.
//!
//! The shape is a `chrono` pattern in the locale files, so a locale picks its
//! own field order and clock convention (en-US runs on a 12-hour clock, ko on
//! 24-hour). Durations and "n minutes ago" labels are a different axis and stay
//! in [`super::strings`].

use chrono::{DateTime, Local, TimeZone};
use std::fmt::Display;

/// A month, day and time, from an instant already in hand. No year: the lists
/// this labels only keep recent entries.
pub fn local_month_day_time<T: TimeZone>(when: DateTime<T>) -> String {
    in_zone(when, &Local)
}

/// Render an instant in `tz`. The zone is explicit so the tests can pin one
/// instead of inheriting whatever the build machine is set to.
fn in_zone<T, Z>(when: DateTime<T>, tz: &Z) -> String
where
    T: TimeZone,
    Z: TimeZone,
    Z::Offset: Display,
{
    let pattern = super::strings::timestamp::month_day_and_time();
    when.with_timezone(tz).format(&pattern).to_string()
}

#[cfg(test)]
mod tests;
