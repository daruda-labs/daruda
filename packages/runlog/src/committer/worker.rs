//! The committer thread: the only code that touches its [`RunLog`].

use std::fmt;
use std::io;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Arc;

use smol::channel::{self, Receiver, Sender};

use super::{AfterCommit, CommitResult, Request, Subscription};
use crate::backend::LogBackend;
use crate::error::{CommitError, SubscribeError};
use crate::log::{Committed, RunLog};
use crate::machine::Machine;
use crate::record::{Body, FORMAT_VERSION, Record};

/// Why the committer refuses further work.
#[derive(Debug, Clone, Copy)]
pub(super) enum Stop {
    /// A write failed and closed the log.
    Closed,
    /// The machine or the hook panicked.
    Poisoned,
    /// The thread is gone.
    Exited,
}

impl fmt::Display for Stop {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Closed => "run log is closed after an earlier write failure",
            Self::Poisoned => "run log committer stopped after a panic",
            Self::Exited => "run log committer has stopped",
        })
    }
}

struct Subscriber<E> {
    after_seq: u64,
    sender: Sender<Arc<Record<E>>>,
}

pub(super) struct Worker<M: Machine, B: LogBackend> {
    log: RunLog<M, B>,
    after_commit: AfterCommit<M>,
    subscribers: Vec<Subscriber<M::Event>>,
    stopped: Option<Stop>,
}

impl<M: Machine, B: LogBackend> Worker<M, B>
where
    M::Event: Clone,
{
    pub(super) fn new(log: RunLog<M, B>, after_commit: AfterCommit<M>) -> Self {
        Self {
            log,
            after_commit,
            subscribers: Vec::new(),
            stopped: None,
        }
    }

    pub(super) fn run(&mut self, queue: &Receiver<Request<M>>) {
        while let Ok(request) = queue.recv_blocking() {
            // A panic outside a proposal drops that request's reply, which
            // its caller reads as stopped; later requests are refused.
            if panic::catch_unwind(AssertUnwindSafe(|| self.handle(request))).is_err() {
                self.stop(Stop::Poisoned);
            }
        }
    }

    fn handle(&mut self, request: Request<M>) {
        // SILENT-OK (every send below): the caller dropped its answer.
        match request {
            Request::Propose { event, reply } => {
                let _ = reply.try_send(self.propose(event));
            }
            Request::Subscribe { after_seq, reply } => {
                let _ = reply.send(self.subscribe(after_seq));
            }
            Request::Compact { reply } => {
                let _ = reply.try_send(self.compact());
            }
            Request::Flush { reply } => {
                let _ = reply.send(());
            }
        }
    }

    fn propose(&mut self, event: M::Event) -> CommitResult<M> {
        match self.stopped {
            Some(Stop::Closed) => return Err(CommitError::Closed),
            Some(Stop::Poisoned | Stop::Exited) => return Err(CommitError::Poisoned),
            None => {}
        }
        let applied = match panic::catch_unwind(AssertUnwindSafe(|| self.log.commit_applied(event)))
        {
            Ok(Ok(applied)) => applied,
            Ok(Err(err @ CommitError::Rejected(_))) => return Err(err),
            Ok(Err(err)) => {
                self.stop(Stop::Closed);
                return Err(err);
            }
            Err(_) => {
                self.stop(Stop::Poisoned);
                return Err(CommitError::Poisoned);
            }
        };
        let record = Arc::new(Record {
            v: FORMAT_VERSION,
            seq: applied.seq,
            at_ms: applied.at_ms,
            body: Body::Event {
                event: applied.event.clone(),
            },
        });
        let state = self.log.state();
        let hook = panic::catch_unwind(AssertUnwindSafe(|| (self.after_commit)(&record, state)));
        // The record is durable either way, so streams stay equal to the log.
        self.fan_out(&record);
        if hook.is_err() {
            self.stop(Stop::Poisoned);
            return Err(CommitError::Poisoned);
        }
        Ok(Committed {
            seq: applied.seq,
            event: applied.event,
            state_after: self.log.state().clone(),
        })
    }

    fn subscribe(&mut self, after_seq: u64) -> Subscription<M> {
        if self.stopped.is_some() {
            return Err(SubscribeError::Stopped);
        }
        let catch_up = self.log.since(after_seq)?;
        let (sender, live) = channel::unbounded();
        self.subscribers.push(Subscriber { after_seq, sender });
        Ok((catch_up, live))
    }

    fn compact(&mut self) -> io::Result<()> {
        if let Some(stop) = self.stopped {
            return Err(io::Error::other(stop.to_string()));
        }
        let compacted = self.log.compact();
        if compacted.is_err() && self.log.is_closed() {
            self.stop(Stop::Closed);
        }
        compacted
    }

    /// A send fails only once the receiver is dropped; that subscriber goes.
    fn fan_out(&mut self, record: &Arc<Record<M::Event>>) {
        self.subscribers.retain(|subscriber| {
            record.seq <= subscriber.after_seq
                || subscriber.sender.try_send(Arc::clone(record)).is_ok()
        });
    }

    /// Ending every stream tells subscribers no further record will come.
    fn stop(&mut self, stop: Stop) {
        self.stopped = Some(stop);
        self.subscribers.clear();
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
    fn a_dropped_subscriber_is_removed_on_the_next_send() {
        let temp = tempfile::tempdir().unwrap();
        let log = RunLog::<Sum, _>::open(FileBackend::new(temp.path()), "b").unwrap();
        let mut worker = Worker::new(log, Box::new(|_, _| {}));
        let (_, kept) = worker.subscribe(0).unwrap();
        let (_, ahead) = worker.subscribe(2).unwrap();
        drop(worker.subscribe(0).unwrap());
        assert_eq!(worker.subscribers.len(), 3);
        worker.propose(3).unwrap();
        assert_eq!(worker.subscribers.len(), 2);
        assert_eq!(kept.try_recv().unwrap().seq, 2);
        // A subscriber only gets records after the seq it asked from.
        assert!(ahead.try_recv().is_err());
    }
}
