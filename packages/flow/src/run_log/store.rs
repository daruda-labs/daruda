//! The run thread's handle on a run's log. Commits go to a `Committer` on its
//! own thread, so an fsync never stalls the other nodes of a wave (F11); the
//! committer narrates each record to the host only once it is durable.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use daruda_runlog::{
    AfterCommit, Body, CatchUp, CommitError, Committed, Committer, FileBackend, LogBackend,
    OpenError, Record, RunLog, SubscribeError,
};

use super::machine::{FlowRunEvent, FlowRunMachine, FlowRunRejected, FlowRunState};
use crate::error::{FlowIoError, FlowLogError, IoSite, LogRefusal};
use crate::event::{RunNotice, Sequenced};
use crate::schedule::RunOutcome;

/// Who wrote a record, for diagnosing two builds sharing one run directory.
const BUILD: &str = concat!("daruda_flow ", env!("CARGO_PKG_VERSION"));
const RECORD: &str = "recording the run's progress";
const OPEN: &str = "opening the run's progress log";

pub(crate) type FlowCommitError = CommitError<FlowRunRejected>;

/// Open the log in `run_dir`, creating its directory. The caller holds the
/// run's lock: the log assumes one writer.
pub(crate) fn open_file_log(
    run_dir: &Path,
) -> Result<RunLog<FlowRunMachine, FileBackend>, RunOutcome> {
    let dir = super::log_dir(run_dir);
    std::fs::create_dir_all(&dir).map_err(|source| open_failed(&dir, source.into()))?;
    RunLog::open(FileBackend::new(dir.clone()), BUILD).map_err(|e| open_failed(&dir, e.into()))
}

/// The same over any backend, for the tests that crash one.
#[cfg(test)]
pub(crate) fn open_log<B: LogBackend>(
    backend: B,
    dir: &Path,
) -> Result<RunLog<FlowRunMachine, B>, RunOutcome> {
    RunLog::open(backend, BUILD).map_err(|e| open_failed(dir, e.into()))
}

fn open_failed(dir: &Path, fault: LogFault) -> RunOutcome {
    fault.outcome(IoSite::Run, OPEN, dir.to_path_buf())
}

/// What went wrong between a run and its log: the disk, or a refusal the
/// disk had no part in. Kept apart until it becomes the run's outcome, so a
/// host can tell them apart.
#[derive(Debug)]
pub(crate) enum LogFault {
    Disk(std::io::Error),
    Refused(LogRefusal),
}

impl LogFault {
    pub(crate) fn outcome(self, site: IoSite, doing: &'static str, path: PathBuf) -> RunOutcome {
        match self {
            LogFault::Disk(source) => RunOutcome::Io(FlowIoError {
                site,
                doing,
                path,
                source,
            }),
            LogFault::Refused(refusal) => RunOutcome::Log(FlowLogError {
                site,
                doing,
                path,
                refusal,
            }),
        }
    }
}

impl From<std::io::Error> for LogFault {
    fn from(error: std::io::Error) -> Self {
        LogFault::Disk(error)
    }
}

impl From<LogRefusal> for LogFault {
    fn from(refusal: LogRefusal) -> Self {
        LogFault::Refused(refusal)
    }
}

impl From<FlowCommitError> for LogFault {
    fn from(error: FlowCommitError) -> Self {
        match error {
            CommitError::Io(e) => LogFault::Disk(e),
            CommitError::Rejected(rejected) => LogRefusal::Rejected(rejected).into(),
            CommitError::Closed => LogRefusal::Closed.into(),
            CommitError::Poisoned => LogRefusal::Stopped.into(),
        }
    }
}

impl From<OpenError> for LogFault {
    fn from(error: OpenError) -> Self {
        match error {
            OpenError::Io(e) => LogFault::Disk(e),
            OpenError::Corrupt { line } => LogRefusal::Corrupt { line }.into(),
            OpenError::TooNew { found, supported } => {
                LogRefusal::TooNew { found, supported }.into()
            }
            OpenError::NoWholeGeneration { generation } => {
                LogRefusal::NoWholeGeneration { generation }.into()
            }
        }
    }
}

impl From<SubscribeError> for LogFault {
    fn from(error: SubscribeError) -> Self {
        match error {
            SubscribeError::Io(e) => LogFault::Disk(e),
            SubscribeError::Stopped => LogRefusal::Stopped.into(),
            SubscribeError::OnCommitterThread => LogRefusal::Reentered.into(),
        }
    }
}

/// Time the run spent with a question open, read off the committed state.
/// Feeds the run deadline, so a sibling's boundary check does not count a
/// wait that has not settled yet as work (F15).
///
/// Observed by the committer's hook, so a detached commit (a drop guard
/// closing its question) moves it as soon as it is durable.
#[derive(Debug, Default)]
pub(crate) struct ParkClock(Mutex<Parked>);

#[derive(Debug, Default)]
struct Parked {
    since: Option<Instant>,
    total: Duration,
}

impl ParkClock {
    fn observe(&self, open: bool, now: Instant) {
        let mut parked = self.lock();
        match (open, parked.since) {
            (true, None) => parked.since = Some(now),
            (false, Some(start)) => {
                parked.total += now.saturating_duration_since(start);
                parked.since = None;
            }
            _ => {}
        }
    }

    pub(crate) fn parked(&self, now: Instant) -> Duration {
        let parked = self.lock();
        parked.total
            + parked
                .since
                .map(|start| now.saturating_duration_since(start))
                .unwrap_or_default()
    }

    /// Two plain fields, each written whole: a panic elsewhere cannot leave
    /// them half-updated, so a poisoned lock's value is still the clock.
    fn lock(&self) -> std::sync::MutexGuard<'_, Parked> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Starts whose commits are still on their way. Single-threaded, like the
/// drive future that uses it.
#[derive(Default)]
struct StartGate {
    pending: Cell<usize>,
    waiters: RefCell<Vec<std::task::Waker>>,
}

/// One start in flight; leaving on drop, so a start whose future is dropped
/// never holds the others back.
struct PendingStart<'g>(&'g StartGate);

impl StartGate {
    fn enter(&self) -> PendingStart<'_> {
        self.pending.set(self.pending.get() + 1);
        PendingStart(self)
    }

    async fn opened(&self) {
        std::future::poll_fn(|cx| {
            if self.pending.get() == 0 {
                return std::task::Poll::Ready(());
            }
            self.waiters.borrow_mut().push(cx.waker().clone());
            std::task::Poll::Pending
        })
        .await;
    }
}

impl Drop for PendingStart<'_> {
    fn drop(&mut self) {
        let gate = self.0;
        gate.pending.set(gate.pending.get() - 1);
        if gate.pending.get() == 0 {
            for waker in gate.waiters.take() {
                waker.wake();
            }
        }
    }
}

/// Cheap to clone; every clone is the same run's log. `!Send` on purpose: it
/// lives on the run thread, beside the drive future.
#[derive(Clone)]
pub(crate) struct FlowRunStore {
    inner: Rc<Inner>,
}

struct Inner {
    committer: Committer<FlowRunMachine>,
    starts: StartGate,
    /// The newest committed state this thread has seen, by seq.
    latest: RefCell<(u64, FlowRunState)>,
    clock: Arc<ParkClock>,
    dir: PathBuf,
}

impl FlowRunStore {
    /// Hand `log` to its committer thread. Each record past this point is
    /// narrated on `events` once durable; `RunEnded` is left to the caller,
    /// which says it only after the marker and the lock release.
    pub(crate) fn spawn<B: LogBackend + Send + 'static>(
        log: RunLog<FlowRunMachine, B>,
        run_dir: &Path,
        events: Option<smol::channel::Sender<RunNotice>>,
    ) -> Result<Self, RunOutcome> {
        let dir = super::log_dir(run_dir);
        let latest = (log.last_seq(), log.state().clone());
        let clock = Arc::new(ParkClock::default());
        clock.observe(latest.1.has_open_asks(), Instant::now());
        let narrated_dir = run_dir.to_path_buf();
        let observed = clock.clone();
        let hook: AfterCommit<FlowRunMachine> = Box::new(move |record, state| {
            observed.observe(state.has_open_asks(), Instant::now());
            narrate_live(&narrated_dir, events.as_ref(), record, state);
        });
        let committer = Committer::spawn(log, "flow-run-log", hook)
            .map_err(|source| open_failed(&dir, source.into()))?;
        Ok(Self {
            inner: Rc::new(Inner {
                committer,
                starts: StartGate::default(),
                latest: RefCell::new(latest),
                clock,
                dir,
            }),
        })
    }

    /// Commit `event` and wait until it is durable; the result carries the
    /// numbers `assign` gave it.
    pub(crate) async fn commit(
        &self,
        event: FlowRunEvent,
    ) -> Result<Committed<FlowRunMachine>, FlowCommitError> {
        let committed = self.inner.committer.propose(event).await?;
        self.observe(committed.seq, &committed.state_after);
        Ok(committed)
    }

    /// Commit an attempt's start, then wait for every start proposed beside
    /// it: a wave's nodes begin their sessions together, however long each
    /// commit's fsync took.
    pub(crate) async fn commit_start(
        &self,
        event: FlowRunEvent,
    ) -> Result<Committed<FlowRunMachine>, FlowCommitError> {
        let pending = self.inner.starts.enter();
        let committed = self.commit(event).await;
        drop(pending);
        // Once round the executor, so every start resumes in the order the
        // wave proposed them, the last to commit included.
        smol::future::yield_now().await;
        self.inner.starts.opened().await;
        committed
    }

    /// Queue `event` without waiting: for a drop guard, which cannot await.
    /// The commit still happens, in order with everything proposed after it.
    pub(crate) fn commit_detached(&self, event: FlowRunEvent) {
        // SILENT-OK: a failed commit closes the log, and the next awaited
        // commit reports that.
        drop(self.inner.committer.propose(event));
    }

    fn observe(&self, seq: u64, state: &FlowRunState) {
        let mut latest = self.inner.latest.borrow_mut();
        if seq > latest.0 {
            *latest = (seq, state.clone());
        }
    }

    /// The newest state this thread has committed or seen.
    pub(crate) fn state(&self) -> std::cell::Ref<'_, FlowRunState> {
        std::cell::Ref::map(self.inner.latest.borrow(), |(_, state)| state)
    }

    pub(crate) fn clock(&self) -> Arc<ParkClock> {
        self.inner.clock.clone()
    }

    /// Every event committed so far, read on the committer thread.
    pub(crate) fn events(&self) -> Result<Vec<FlowRunEvent>, LogFault> {
        let (catch_up, _live) = self.inner.committer.subscribe(0)?;
        Ok(match catch_up {
            CatchUp::Records(records) => records
                .into_iter()
                .filter_map(|record| match record.body {
                    Body::Event { event } => Some(event),
                    Body::Opened { .. } => None,
                })
                .collect(),
            CatchUp::Snapshot { .. } => Vec::new(),
        })
    }

    /// A commit that failed, as the outcome the run ends with: past this
    /// point nothing the run does can be recorded, so it cannot go on.
    pub(crate) fn failure(&self, site: IoSite, error: impl Into<LogFault>) -> RunOutcome {
        error.into().outcome(site, RECORD, self.inner.dir.clone())
    }
}

fn narrate_live(
    run_dir: &Path,
    events: Option<&smol::channel::Sender<RunNotice>>,
    record: &Record<FlowRunEvent>,
    state: &FlowRunState,
) {
    let Body::Event { event } = &record.body else {
        return;
    };
    if matches!(event, FlowRunEvent::RunEnded { .. }) {
        return;
    }
    if let Some(narrated) = super::narrate::narrate(run_dir, event, state) {
        crate::event::emit(
            events,
            RunNotice::Recorded(Sequenced {
                seq: record.seq,
                event: narrated,
            }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Closed waits add up, and a wait still open counts up to now.
    #[test]
    fn the_park_clock_counts_open_time_only() {
        let clock = ParkClock::default();
        let start = Instant::now();
        clock.observe(true, start);
        clock.observe(true, start + Duration::from_secs(1));
        clock.observe(false, start + Duration::from_secs(2));
        clock.observe(false, start + Duration::from_secs(5));
        assert_eq!(
            clock.parked(start + Duration::from_secs(9)),
            Duration::from_secs(2)
        );
        clock.observe(true, start + Duration::from_secs(10));
        assert_eq!(
            clock.parked(start + Duration::from_secs(13)),
            Duration::from_secs(5)
        );
    }

    /// A question closed by a drop guard's detached commit stops the clock
    /// when it commits, not at whatever awaited commit comes next.
    #[test]
    fn the_park_clock_sees_a_detached_commit() {
        use super::super::machine::AbandonReason;
        let dir = tempfile::tempdir().expect("tempdir");
        let node = crate::NodeId::from("design");
        let store = super::super::testing::attempt_in_flight(dir.path(), &node);
        let owner = crate::runner::AskOwner::Node {
            node,
            generation: 0,
            attempt: 1,
        };
        let ask = crate::run_log::line::AskLine {
            tool: "write".to_string(),
            detail: None,
        };
        let raised = smol::block_on(store.commit(FlowRunEvent::ask_raised(owner, ask)))
            .expect("the question commits");
        let FlowRunEvent::AskRaised { id, .. } = raised.event else {
            panic!("{:?}", raised.event);
        };
        store.commit_detached(FlowRunEvent::AskAbandoned {
            id,
            reason: AbandonReason::Interrupted,
        });
        // Answered in arrival order, so the detached commit is behind it.
        let events = store.events().expect("the log reads");
        assert!(matches!(
            events.last(),
            Some(FlowRunEvent::AskAbandoned { .. })
        ));

        let clock = store.clock();
        let now = Instant::now();
        assert_eq!(
            clock.parked(now),
            clock.parked(now + Duration::from_secs(60)),
            "the closed question still counts as open"
        );
    }

    /// A record the machine refuses is an engine fault, not a disk failure,
    /// and the outcome it ends the run with says which.
    #[test]
    fn a_refused_commit_is_not_a_disk_failure() {
        let dir = tempfile::tempdir().expect("tempdir");
        let node = crate::NodeId::from("design");
        let store = super::super::testing::attempt_in_flight(dir.path(), &node);
        let again = smol::block_on(store.commit(FlowRunEvent::attempt_started(node, 0)))
            .expect_err("a second start beside the first is refused");
        let outcome = store.failure(IoSite::Run, again);
        assert!(
            matches!(
                &outcome,
                RunOutcome::Log(FlowLogError {
                    refusal: LogRefusal::Rejected(FlowRunRejected::InFlight(_)),
                    ..
                })
            ),
            "{outcome:?}"
        );
    }

    /// Every start in flight holds the gate; the last one out opens it.
    #[test]
    fn the_start_gate_opens_when_the_last_start_leaves() {
        let gate = StartGate::default();
        let first = gate.enter();
        let second = gate.enter();
        drop(first);
        assert_eq!(gate.pending.get(), 1);
        drop(second);
        smol::block_on(gate.opened());
        assert_eq!(gate.pending.get(), 0);
    }
}
