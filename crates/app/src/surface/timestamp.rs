//! Wall-clock timestamps — the single place a date or a time becomes text a
//! user reads, and the single place one is written down to be read back later.
//! Both directions live here because they have to agree on a format: whatever
//! [`now_rfc3339`] writes is what [`local_datetime`] has to parse.
//!
//! Every shape is a `chrono` pattern in the locale files, so a locale picks its
//! own field order and clock convention (en-US runs on a 12-hour clock, ko on
//! 24-hour). Durations and "n minutes ago" labels are a different axis and stay
//! in [`super::strings`].

use chrono::{DateTime, Local, TimeZone, Utc};
use std::fmt::Display;

/// Which wall-clock shape a timestamp is rendered in — one variant per locale
/// pattern. Private: call sites name the shape they want through the functions
/// below rather than passing this, so adding a shape is one variant, one locale
/// key and one wrapper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    /// Full date and time, for a timestamp that has to stand alone.
    DateAndTime,
    /// Month, day and time, for one whose year the surrounding UI implies.
    MonthDayAndTime,
}

impl Shape {
    fn pattern(self) -> String {
        match self {
            Self::DateAndTime => super::strings::timestamp_date_and_time(),
            Self::MonthDayAndTime => super::strings::timestamp_month_day_and_time(),
        }
    }
}

/// Stamp "now" for a field that is stored and parsed back, not displayed.
/// UTC, matching what agents send over ACP and what the rest of the codebase
/// writes down.
pub fn now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}

/// A full date and time, from the RFC 3339 timestamp an agent sent. `None`
/// when it does not parse — the caller owns what an unreadable timestamp
/// should look like, and the protocol allows ISO 8601 shapes wider than the
/// RFC 3339 subset read here.
pub fn local_datetime(iso: &str) -> Option<String> {
    rfc3339_in_zone(iso, &Local, Shape::DateAndTime)
}

/// A month, day and time, from an instant already in hand. No year: the lists
/// this labels only keep recent entries.
pub fn local_month_day_time<T: TimeZone>(when: DateTime<T>) -> String {
    in_zone(when, &Local, Shape::MonthDayAndTime)
}

/// Render an instant in `tz`. The zone is explicit so the tests can pin one
/// instead of inheriting whatever the build machine is set to.
fn in_zone<T, Z>(when: DateTime<T>, tz: &Z, format: Shape) -> String
where
    T: TimeZone,
    Z: TimeZone,
    Z::Offset: Display,
{
    when.with_timezone(tz).format(&format.pattern()).to_string()
}

/// Same, from an RFC 3339 string. `None` when it does not parse.
fn rfc3339_in_zone<Z>(iso: &str, tz: &Z, format: Shape) -> Option<String>
where
    Z: TimeZone,
    Z::Offset: Display,
{
    DateTime::parse_from_rfc3339(iso)
        .ok()
        .map(|when| in_zone(when, tz, format))
}

#[cfg(test)]
mod tests;
