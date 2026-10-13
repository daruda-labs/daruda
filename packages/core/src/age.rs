//! Elapsed-time bucketing for "N ago" labels.
//!
//! Each caller keeps its own strings and its own "never" state; this module
//! only decides which unit an age in seconds is shown in. Pure.

const SECS_PER_MINUTE: u64 = 60;
const SECS_PER_HOUR: u64 = 3_600;
const SECS_PER_DAY: u64 = 86_400;

/// The display unit for an elapsed time, counted in whole units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgeBucket {
    /// Under a minute.
    JustNow,
    Minutes(u64),
    Hours(u64),
    Days(u64),
}

/// Buckets `secs` into the largest whole unit it reaches, rounding down.
pub fn bucket(secs: u64) -> AgeBucket {
    if secs < SECS_PER_MINUTE {
        AgeBucket::JustNow
    } else if secs < SECS_PER_HOUR {
        AgeBucket::Minutes(secs / SECS_PER_MINUTE)
    } else if secs < SECS_PER_DAY {
        AgeBucket::Hours(secs / SECS_PER_HOUR)
    } else {
        AgeBucket::Days(secs / SECS_PER_DAY)
    }
}
