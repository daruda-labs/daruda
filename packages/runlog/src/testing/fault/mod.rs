//! An in-memory [`LogBackend`](crate::LogBackend) that keeps what a crash would preserve apart
//! from what the process sees, and fails on command.

use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex, MutexGuard};

mod backend;

#[cfg(test)]
mod tests;

/// One injected failure. `k` is 0-based.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    /// The k-th append writes only `keep` bytes of its line, then every call
    /// fails. The partial bytes reach disk.
    TornAppend { k: usize, keep: usize },
    /// The k-th append lands in full, then the following sync fails. The
    /// line reaches disk anyway, as an unacknowledged write may.
    FailSyncAfter { k: usize },
    /// Everything after the k-th sync is lost (power-cut model): the next
    /// append reaches only volatile memory and fails, and so does every
    /// later call.
    LoseUnsyncedAfter { k: usize },
}

/// A directory operation. With [`FaultBackend::lose_dir_ops_from`], the
/// named operation and every directory operation after it are missing from
/// the crashed disk. Publication of a lost create or snapshot replacement
/// fails before the log can acknowledge another event; lost cleanup is safe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirOp {
    ReplaceSnapshot(u64),
    CreateLog(u64),
    DropGeneration(u64),
}

#[derive(Debug, Default)]
struct Inner {
    /// What reads see.
    live: BTreeMap<String, Vec<u8>>,
    /// What survives a crash.
    disk: BTreeMap<String, Vec<u8>>,
    selected: Option<u64>,
    fault: Option<Fault>,
    lose_from: Option<DirOp>,
    losing: bool,
    publication_fails: bool,
    append_lens: Vec<usize>,
    syncs: usize,
    dead: bool,
}

/// A handle to one simulated directory. Clones share it, so a test can keep
/// a handle while a [`RunLog`](crate::RunLog) owns another.
#[derive(Debug, Clone, Default)]
pub struct FaultBackend {
    inner: Arc<Mutex<Inner>>,
}

fn log_name(generation: u64) -> String {
    format!("log.{generation}.jsonl")
}

fn snapshot_name(generation: u64) -> String {
    format!("snapshot.{generation}.json")
}

fn injected() -> io::Error {
    io::Error::other("injected fault")
}

impl Inner {
    fn power_lost(&self) -> bool {
        matches!(self.fault, Some(Fault::LoseUnsyncedAfter { k }) if self.syncs > k)
    }

    /// Fail every call once a fault fired or the power is gone.
    fn alive(&mut self) -> io::Result<()> {
        if self.dead || self.power_lost() {
            self.dead = true;
            return Err(injected());
        }
        Ok(())
    }

    /// Whether `op` reaches the crashed disk.
    fn reflected(&mut self, op: DirOp) -> bool {
        if self.lose_from == Some(op) {
            self.losing = true;
        }
        !self.losing
    }

    fn selected_log(&self) -> io::Result<String> {
        self.selected
            .map(log_name)
            .ok_or_else(|| io::Error::other("no run log generation is selected"))
    }

    fn write(&mut self, name: &str, bytes: &[u8], durable: bool) {
        self.live
            .entry(name.to_owned())
            .or_default()
            .extend_from_slice(bytes);
        if durable && let Some(file) = self.disk.get_mut(name) {
            file.extend_from_slice(bytes);
        }
    }

    fn publish(&mut self, name: &str) -> io::Result<()> {
        self.alive()?;
        if self.losing || self.publication_fails {
            self.dead = true;
            return Err(injected());
        }
        let bytes = self.live.get(name).cloned().ok_or_else(injected)?;
        self.disk.insert(name.to_owned(), bytes);
        Ok(())
    }
}

impl FaultBackend {
    /// An empty directory with no fault.
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_fault(self, fault: Fault) -> Self {
        self.lock().fault = Some(fault);
        self
    }

    pub fn lose_dir_ops_from(self, op: DirOp) -> Self {
        self.lock().lose_from = Some(op);
        self
    }

    /// Fail every durable publication, including one for an existing path.
    pub fn fail_publication(self) -> Self {
        self.lock().publication_fails = true;
        self
    }

    /// Restart the process without a power cut: cached files still exist,
    /// but unpublished names need a successful publication before use.
    pub fn restart(&self) -> Self {
        let inner = self.lock();
        Self {
            inner: Arc::new(Mutex::new(Inner {
                live: inner.live.clone(),
                disk: inner.disk.clone(),
                ..Inner::default()
            })),
        }
    }

    /// A fresh, fault-free backend holding only what survived a crash now.
    pub fn crash(&self) -> Self {
        let disk = self.lock().disk.clone();
        let inner = Inner {
            live: disk.clone(),
            disk,
            ..Inner::default()
        };
        Self {
            inner: Arc::new(Mutex::new(inner)),
        }
    }

    /// The byte length of every append attempted so far, in order.
    pub fn append_lens(&self) -> Vec<usize> {
        self.lock().append_lens.clone()
    }

    /// Syncs that succeeded so far.
    pub fn syncs(&self) -> usize {
        self.lock().syncs
    }

    /// The bytes a reader sees in `name`, such as `log.0.jsonl`.
    pub fn file(&self, name: &str) -> Option<Vec<u8>> {
        self.lock().live.get(name).cloned()
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // A test that panicked while holding the lock already failed.
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}
