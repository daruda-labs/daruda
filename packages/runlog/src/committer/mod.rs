//! [`Committer`]: a dedicated thread that owns a [`RunLog`], commits
//! proposals in arrival order off the caller's thread, and streams every
//! committed record to subscribers.
//!
//! Per commit, on that thread: [`RunLog`] commit (durable) -> after-commit
//! hook -> subscribers -> the proposer's future resolves.

use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle, ThreadId};
use std::time::Duration;
use std::{fmt, io};

use smol::channel::{self, Receiver, Sender};

use crate::backend::LogBackend;
use crate::error::{CommitError, FlushTimedOut, SubscribeError};
use crate::log::{CatchUp, Committed, RunLog};
use crate::machine::Machine;
use crate::record::Record;

mod worker;

use worker::{Stop, Worker};

/// Runs on the committer thread after a record is durable and before its
/// proposer resumes, so a host can update a projection the proposer reads.
pub type AfterCommit<M> =
    Box<dyn Fn(&Record<<M as Machine>::Event>, &<M as Machine>::State) + Send>;

type Live<E> = Receiver<Arc<Record<E>>>;
type CommitResult<M> = Result<Committed<M>, CommitError<<M as Machine>::Rejected>>;
type Subscription<M> = Result<(CatchUp<M>, Live<<M as Machine>::Event>), SubscribeError>;

enum Request<M: Machine> {
    Propose {
        event: M::Event,
        reply: Sender<CommitResult<M>>,
    },
    Subscribe {
        after_seq: u64,
        reply: mpsc::SyncSender<Subscription<M>>,
    },
    Compact {
        reply: Sender<io::Result<()>>,
    },
    Flush {
        reply: mpsc::SyncSender<()>,
    },
}

/// A cheap, cloneable handle to the thread that owns one [`RunLog`]. The
/// last handle dropped lets the thread drain what is queued, then joins it.
pub struct Committer<M: Machine> {
    shared: Arc<Shared<M>>,
}

struct Shared<M: Machine> {
    requests: Sender<Request<M>>,
    thread_id: ThreadId,
    thread: Option<JoinHandle<()>>,
}

impl<M: Machine> Clone for Committer<M> {
    fn clone(&self) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
        }
    }
}

impl<M: Machine> fmt::Debug for Committer<M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Committer")
            .field("thread", &self.shared.thread_id)
            .finish_non_exhaustive()
    }
}

impl<M: Machine> Drop for Shared<M> {
    fn drop(&mut self) {
        // Closing still lets the thread receive everything already queued.
        self.requests.close();
        let Some(thread) = self.thread.take() else {
            return;
        };
        // A hook that held the last handle would otherwise join itself.
        if thread.thread().id() != thread::current().id() {
            // SILENT-OK: panics are caught per request on the thread, and a
            // destructor has nowhere to report one that escaped.
            let _ = thread.join();
        }
    }
}

impl<M: Machine> Committer<M>
where
    M: 'static,
    M::State: Send,
    M::Event: Clone + Send + Sync,
    M::Rejected: Send,
{
    /// Move `log` onto a new thread named `name`.
    pub fn spawn<B: LogBackend + Send + 'static>(
        log: RunLog<M, B>,
        name: &str,
        after_commit: AfterCommit<M>,
    ) -> io::Result<Self> {
        let (requests, queue) = channel::unbounded();
        let mut worker = Worker::new(log, after_commit);
        let thread = thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || worker.run(&queue))?;
        Ok(Self {
            shared: Arc::new(Shared {
                requests,
                thread_id: thread.thread().id(),
                thread: Some(thread),
            }),
        })
    }

    /// Queue `event` now; the future resolves once it is durable, the hook
    /// has run, and subscribers have it. The future borrows nothing, so a
    /// `!Send` caller can await it on any executor.
    pub fn propose(&self, event: M::Event) -> impl Future<Output = CommitResult<M>> + use<M> {
        let (reply, answer) = channel::bounded(1);
        // SILENT-OK: a failed send drops `reply`, which resolves `Closed`.
        let _ = self
            .shared
            .requests
            .try_send(Request::Propose { event, reply });
        async move { answer.recv().await.unwrap_or(Err(CommitError::Closed)) }
    }

    /// Records after `after_seq` (or a snapshot, if they were compacted
    /// away), then every later record live. Both are taken on the committer
    /// thread between two commits, so there is no gap and no duplicate.
    /// Blocks while earlier requests are processed.
    pub fn subscribe(&self, after_seq: u64) -> Subscription<M> {
        if thread::current().id() == self.shared.thread_id {
            return Err(SubscribeError::OnCommitterThread);
        }
        let (reply, answer) = mpsc::sync_channel(1);
        let request = Request::Subscribe { after_seq, reply };
        if self.shared.requests.try_send(request).is_err() {
            return Err(SubscribeError::Stopped);
        }
        answer.recv().unwrap_or(Err(SubscribeError::Stopped))
    }

    /// Start a new generation once every earlier request is processed.
    pub fn compact(&self) -> impl Future<Output = io::Result<()>> + use<M> {
        let (reply, answer) = channel::bounded(1);
        // SILENT-OK: a failed send drops `reply`, which resolves to an error.
        let _ = self.shared.requests.try_send(Request::Compact { reply });
        async move {
            answer
                .recv()
                .await
                .unwrap_or_else(|_| Err(io::Error::other(Stop::Exited.to_string())))
        }
    }

    /// Block until every request queued before this call is answered, or
    /// `deadline` passes. A stopped committer answers at once. Calling it
    /// from the after-commit hook always times out.
    pub fn flush(&self, deadline: Duration) -> Result<(), FlushTimedOut> {
        let (reply, answer) = mpsc::sync_channel(1);
        if self
            .shared
            .requests
            .try_send(Request::Flush { reply })
            .is_err()
        {
            return Ok(());
        }
        match answer.recv_timeout(deadline) {
            // A thread that is gone dropped its queue, answering it `Closed`.
            Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => Ok(()),
            Err(mpsc::RecvTimeoutError::Timeout) => Err(FlushTimedOut { deadline }),
        }
    }
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
    fn a_handle_can_be_shared_across_threads() {
        fn shareable<T: Clone + Send + Sync>() {}
        shareable::<Committer<Sum>>();
    }
}
