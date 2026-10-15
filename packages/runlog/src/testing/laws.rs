//! The properties a [`Machine`] needs for its log to replay into the state
//! it committed.

use super::{FaultBackend, HARNESS_BUILD, json};
use crate::backend::LogBackend as _;
use crate::error::CommitError;
use crate::log::RunLog;
use crate::machine::Machine;
use crate::snapshot;

/// A log whose state is `state`, restored from a generation-1 snapshot so
/// any sample state can be committed against.
fn seeded<M: Machine>(state: &M::State) -> RunLog<M, FaultBackend> {
    let mut backend = FaultBackend::new();
    let bytes = snapshot::encode(state, 0, 0).expect("sample state serializes");
    backend
        .replace_snapshot(1, &bytes)
        .expect("the in-memory backend accepts a snapshot");
    RunLog::open(backend, HARNESS_BUILD).expect("a seeded log opens")
}

/// For every state × event: `check` does not mutate; a rejection writes and
/// changes nothing, so `assign` never sees it; an accepted event still passes
/// `check` once assigned (replay re-checks it); `apply` is deterministic;
/// `commit` returns that event and state, and a reopen replays to it.
pub fn assert_machine_laws<M: Machine>(states: &[M::State], events: &[M::Event])
where
    M::Event: Clone,
{
    for (s, state) in states.iter().enumerate() {
        for (e, event) in events.iter().enumerate() {
            let at = format!("state #{s}, event #{e}");
            let before = json(state);
            let verdict = M::check(state, event);
            assert_eq!(json(state), before, "{at}: check mutated the state");

            let mut log = seeded::<M>(state);
            let seq = log.last_seq();
            let appends = log.backend().append_lens().len();
            let Ok(()) = verdict else {
                let result = log.commit(event.clone());
                assert!(
                    matches!(result, Err(CommitError::Rejected(_))),
                    "{at}: commit did not reject what check rejects"
                );
                assert_eq!(log.last_seq(), seq, "{at}: a rejection took a seq");
                assert_eq!(log.backend().append_lens().len(), appends, "{at}: wrote");
                assert_eq!(json(log.state()), before, "{at}: a rejection changed state");
                continue;
            };

            let mut assigned = event.clone();
            M::assign(state, &mut assigned);
            assert!(
                M::check(state, &assigned).is_ok(),
                "{at}: check rejects the event assign filled in"
            );
            let mut once = state.clone();
            M::apply(&mut once, &assigned);
            let mut twice = state.clone();
            M::apply(&mut twice, &assigned);
            assert_eq!(
                json(&once),
                json(&twice),
                "{at}: apply is not deterministic"
            );

            let committed = log
                .commit(event.clone())
                .unwrap_or_else(|err| panic!("{at}: commit failed: {err}"));
            assert_eq!(committed.seq, seq + 1, "{at}: seq did not advance by one");
            assert_eq!(json(&committed.event), json(&assigned), "{at}: event");
            assert_eq!(json(&committed.state_after), json(&once), "{at}: state");

            let reopened = RunLog::<M, _>::open(log.backend().crash(), HARNESS_BUILD)
                .unwrap_or_else(|err| panic!("{at}: reopen failed: {err}"));
            assert_eq!(json(reopened.state()), json(&once), "{at}: replay differs");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Accepts only positive steps.
    struct Step;

    impl Machine for Step {
        type State = i32;
        type Event = i32;
        type Rejected = std::fmt::Error;

        fn check(_: &i32, event: &i32) -> Result<(), std::fmt::Error> {
            if *event > 0 {
                Ok(())
            } else {
                Err(std::fmt::Error)
            }
        }

        fn apply(state: &mut i32, event: &i32) {
            *state += event;
        }
    }

    #[test]
    fn a_well_behaved_machine_passes() {
        assert_machine_laws::<Step>(&[0, 5], &[1, -1, 3]);
    }

    /// Fills in a number its own `check` then refuses.
    struct SelfDefeating;

    impl Machine for SelfDefeating {
        type State = i32;
        type Event = i32;
        type Rejected = std::fmt::Error;

        fn check(_: &i32, event: &i32) -> Result<(), std::fmt::Error> {
            if *event == 0 {
                Ok(())
            } else {
                Err(std::fmt::Error)
            }
        }

        fn assign(_: &i32, event: &mut i32) {
            *event = 9;
        }

        fn apply(state: &mut i32, event: &i32) {
            *state += event;
        }
    }

    #[test]
    #[should_panic(expected = "check rejects the event assign filled in")]
    fn an_assign_that_breaks_check_is_caught() {
        assert_machine_laws::<SelfDefeating>(&[0], &[0]);
    }
}
