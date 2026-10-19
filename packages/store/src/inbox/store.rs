//! One writer and the bot lease that must outlive all its queued claims.

use std::io;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use daruda_runlog::{
    CommitError, Committed, Committer, FileBackend, FlushTimedOut, OpenError, RunLog,
};

use super::{
    ChannelKey, InboxEvent, InboxId, InboxLease, InboxMachine, InboxRejected, InboxSeed, InboxState,
};

#[derive(Debug, thiserror::Error)]
pub enum InboxStoreError {
    #[error("inbox could not be prepared: {0}")]
    Io(#[from] io::Error),
    #[error("inbox log could not be opened: {0}")]
    Open(#[from] OpenError),
    #[error("inbox legacy history could not be imported: {0}")]
    Import(#[from] CommitError<InboxRejected>),
}

/// Field order drains the writer before releasing the original bot lock, and
/// signals [`InboxReleased`] only once that lock is gone.
#[derive(Debug)]
struct OwnedInbox {
    committer: Committer<InboxMachine>,
    _lease: InboxLease,
    _released: smol::channel::Sender<()>,
}

/// Resolves once a writer's bot lease has been released.
///
/// A retired writer can outlive its owner's handle: queued claims, in-flight
/// sends and the asynchronous drain all keep the lease. Until this resolves,
/// the lock reads as held to this process too, so a reopen that asks for it
/// sooner would mistake its own lease for another daruda's.
#[derive(Debug, Clone)]
pub struct InboxReleased(smol::channel::Receiver<()>);

impl InboxReleased {
    pub fn is_released(&self) -> bool {
        self.0.is_closed()
    }

    /// Wait for the release. Nothing is ever sent, so `recv` fails only once
    /// the last sender, owned by the writer, is dropped.
    pub async fn wait(self) {
        while self.0.recv().await.is_ok() {}
    }
}

/// Non-cloneable writer; share an `Arc<InboxStore>`, not its lock handle.
/// Last drop drains the committer off the caller's thread before unlocking.
#[derive(Debug)]
pub struct InboxStore {
    owned: Option<OwnedInbox>,
    snapshot: Arc<RwLock<InboxState>>,
    released: InboxReleased,
}

impl InboxStore {
    /// Open only after acquiring the bot's lease. Opening and importing do
    /// filesystem I/O; desktop callers run this off the foreground thread.
    /// `seed` is evaluated once, including when the legacy input is empty.
    pub fn open(
        lease: InboxLease,
        build: &str,
        seed: impl FnOnce() -> io::Result<InboxSeed>,
    ) -> Result<Self, InboxStoreError> {
        daruda_core::path::create_owner_only_dir(lease.directory())?;
        let mut log = RunLog::<InboxMachine, _>::open(FileBackend::new(lease.directory()), build)?;
        if !log.state().initialized {
            log.commit(InboxEvent::Initialized { seed: seed()? })?;
        }
        let snapshot = Arc::new(RwLock::new(log.state().clone()));
        let projection = snapshot.clone();
        let committer = Committer::spawn(
            log,
            "inbox-log",
            Box::new(move |_, state| {
                *projection
                    .write()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = state.clone();
            }),
        )?;
        Ok(Self::assemble(committer, lease, snapshot))
    }

    fn assemble(
        committer: Committer<InboxMachine>,
        lease: InboxLease,
        snapshot: Arc<RwLock<InboxState>>,
    ) -> Self {
        let (sender, receiver) = smol::channel::bounded(1);
        Self {
            owned: Some(OwnedInbox {
                committer,
                _lease: lease,
                _released: sender,
            }),
            snapshot,
            released: InboxReleased(receiver),
        }
    }

    /// The latest durable state, without replay, subscriptions or disk I/O.
    pub fn snapshot(&self) -> InboxState {
        self.snapshot
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Queue a claim now; canceling its waiter never cancels the write.
    /// Only `Ok` permits its effect. After a write failure or poisoning, retire
    /// this writer and reopen under a newly acquired lease, not its old handle.
    pub fn claim(
        &self,
        channel: ChannelKey,
        id: InboxId,
    ) -> impl Future<Output = Result<Committed<InboxMachine>, CommitError<InboxRejected>>> + use<>
    {
        self.owned
            .as_ref()
            .expect("a live inbox owns its writer")
            .committer
            .propose(InboxEvent::Claimed { channel, id })
    }

    /// A handle that resolves when this writer's lease is released, however
    /// many clones of the store are still alive when it is taken.
    pub fn released(&self) -> InboxReleased {
        self.released.clone()
    }

    pub fn flush(&self, deadline: Duration) -> Result<(), FlushTimedOut> {
        self.owned
            .as_ref()
            .expect("a live inbox owns its writer")
            .committer
            .flush(deadline)
    }
}

impl Drop for InboxStore {
    fn drop(&mut self) {
        if let Some(owned) = self.owned.take() {
            smol::unblock(move || drop(owned)).detach();
        }
    }
}

#[cfg(test)]
mod tests;
