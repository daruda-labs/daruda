use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use std::thread;
use std::time::Duration;

use daruda_runlog::testing::{Fault, FaultBackend};
use daruda_runlog::{
    Body, CatchUp, CommitError, Committer, FileBackend, LogBackend, Record, RunLog,
};
use smol::channel::Receiver;

use crate::toy::{BUILD, Toy, ToyEvent, ToyState, add, insert, scenario, ticket};

type Hook = Box<dyn Fn(&Record<ToyEvent>, &ToyState) + Send>;

fn no_hook() -> Hook {
    Box::new(|_, _| {})
}

fn spawn<B: LogBackend + Send + 'static>(backend: B, hook: Hook) -> Committer<Toy> {
    let log = RunLog::<Toy, _>::open(backend, BUILD).unwrap();
    Committer::spawn(log, "runlog-test", hook).unwrap()
}

fn records(catch_up: CatchUp<Toy>) -> Vec<Record<ToyEvent>> {
    match catch_up {
        CatchUp::Records(records) => records,
        CatchUp::Snapshot { .. } => panic!("expected records, got a snapshot"),
    }
}

/// Everything the stream delivered; ends once the committer is gone.
fn drain(live: &Receiver<Arc<Record<ToyEvent>>>) -> Vec<Record<ToyEvent>> {
    std::iter::from_fn(|| live.recv_blocking().ok())
        .map(|record| (*record).clone())
        .collect()
}

fn reopened_records(backend: impl LogBackend, after_seq: u64) -> Vec<Record<ToyEvent>> {
    let log = RunLog::<Toy, _>::open(backend, BUILD).unwrap();
    let mut all = records(log.since(after_seq).unwrap());
    // The reopen's own `opened` record was not there before.
    all.pop();
    all
}

/// Every fault the crash harness would inject into a run with these writes.
fn faults(append_lens: &[usize], syncs: usize) -> Vec<Fault> {
    let mut faults = Vec::new();
    for (k, &len) in append_lens.iter().enumerate() {
        let mut keeps = vec![0, 1, len / 2, len.saturating_sub(1)];
        keeps.sort_unstable();
        keeps.dedup();
        faults.extend(keeps.into_iter().map(|keep| Fault::TornAppend { k, keep }));
        faults.push(Fault::FailSyncAfter { k });
    }
    faults.extend((0..syncs).map(|k| Fault::LoseUnsyncedAfter { k }));
    faults
}

#[test]
fn a_subscriber_never_sees_a_record_the_disk_lacks() {
    let clean = FaultBackend::new();
    let committer = spawn(clean.clone(), no_hook());
    for event in scenario() {
        let _ = smol::block_on(committer.propose(event));
    }
    drop(committer);

    let mut cases = 0;
    for fault in faults(&clean.append_lens(), clean.syncs()) {
        let backend = FaultBackend::new().with_fault(fault);
        let Ok(log) = RunLog::<Toy, _>::open(backend.clone(), BUILD) else {
            continue;
        };
        let hooked = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&hooked);
        let hook: Hook = Box::new(move |record, _| seen.lock().unwrap().push(record.clone()));
        let committer = Committer::spawn(log, "runlog-test", hook).unwrap();
        let (catch_up, live) = committer.subscribe(0).unwrap();
        for event in scenario() {
            if let Err(CommitError::Io(_) | CommitError::Closed) =
                smol::block_on(committer.propose(event))
            {
                break;
            }
        }
        drop(committer);
        let mut received = records(catch_up);
        received.extend(drain(&live));

        let on_disk = reopened_records(backend.crash(), 0);
        assert!(
            on_disk.starts_with(&received),
            "{fault:?}: a subscriber saw {received:?}, the disk holds {on_disk:?}"
        );
        let hooked = hooked.lock().unwrap();
        // `on_disk[0]` is the first open's `opened` record, which no hook sees.
        assert!(
            on_disk.get(1..).unwrap_or_default().starts_with(&hooked),
            "{fault:?}: the hook saw {hooked:?}, the disk holds {on_disk:?}"
        );
        cases += 1;
    }
    assert!(cases >= 9 * 3, "only {cases} cases ran");
}

#[test]
fn catch_up_then_live_has_no_gap_and_no_duplicate() {
    let temp = tempfile::tempdir().unwrap();
    let committer = spawn(FileBackend::new(temp.path()), no_hook());
    let done = Arc::new(AtomicU64::new(0));
    let proposers: Vec<_> = (0..4)
        .map(|_| {
            let committer = committer.clone();
            let done = Arc::clone(&done);
            thread::spawn(move || {
                for _ in 0..30 {
                    smol::block_on(committer.propose(ticket())).unwrap();
                    done.fetch_add(1, Ordering::SeqCst);
                }
            })
        })
        .collect();
    while done.load(Ordering::SeqCst) < 20 {
        thread::yield_now();
    }
    let (catch_up, live) = committer.subscribe(5).unwrap();
    for proposer in proposers {
        proposer.join().unwrap();
    }
    drop(committer);

    let mut stream = records(catch_up);
    stream.extend(drain(&live));
    let expected = reopened_records(FileBackend::new(temp.path()), 5);
    assert_eq!(expected.len(), 1 + 120 - 5);
    assert_eq!(stream, expected);
}

#[test]
fn a_subscriber_behind_a_compaction_starts_from_the_snapshot() {
    let committer = spawn(FaultBackend::new(), no_hook());
    for n in [1, 2, 3] {
        smol::block_on(committer.propose(add(n))).unwrap();
    }
    smol::block_on(committer.compact()).unwrap();
    let last = smol::block_on(committer.propose(add(4))).unwrap().seq;

    let (catch_up, live) = committer.subscribe(2).unwrap();
    let CatchUp::Snapshot { state, seq } = catch_up else {
        panic!("expected a snapshot, got {catch_up:?}");
    };
    assert_eq!((state.total, seq), (10, last));

    smol::block_on(committer.propose(add(5))).unwrap();
    drop(committer);
    let streamed = drain(&live);
    assert_eq!(streamed.len(), 1);
    assert_eq!(streamed[0].seq, last + 1);
    assert_eq!(streamed[0].body, Body::Event { event: add(5) });
}

#[test]
fn flush_returns_after_every_queued_proposal_is_durable() {
    let backend = FaultBackend::new();
    let committer = spawn(backend.clone(), no_hook());
    let pending: Vec<_> = (0..20).map(|_| committer.propose(ticket())).collect();
    committer.flush(Duration::from_secs(10)).unwrap();

    let durable = RunLog::<Toy, _>::open(backend.crash(), BUILD).unwrap();
    assert_eq!(durable.state().tickets, (1..=20).collect::<Vec<_>>());
    for proposal in pending {
        let answered = smol::block_on(smol::future::poll_once(proposal));
        assert!(matches!(answered, Some(Ok(_))), "{answered:?}");
    }
}

#[test]
fn flush_times_out_while_a_proposal_is_still_queued() {
    let release = Arc::new(Barrier::new(2));
    let hold = Arc::clone(&release);
    let committer = spawn(
        FaultBackend::new(),
        Box::new(move |_, _| {
            hold.wait();
        }),
    );
    let pending = committer.propose(add(1));
    let timed_out = committer.flush(Duration::from_millis(50));
    assert!(timed_out.is_err());
    assert!(timed_out.unwrap_err().to_string().contains("50ms"));
    release.wait();
    smol::block_on(pending).unwrap();
}

#[test]
fn concurrent_proposals_get_distinct_assigned_numbers() {
    let committer = spawn(FaultBackend::new(), no_hook());
    let start = Arc::new(Barrier::new(8));
    let proposers: Vec<_> = (0..8)
        .map(|_| {
            let committer = committer.clone();
            let start = Arc::clone(&start);
            thread::spawn(move || {
                start.wait();
                (0..25)
                    .map(|_| match smol::block_on(committer.propose(ticket())) {
                        Ok(committed) => match committed.event {
                            ToyEvent::Ticket { number } => number,
                            other => panic!("unexpected event {other:?}"),
                        },
                        Err(err) => panic!("proposal failed: {err}"),
                    })
                    .collect::<Vec<_>>()
            })
        })
        .collect();
    let mut numbers: Vec<u32> = proposers
        .into_iter()
        .flat_map(|proposer| proposer.join().unwrap())
        .collect();
    numbers.sort_unstable();
    assert_eq!(numbers, (1..=200).collect::<Vec<_>>());
}

/// The proposer is answered only once the hook has returned: a proposal is
/// still pending while the hook is held, and a subscriber has the record by
/// the time the proposer has its answer.
#[test]
fn the_after_commit_hook_runs_before_the_proposer_resumes() {
    // Channels rather than a barrier or an in-hook assertion: whatever fails,
    // a sender drops and both sides are let go, so the failure is not a hang.
    let (entered, in_hook) = std::sync::mpsc::channel::<i64>();
    let (sender, released) = std::sync::mpsc::channel::<()>();
    let committer = spawn(
        FaultBackend::new(),
        Box::new(move |_, state| {
            entered.send(state.total).ok();
            released.recv().ok();
        }),
    );
    // Bound after the committer so it drops first: a committer dropped while
    // its hook still waits would never finish joining its thread.
    let release = sender;
    let (_, live) = committer.subscribe(0).unwrap();
    let mut proposal = std::pin::pin!(committer.propose(add(3)));
    let total = in_hook
        .recv_timeout(Duration::from_secs(10))
        .expect("the hook never ran");
    assert_eq!(total, 3, "the hook saw the state before the commit");
    for _ in 0..5 {
        assert!(
            smol::block_on(smol::future::poll_once(proposal.as_mut())).is_none(),
            "the proposer resumed while the hook was still running"
        );
        thread::sleep(Duration::from_millis(10));
    }
    release.send(()).unwrap();
    let committed = smol::block_on(proposal).unwrap();
    // Subscribers are fed before the proposer is answered, too.
    assert_eq!(live.try_recv().unwrap().seq, committed.seq);
}

#[test]
fn after_an_io_error_every_later_proposal_is_closed() {
    // Append 0 is the `opened` record; the first proposal's sync fails.
    let backend = FaultBackend::new().with_fault(Fault::FailSyncAfter { k: 1 });
    let calls = Arc::new(AtomicU64::new(0));
    let counted = Arc::clone(&calls);
    let committer = spawn(
        backend,
        Box::new(move |_, _| {
            counted.fetch_add(1, Ordering::SeqCst);
        }),
    );
    let (_, live) = committer.subscribe(0).unwrap();
    assert!(matches!(
        smol::block_on(committer.propose(add(1))),
        Err(CommitError::Io(_))
    ));
    assert!(matches!(
        smol::block_on(committer.propose(add(1))),
        Err(CommitError::Closed)
    ));
    committer.flush(Duration::from_secs(10)).unwrap();
    assert!(smol::block_on(committer.compact()).is_err());
    // The stream ends without delivering the failed record.
    assert!(live.recv_blocking().is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn a_panicking_hook_poisons_the_committer() {
    let committer = spawn(FaultBackend::new(), Box::new(|_, _| panic!("hook bug")));
    let (_, live) = committer.subscribe(0).unwrap();
    assert!(matches!(
        smol::block_on(committer.propose(add(1))),
        Err(CommitError::Poisoned)
    ));
    assert!(matches!(
        smol::block_on(committer.propose(add(1))),
        Err(CommitError::Poisoned)
    ));
    committer.flush(Duration::from_secs(10)).unwrap();
    // The durable record still reaches the stream, which then ends.
    assert_eq!(live.recv_blocking().unwrap().seq, 2);
    assert!(live.recv_blocking().is_err());
}

/// A machine with a bug: `check` panics on one event, `apply` on another.
struct Fuse;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
enum FuseEvent {
    Fine,
    BlowInCheck,
    BlowInApply,
}

impl daruda_runlog::Machine for Fuse {
    type State = u32;
    type Event = FuseEvent;
    type Rejected = std::fmt::Error;

    fn check(_: &u32, event: &FuseEvent) -> Result<(), std::fmt::Error> {
        assert!(*event != FuseEvent::BlowInCheck, "machine bug in check");
        Ok(())
    }

    fn apply(state: &mut u32, event: &FuseEvent) {
        assert!(*event != FuseEvent::BlowInApply, "machine bug in apply");
        *state += 1;
    }
}

/// A panic inside the machine poisons the committer like a panicking hook
/// does, whether it struck before the record was written (`check`) or after
/// it was durable (`apply`). Either way no subscriber sees the record, and
/// only the `apply` case leaves it in the log for the next open to replay.
#[test]
fn a_panicking_machine_poisons_the_committer() {
    for (blow, durable) in [
        (FuseEvent::BlowInCheck, false),
        (FuseEvent::BlowInApply, true),
    ] {
        let backend = FaultBackend::new();
        let log = RunLog::<Fuse, _>::open(backend.clone(), BUILD).unwrap();
        let committer = Committer::spawn(log, "runlog-test", Box::new(|_, _| {})).unwrap();
        let (_, live) = committer.subscribe(0).unwrap();
        smol::block_on(committer.propose(FuseEvent::Fine)).unwrap();
        assert!(
            matches!(
                smol::block_on(committer.propose(blow.clone())),
                Err(CommitError::Poisoned)
            ),
            "{blow:?}"
        );
        assert!(
            matches!(
                smol::block_on(committer.propose(FuseEvent::Fine)),
                Err(CommitError::Poisoned)
            ),
            "{blow:?}: a later proposal went through"
        );
        let log = String::from_utf8(backend.file("log.0.jsonl").unwrap()).unwrap();
        assert_eq!(
            log.contains(&format!("{blow:?}")),
            durable,
            "{blow:?}: durability of the blown record"
        );
        assert_eq!(
            log.matches("Fine").count(),
            1,
            "{blow:?}: only the first proposal reached the log"
        );
        assert!(matches!(
            live.recv_blocking().unwrap().body,
            Body::Event {
                event: FuseEvent::Fine
            }
        ));
        assert!(
            live.recv_blocking().is_err(),
            "{blow:?}: the stream must end without the blown record"
        );
        // A dead committer answers a flush at once instead of timing out.
        assert!(committer.flush(Duration::from_secs(10)).is_ok(), "{blow:?}");
    }
}

/// What the hook got back from the committer it was called by.
#[derive(Debug, Default)]
struct FromInside {
    subscribe: Option<bool>,
    flush: Option<bool>,
}

/// The hook cannot wait on the committer that runs it: `subscribe` refuses
/// at once and `flush` times out, rather than either deadlocking.
#[test]
fn the_hook_cannot_wait_on_its_own_committer() {
    let slot: Arc<Mutex<Option<Committer<Toy>>>> = Arc::new(Mutex::new(None));
    let seen = Arc::new(Mutex::new(FromInside::default()));
    let (inner, out) = (Arc::clone(&slot), Arc::clone(&seen));
    let committer = spawn(
        FaultBackend::new(),
        Box::new(move |_, _| {
            let Some(own) = inner.lock().unwrap().clone() else {
                return;
            };
            let mut seen = out.lock().unwrap();
            seen.subscribe = Some(matches!(
                own.subscribe(0),
                Err(daruda_runlog::SubscribeError::OnCommitterThread)
            ));
            seen.flush = Some(own.flush(Duration::from_millis(50)).is_err());
        }),
    );
    *slot.lock().unwrap() = Some(committer.clone());

    smol::block_on(committer.propose(add(1))).unwrap();
    // The hook holding its own committer is a cycle; break it so the thread
    // can exit (see the crate's AGENTS.md).
    slot.lock().unwrap().take();

    let seen = seen.lock().unwrap();
    assert_eq!(seen.subscribe, Some(true), "subscribe did not refuse");
    assert_eq!(seen.flush, Some(true), "flush did not time out");
}

#[test]
fn dropping_the_committer_joins_its_thread() {
    struct SetOnDrop(Arc<AtomicBool>);
    impl Drop for SetOnDrop {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    let stopped = Arc::new(AtomicBool::new(false));
    let guard = SetOnDrop(Arc::clone(&stopped));
    let committer = spawn(
        FaultBackend::new(),
        Box::new(move |_, _| {
            let _alive = &guard;
        }),
    );
    let handle = committer.clone();
    let queued: Vec<_> = (1..=5).map(|n| committer.propose(add(n))).collect();
    drop(committer);
    assert!(!stopped.load(Ordering::SeqCst), "a clone still holds it");
    drop(handle);
    // The thread drained what was queued and dropped its hook on the way out.
    assert!(stopped.load(Ordering::SeqCst));
    let totals: Vec<i64> = queued
        .into_iter()
        .map(|proposal| smol::block_on(proposal).unwrap().state_after.total)
        .collect();
    assert_eq!(totals, vec![1, 3, 6, 10, 15]);
}

#[test]
fn a_rejected_proposal_emits_nothing() {
    let calls = Arc::new(AtomicU64::new(0));
    let counted = Arc::clone(&calls);
    let committer = spawn(
        FaultBackend::new(),
        Box::new(move |_, _| {
            counted.fetch_add(1, Ordering::SeqCst);
        }),
    );
    let (_, live) = committer.subscribe(1).unwrap();
    smol::block_on(committer.propose(insert("a"))).unwrap();
    assert!(matches!(
        smol::block_on(committer.propose(insert("a"))),
        Err(CommitError::Rejected(_))
    ));
    drop(committer);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let streamed = drain(&live);
    assert_eq!(streamed.len(), 1);
    assert_eq!(streamed[0].body, Body::Event { event: insert("a") });
}
