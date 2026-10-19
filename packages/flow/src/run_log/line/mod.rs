//! The shapes a run log writes. Their own types rather than `serde` on the
//! scheduler's, so an internal rename is never a format change.

use std::time::Duration;

mod attempt;
mod end;
mod failure;

pub use attempt::{AnswerLine, AskLine, AttemptLine, SpentLine};
pub use end::EndLine;
pub use failure::FailureLine;

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn epoch_millis(at: std::time::SystemTime) -> u64 {
    at.duration_since(std::time::UNIX_EPOCH)
        .map(millis)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_before_the_epoch_or_past_u64_saturate() {
        assert_eq!(millis(Duration::MAX), u64::MAX);
        assert_eq!(
            epoch_millis(std::time::UNIX_EPOCH - Duration::from_secs(1)),
            0
        );
    }
}
