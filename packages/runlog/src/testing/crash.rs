//! Crash every write of a scenario, in every way [`Fault`] models, and
//! check that the reopened log holds a committed prefix and keeps going.

use super::{Fault, FaultBackend, HARNESS_BUILD, json};
use crate::error::{CommitError, OpenError};
use crate::log::RunLog;
use crate::machine::Machine;

/// One recovered crash, for checks a caller adds on top.
pub struct CrashCase<'a, M: Machine> {
    pub fault: Fault,
    /// Commits that returned `Ok` before the crash.
    pub acknowledged: usize,
    /// Accepted events the reopened log holds: `acknowledged`, or one more
    /// when the crash interrupted a commit whose line still reached disk.
    pub recovered: usize,
    pub reopened: &'a RunLog<M, FaultBackend>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CrashSummary {
    /// Crash cases run.
    pub cases: usize,
}

/// [`enumerate_crashes_with`] with no extra checks.
pub fn enumerate_crashes<M: Machine>(scenario: Vec<M::Event>) -> CrashSummary
where
    M::Event: Clone,
{
    enumerate_crashes_with::<M>(scenario, |_| {})
}

/// The scenario as committed without faults.
struct CleanRun {
    /// `prefix[m]` is the state after the first `m` accepted events.
    prefix: Vec<String>,
    append_lens: Vec<usize>,
    syncs: usize,
}

fn clean_run<M: Machine>(scenario: &[M::Event]) -> CleanRun
where
    M::Event: Clone,
{
    let mut log =
        RunLog::<M, _>::open(FaultBackend::new(), HARNESS_BUILD).expect("a fault-free log opens");
    let mut prefix = vec![json(log.state())];
    for (index, event) in scenario.iter().enumerate() {
        match log.commit(event.clone()) {
            Ok(committed) => prefix.push(json(&committed.state_after)),
            Err(CommitError::Rejected(_)) => {}
            Err(err) => panic!("event #{index} failed without a fault: {err}"),
        }
    }
    CleanRun {
        prefix,
        append_lens: log.backend().append_lens(),
        syncs: log.backend().syncs(),
    }
}

fn faults(clean: &CleanRun) -> Vec<Fault> {
    let mut faults = Vec::new();
    for (k, &len) in clean.append_lens.iter().enumerate() {
        let mut keeps = vec![0, 1, len / 2, len.saturating_sub(1)];
        keeps.sort_unstable();
        keeps.dedup();
        faults.extend(keeps.into_iter().map(|keep| Fault::TornAppend { k, keep }));
        faults.push(Fault::FailSyncAfter { k });
    }
    faults.extend((0..clean.syncs).map(|k| Fault::LoseUnsyncedAfter { k }));
    faults
}

/// Where a faulted run stopped.
struct Stopped {
    backend: FaultBackend,
    acknowledged: usize,
    /// The scenario index the next commit would take.
    resume_at: usize,
    /// Whether a commit was cut short after its line went to the backend.
    in_flight: bool,
}

fn run_until_crash<M: Machine>(scenario: &[M::Event], fault: Fault, clean: &CleanRun) -> Stopped
where
    M::Event: Clone,
{
    let backend = FaultBackend::new().with_fault(fault);
    let mut stopped = Stopped {
        backend: backend.clone(),
        acknowledged: 0,
        resume_at: 0,
        in_flight: false,
    };
    let mut log = match RunLog::<M, _>::open(backend, HARNESS_BUILD) {
        Ok(log) => log,
        Err(OpenError::Io(_)) => return stopped,
        Err(err) => panic!("{fault:?}: the first open failed: {err}"),
    };
    stopped.resume_at = scenario.len();
    for (index, event) in scenario.iter().enumerate() {
        match log.commit(event.clone()) {
            Ok(_) => stopped.acknowledged += 1,
            Err(CommitError::Rejected(_)) => {}
            Err(CommitError::Io(_)) => {
                stopped.in_flight = true;
                stopped.resume_at = index;
                break;
            }
            Err(CommitError::Closed | CommitError::Poisoned) => {
                panic!("{fault:?}: closed before an Io error")
            }
        }
    }
    assert_eq!(
        json(log.state()),
        clean.prefix[stopped.acknowledged],
        "{fault:?}: in-memory state ran ahead of what was acknowledged"
    );
    stopped
}

/// Crash every append in every [`Fault`] kind, reopen, and assert: (a) open
/// succeeds; (b) the state is a prefix holding every acknowledged commit and
/// nothing past the interrupted one; (c) the rest of the scenario commits
/// gap-free to the fault-free final state.
///
/// `extra` sees each case before (c), for host checks such as subscribers.
pub fn enumerate_crashes_with<M: Machine>(
    scenario: Vec<M::Event>,
    mut extra: impl FnMut(&CrashCase<'_, M>),
) -> CrashSummary
where
    M::Event: Clone,
{
    let clean = clean_run::<M>(&scenario);
    let all = faults(&clean);
    for &fault in &all {
        let stopped = run_until_crash::<M>(&scenario, fault, &clean);

        let mut reopened = RunLog::<M, _>::open(stopped.backend.crash(), HARNESS_BUILD)
            .unwrap_or_else(|err| panic!("{fault:?}: reopen failed: {err}"));
        // On disk before this reopen: the first open's `opened` record, if
        // it landed, then one record per recovered event.
        let on_disk = usize::try_from(reopened.last_seq() - 1).expect("seq fits usize");
        let recovered = on_disk.saturating_sub(1);
        let most = stopped.acknowledged + usize::from(stopped.in_flight);
        assert!(
            (stopped.acknowledged..=most).contains(&recovered),
            "{fault:?}: recovered {recovered} events, acknowledged {}",
            stopped.acknowledged
        );
        assert_eq!(
            json(reopened.state()),
            clean.prefix[recovered],
            "{fault:?}: the recovered state is not a committed prefix"
        );

        extra(&CrashCase {
            fault,
            acknowledged: stopped.acknowledged,
            recovered,
            reopened: &reopened,
        });

        let resume = stopped.resume_at + (recovered - stopped.acknowledged);
        for event in scenario.iter().skip(resume) {
            let before = reopened.last_seq();
            match reopened.commit(event.clone()) {
                Ok(committed) => assert_eq!(committed.seq, before + 1, "{fault:?}: seq gap"),
                Err(CommitError::Rejected(_)) => {
                    assert_eq!(
                        reopened.last_seq(),
                        before,
                        "{fault:?}: rejection took a seq"
                    );
                }
                Err(err) => panic!("{fault:?}: commit after recovery failed: {err}"),
            }
        }
        let expected = clean.prefix.last().expect("prefix starts with the default");
        assert_eq!(&json(reopened.state()), expected, "{fault:?}: final state");
        let again = RunLog::<M, _>::open(reopened.backend().crash(), HARNESS_BUILD)
            .unwrap_or_else(|err| panic!("{fault:?}: second reopen failed: {err}"));
        assert_eq!(&json(again.state()), expected, "{fault:?}: second replay");
    }
    CrashSummary { cases: all.len() }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Sum;

    impl Machine for Sum {
        type State = u64;
        type Event = u64;
        type Rejected = std::fmt::Error;

        fn check(_: &u64, _: &u64) -> Result<(), std::fmt::Error> {
            Ok(())
        }

        fn apply(state: &mut u64, event: &u64) {
            *state += event;
        }
    }

    #[test]
    fn every_append_is_crossed_with_every_fault_kind() {
        let clean = clean_run::<Sum>(&[1, 2]);
        assert_eq!(clean.prefix, vec!["0", "1", "3"]);
        let faults = faults(&clean);
        for k in 0..3 {
            assert!(faults.contains(&Fault::FailSyncAfter { k }));
            assert!(faults.contains(&Fault::LoseUnsyncedAfter { k }));
            assert!(faults.contains(&Fault::TornAppend { k, keep: 0 }));
        }
        assert_eq!(enumerate_crashes::<Sum>(vec![1, 2]).cases, faults.len());
    }
}
