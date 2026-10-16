//! The run thread's handle on a run's log. Commits go to a `Committer` on its
//! own thread, so an fsync never stalls the other nodes of a wave (F11); the
//! committer narrates each record to the host only once it is durable.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use daruda_runlog::{
    AfterCommit, Body, CatchUp, CommitError, Committed, Committer, FileBackend, LogBackend,
    OpenError, Record, RunLog,
};

use super::machine::{FlowRunEvent, FlowRunMachine, FlowRunRejected, FlowRunState};
use crate::error::{FlowIoError, IoSite};
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
    std::fs::create_dir_all(&dir).map_err(|source| open_failed(&dir, source))?;
    RunLog::open(FileBackend::new(dir.clone()), BUILD).map_err(|e| open_failed(&dir, open_io(e)))
}

/// The same over any backend, for the tests that crash one.
#[cfg(test)]
pub(crate) fn open_log<B: LogBackend>(
    backend: B,
    dir: &Path,
) -> Result<RunLog<FlowRunMachine, B>, RunOutcome> {
    RunLog::open(backend, BUILD).map_err(|e| open_failed(dir, open_io(e)))
}

fn open_io(error: OpenError) -> std::io::Error {
    match error {
        OpenError::Io(e) => e,
        other => std::io::Error::other(other.to_string()),
    }
}

fn open_failed(dir: &Path, source: std::io::Error) -> RunOutcome {
    RunOutcome::Io(FlowIoError {
        site: IoSite::Run,
        doing: OPEN,
        path: dir.to_path_buf(),
        source,
    })
}

/// Time the run spent with a question open, read off the committed state.
/// Feeds the run deadline, so a sibling's boundary check does not count a
/// wait that has not settled yet as work (F15).
#[derive(Debug, Default)]
pub(crate) struct ParkClock {
    since: Cell<Option<Instant>>,
    total: Cell<Duration>,
}

impl ParkClock {
    fn observe(&self, open: bool, now: Instant) {
        match (open, self.since.get()) {
            (true, None) => self.since.set(Some(now)),
            (false, Some(start)) => {
                self.total
                    .set(self.total.get() + now.saturating_duration_since(start));
                self.since.set(None);
            }
            _ => {}
        }
    }

    pub(crate) fn parked(&self, now: Instant) -> Duration {
        self.total.get()
            + self
                .since
                .get()
                .map(|start| now.saturating_duration_since(start))
                .unwrap_or_default()
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
    clock: Rc<ParkClock>,
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
        let narrated_dir = run_dir.to_path_buf();
        let hook: AfterCommit<FlowRunMachine> = Box::new(move |record, state| {
            narrate_live(&narrated_dir, events.as_ref(), record, state);
        });
        let committer = Committer::spawn(log, "flow-run-log", hook)
            .map_err(|source| open_failed(&dir, source))?;
        let clock = Rc::new(ParkClock::default());
        clock.observe(latest.1.has_open_asks(), Instant::now());
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
            self.inner
                .clock
                .observe(state.has_open_asks(), Instant::now());
        }
    }

    /// The newest state this thread has committed or seen.
    pub(crate) fn state(&self) -> std::cell::Ref<'_, FlowRunState> {
        std::cell::Ref::map(self.inner.latest.borrow(), |(_, state)| state)
    }

    pub(crate) fn clock(&self) -> Rc<ParkClock> {
        self.inner.clock.clone()
    }

    /// Every event committed so far, read on the committer thread.
    pub(crate) fn events(&self) -> Result<Vec<FlowRunEvent>, FlowCommitError> {
        let (catch_up, _live) = self
            .inner
            .committer
            .subscribe(0)
            .map_err(|e| CommitError::Io(std::io::Error::other(e.to_string())))?;
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
    pub(crate) fn failure(&self, site: IoSite, error: FlowCommitError) -> RunOutcome {
        let source = match error {
            CommitError::Io(e) => e,
            other => std::io::Error::other(other.to_string()),
        };
        RunOutcome::Io(FlowIoError {
            site,
            doing: RECORD,
            path: self.inner.dir.clone(),
            source,
        })
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
