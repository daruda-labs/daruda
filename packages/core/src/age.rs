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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_boundaries_round_down_to_whole_units() {
        assert_eq!(bucket(0), AgeBucket::JustNow);
        assert_eq!(bucket(59), AgeBucket::JustNow);
        assert_eq!(bucket(60), AgeBucket::Minutes(1));
        assert_eq!(bucket(119), AgeBucket::Minutes(1));
        assert_eq!(bucket(3_599), AgeBucket::Minutes(59));
        assert_eq!(bucket(3_600), AgeBucket::Hours(1));
        assert_eq!(bucket(23 * 3_600), AgeBucket::Hours(23));
        assert_eq!(bucket(86_399), AgeBucket::Hours(23));
        assert_eq!(bucket(86_400), AgeBucket::Days(1));
        assert_eq!(bucket(3 * 86_400), AgeBucket::Days(3));
        assert_eq!(bucket(u64::MAX), AgeBucket::Days(u64::MAX / 86_400));
    }
}
