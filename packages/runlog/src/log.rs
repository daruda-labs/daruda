//! [`RunLog`]: a machine's state kept as an append-only log of its events.

use std::{fmt, io};

use serde::Serialize;

use crate::backend::LogBackend;
use crate::error::{CommitError, OpenError};
use crate::machine::Machine;
use crate::record::{Body, FORMAT_VERSION, Record, now_ms};
use crate::replay::replay;
use crate::snapshot::{self, Base, Decoded};

/// Past either budget the current generation's log is worth compacting.
const COMPACT_AFTER_RECORDS: u64 = 1024;
const COMPACT_AFTER_BYTES: u64 = 1024 * 1024;

/// A durable, applied event.
pub struct Committed<M: Machine> {
    pub seq: u64,
    /// The event as `assign` filled it in.
    pub event: M::Event,
    /// Lets a host update its projection before answering the proposer.
    pub state_after: M::State,
}

/// A committed event with the envelope fields its record was written with.
pub(crate) struct Applied<E> {
    pub seq: u64,
    pub at_ms: u64,
    pub event: E,
}

/// What a reader that last saw some seq needs to catch up.
pub enum CatchUp<M: Machine> {
    Records(Vec<Record<M::Event>>),
    /// The records it missed were compacted away; this replaces its state.
    Snapshot {
        state: M::State,
        seq: u64,
    },
}

impl<M: Machine> fmt::Debug for Committed<M>
where
    M::Event: fmt::Debug,
    M::State: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Committed")
            .field("seq", &self.seq)
            .field("event", &self.event)
            .field("state_after", &self.state_after)
            .finish()
    }
}

impl<M: Machine> fmt::Debug for CatchUp<M>
where
    M::Event: fmt::Debug,
    M::State: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Records(records) => f.debug_tuple("Records").field(records).finish(),
            Self::Snapshot { state, seq } => f
                .debug_struct("Snapshot")
                .field("state", state)
                .field("seq", seq)
                .finish(),
        }
    }
}

/// The single writer of one machine's log. The caller must hold the external
/// lock that guarantees no other process opens the same backend.
pub struct RunLog<M: Machine, B: LogBackend> {
    backend: B,
    state: M::State,
    generation: u64,
    /// The seq the current generation's snapshot includes; 0 for generation 0.
    base_seq: u64,
    last_seq: u64,
    epoch: u64,
    log_records: u64,
    log_bytes: u64,
    /// Set by a failed write: what is on disk is no longer known.
    closed: bool,
}

fn closed_error() -> io::Error {
    io::Error::other("run log is closed after an earlier write failure")
}

/// The newest generation whose snapshot is whole, with that snapshot.
/// Every snapshot's version is read first: a newer one refuses the open.
fn choose_generation<S: Default + serde::de::DeserializeOwned>(
    backend: &impl LogBackend,
    generations: &[u64],
) -> Result<(u64, Base<S>), OpenError> {
    let mut whole = Vec::new();
    for &generation in generations {
        if generation == 0 {
            continue;
        }
        let Some(bytes) = backend.read(generation)?.snapshot else {
            continue;
        };
        match snapshot::decode(&bytes) {
            Decoded::Whole(base) => whole.push((generation, base)),
            Decoded::TooNew(found) => {
                return Err(OpenError::TooNew {
                    found,
                    supported: FORMAT_VERSION,
                });
            }
            Decoded::Broken => {}
        }
    }
    if let Some(chosen) = whole.pop() {
        return Ok(chosen);
    }
    match (generations.first(), generations.last()) {
        (None, _) | (Some(0), _) => Ok((
            0,
            Base {
                state: S::default(),
                seq: 0,
                epoch: 0,
            },
        )),
        // Generation 0 is gone, so falling back to it would silently
        // discard everything the broken snapshot held.
        (Some(_), Some(&newest)) | (Some(&newest), None) => {
            Err(OpenError::NoWholeGeneration { generation: newest })
        }
    }
}

impl<M: Machine, B: LogBackend> RunLog<M, B> {
    /// Replay the last committed state without opening a writer, repairing
    /// a torn tail, removing generations, or recording a new epoch.
    pub fn read_only(backend: &B) -> Result<(u64, M::State), OpenError> {
        let generations = backend.generations()?;
        let (generation, base) = choose_generation::<M::State>(backend, &generations)?;
        let log = backend.read(generation)?.log;
        let replayed = replay::<M>(&log, base)?;
        Ok((replayed.last_seq, replayed.state))
    }

    /// Restore from `backend` and append an `opened` record. Nothing is
    /// written until the whole chosen generation has been read and accepted.
    pub fn open(mut backend: B, build: &str) -> Result<Self, OpenError> {
        let generations = backend.generations()?;
        let (generation, base) = choose_generation::<M::State>(&backend, &generations)?;
        let base_seq = base.seq;
        let log = backend.read(generation)?.log;
        let replayed = replay::<M>(&log, base)?;

        backend.select(generation)?;
        if replayed.torn {
            backend.truncate_tail(replayed.whole_len)?;
        }
        for &other in generations.iter().filter(|&&other| other != generation) {
            backend.drop_generation(other)?;
        }

        let mut opened = Self {
            backend,
            state: replayed.state,
            generation,
            base_seq,
            last_seq: replayed.last_seq,
            epoch: replayed.epoch + 1,
            log_records: replayed.records,
            log_bytes: replayed.whole_len,
            closed: false,
        };
        let line = opened.encode(Body::<&M::Event>::Opened {
            epoch: opened.epoch,
            build: build.to_owned(),
        })?;
        opened.write(&line)?;
        Ok(opened)
    }

    /// Check, number, persist, then apply `event`. In-memory state changes
    /// only after the record is durable; a failed write closes the log.
    pub fn commit(&mut self, event: M::Event) -> Result<Committed<M>, CommitError<M::Rejected>> {
        let applied = self.commit_applied(event)?;
        Ok(Committed {
            seq: applied.seq,
            event: applied.event,
            state_after: self.state.clone(),
        })
    }

    /// [`Self::commit`] without cloning the state, keeping the record's time.
    pub(crate) fn commit_applied(
        &mut self,
        mut event: M::Event,
    ) -> Result<Applied<M::Event>, CommitError<M::Rejected>> {
        if self.closed {
            return Err(CommitError::Closed);
        }
        M::check(&self.state, &event).map_err(CommitError::Rejected)?;
        M::assign(&self.state, &mut event);
        let at_ms = now_ms();
        let line = self.encode_at(at_ms, Body::Event { event: &event })?;
        let seq = self.write(&line)?;
        M::apply(&mut self.state, &event);
        Ok(Applied { seq, at_ms, event })
    }

    /// Records after `seq`, or the current state if some were compacted away.
    pub fn since(&self, seq: u64) -> io::Result<CatchUp<M>> {
        if seq < self.base_seq {
            return Ok(CatchUp::Snapshot {
                state: self.state.clone(),
                seq: self.last_seq,
            });
        }
        let log = self.backend.read(self.generation)?.log;
        let mut records = Vec::new();
        for line in log.split(|&byte| byte == b'\n') {
            if line.is_empty() {
                continue;
            }
            let record: Record<M::Event> = serde_json::from_slice(line)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
            if record.seq > seq {
                records.push(record);
            }
        }
        Ok(CatchUp::Records(records))
    }

    /// Start a new generation from the current state and drop the old one.
    /// Once the new snapshot may be on disk, a failure closes the log: a
    /// reopen could pick that snapshot, so appending to the old log would
    /// write records no one reads.
    pub fn compact(&mut self) -> io::Result<()> {
        if self.closed {
            return Err(closed_error());
        }
        let next = self.generation + 1;
        let bytes = snapshot::encode(&self.state, self.last_seq, self.epoch)?;
        let switched = (|| {
            // A leftover from an earlier lost switch must not prefix the new log.
            if self.backend.generations()?.contains(&next) {
                self.backend.drop_generation(next)?;
            }
            self.backend.replace_snapshot(next, &bytes)?;
            self.backend.select(next)
        })();
        if let Err(err) = switched {
            self.closed = true;
            return Err(err);
        }
        let previous = std::mem::replace(&mut self.generation, next);
        self.base_seq = self.last_seq;
        self.log_records = 0;
        self.log_bytes = 0;
        // The log stays usable if this fails: open and the next compaction
        // remove leftover generations.
        self.backend.drop_generation(previous)
    }

    /// Whether a failed write closed the log.
    pub(crate) fn is_closed(&self) -> bool {
        self.closed
    }

    pub fn should_compact(&self) -> bool {
        self.log_records > COMPACT_AFTER_RECORDS || self.log_bytes > COMPACT_AFTER_BYTES
    }

    pub fn state(&self) -> &M::State {
        &self.state
    }

    pub fn last_seq(&self) -> u64 {
        self.last_seq
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }

    /// One newline-terminated record carrying the next seq.
    fn encode<E: Serialize>(&self, body: Body<E>) -> io::Result<Vec<u8>> {
        self.encode_at(now_ms(), body)
    }

    fn encode_at<E: Serialize>(&self, at_ms: u64, body: Body<E>) -> io::Result<Vec<u8>> {
        let record = Record {
            v: FORMAT_VERSION,
            seq: self.last_seq + 1,
            at_ms,
            body,
        };
        let mut line = serde_json::to_vec(&record)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        line.push(b'\n');
        Ok(line)
    }

    /// Append and sync `line`, then advance the seq it carries.
    fn write(&mut self, line: &[u8]) -> io::Result<u64> {
        if let Err(err) = self.backend.append(line).and_then(|()| self.backend.sync()) {
            self.closed = true;
            return Err(err);
        }
        self.last_seq += 1;
        self.log_records += 1;
        self.log_bytes += line.len() as u64;
        Ok(self.last_seq)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::FileBackend;

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
    fn a_fresh_directory_opens_empty_at_generation_zero() {
        let temp = tempfile::tempdir().unwrap();
        let mut log = RunLog::<Sum, _>::open(FileBackend::new(temp.path()), "b").unwrap();
        assert_eq!((*log.state(), log.last_seq()), (0, 1));
        assert_eq!(log.commit(4).unwrap().seq, 2);
        assert_eq!(log.backend().generations().unwrap(), vec![0]);
    }

    #[test]
    fn compaction_moves_to_the_next_generation() {
        let temp = tempfile::tempdir().unwrap();
        let mut log = RunLog::<Sum, _>::open(FileBackend::new(temp.path()), "b").unwrap();
        log.commit(4).unwrap();
        log.compact().unwrap();
        assert_eq!(log.backend().generations().unwrap(), vec![1]);
        drop(log);
        let log = RunLog::<Sum, _>::open(FileBackend::new(temp.path()), "b").unwrap();
        assert_eq!(*log.state(), 4);
    }
}
