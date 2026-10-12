//! The chat's wall-clock text, and the stamp it writes down to read back.
//!
//! Both directions live here because they have to agree on a format: whatever
//! [`now_rfc3339`] writes is what [`local_datetime`] has to parse. Every shape
//! is a `chrono` pattern in the locale files, so a locale picks its own field
//! order and clock convention (en-US runs on a 12-hour clock, ko on 24-hour).

use chrono::{DateTime, Local, TimeZone, Utc};
use std::fmt::Display;

use crate::strings as s;

/// Which wall-clock shape a timestamp is rendered in — one variant per locale
/// pattern, named by the functions below rather than passed by callers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    /// Full date and time, for a timestamp that has to stand alone.
    DateAndTime,
    /// Time alone, for a stamp on a row in a conversation the user is
    /// watching happen.
    TimeOnly,
}

impl Shape {
    fn pattern(self) -> String {
        match self {
            Self::DateAndTime => s::agent_chat::stamp_date_and_time(),
            Self::TimeOnly => s::agent_chat::stamp_time_only(),
        }
    }
}

/// Stamp "now" for a field that is stored and parsed back, not displayed.
/// UTC, matching what agents send over ACP.
pub(crate) fn now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}

/// A full date and time, from the RFC 3339 timestamp an agent sent. `None`
/// when it does not parse — the caller owns what an unreadable timestamp
/// should look like, and the protocol allows ISO 8601 shapes wider than the
/// RFC 3339 subset read here.
pub(crate) fn local_datetime(iso: &str) -> Option<String> {
    rfc3339_in_zone(iso, &Local, Shape::DateAndTime)
}

/// Time of day in the local zone, for a stamp whose date the surrounding UI
/// already implies. The clock convention travels with the locale, so a call
/// site must not assemble one by hand.
pub(crate) fn local_time<T: TimeZone>(when: DateTime<T>) -> String {
    in_zone(when, &Local, Shape::TimeOnly)
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
