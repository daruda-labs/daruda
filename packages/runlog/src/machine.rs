//! The contract a domain state machine fulfils to be kept in a [`RunLog`].
//!
//! [`RunLog`]: crate::RunLog

use serde::Serialize;
use serde::de::DeserializeOwned;

/// A deterministic state machine whose transitions are events.
///
/// [`RunLog::commit`](crate::RunLog::commit) runs `check`, then `assign`,
/// makes the event durable, and only then runs `apply`. Replay runs `apply`
/// alone, so `apply` must be deterministic and must not depend on anything
/// outside its arguments.
pub trait Machine {
    type State: Default + Clone + Serialize + DeserializeOwned;
    type Event: Serialize + DeserializeOwned + Send;
    type Rejected: std::error::Error + Send + 'static;

    /// Pure precondition. Never mutates. It must also accept the event as
    /// `assign` filled it in: test builds re-check every replayed record.
    fn check(state: &Self::State, event: &Self::Event) -> Result<(), Self::Rejected>;

    /// Fill in numbers the log owns (attempt, evidence, ask id …) from current
    /// state, so concurrent proposers never pick the same one. Runs between
    /// `check` and the append.
    fn assign(state: &Self::State, event: &mut Self::Event) {
        let _ = (state, event);
    }

    /// Infallible. Called only after `check` passed and the record is durable,
    /// and on replay.
    fn apply(state: &mut Self::State, event: &Self::Event);
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Noop;

    impl Machine for Noop {
        type State = u32;
        type Event = u32;
        type Rejected = std::fmt::Error;

        fn check(_: &u32, _: &u32) -> Result<(), std::fmt::Error> {
            Ok(())
        }

        fn apply(state: &mut u32, event: &u32) {
            *state += event;
        }
    }

    #[test]
    fn the_default_assign_leaves_the_event_alone() {
        let mut event = 7;
        Noop::assign(&0, &mut event);
        assert_eq!(event, 7);
    }
}
