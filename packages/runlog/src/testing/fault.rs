//! An in-memory [`LogBackend`] that keeps what a crash would preserve apart
//! from what the process sees, and fails on command.

use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::backend::{Generation, LogBackend};

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
/// the crashed disk: a prefix of metadata changes survives, as on a
/// journaling filesystem without a directory fsync.
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

impl LogBackend for FaultBackend {
    fn generations(&self) -> io::Result<Vec<u64>> {
        let inner = self.lock();
        let mut found: Vec<u64> = inner
            .live
            .keys()
            .filter_map(|name| {
                name.strip_prefix("snapshot.")
                    .and_then(|rest| rest.strip_suffix(".json"))
                    .or_else(|| {
                        name.strip_prefix("log.")
                            .and_then(|rest| rest.strip_suffix(".jsonl"))
                    })
                    .and_then(|number| number.parse().ok())
            })
            .collect();
        found.sort_unstable();
        found.dedup();
        Ok(found)
    }

    fn read(&self, generation: u64) -> io::Result<Generation> {
        let inner = self.lock();
        let snapshot = (generation != 0)
            .then(|| inner.live.get(&snapshot_name(generation)).cloned())
            .flatten();
        let log = inner
            .live
            .get(&log_name(generation))
            .cloned()
            .unwrap_or_default();
        Ok(Generation {
            generation,
            snapshot,
            log,
        })
    }

    fn select(&mut self, generation: u64) -> io::Result<()> {
        let mut inner = self.lock();
        inner.alive()?;
        let name = log_name(generation);
        if !inner.live.contains_key(&name) {
            inner.live.insert(name.clone(), Vec::new());
            if inner.reflected(DirOp::CreateLog(generation)) {
                inner.disk.insert(name, Vec::new());
            }
        }
        inner.selected = Some(generation);
        Ok(())
    }

    fn append(&mut self, line: &[u8]) -> io::Result<()> {
        let mut inner = self.lock();
        if inner.dead {
            return Err(injected());
        }
        let name = inner.selected_log()?;
        let k = inner.append_lens.len();
        inner.append_lens.push(line.len());
        match inner.fault {
            Some(Fault::TornAppend { k: at, keep }) if at == k => {
                inner.write(&name, &line[..keep.min(line.len())], true);
                inner.dead = true;
                Err(injected())
            }
            _ if inner.power_lost() => {
                inner.write(&name, line, false);
                inner.dead = true;
                Err(injected())
            }
            _ => {
                inner.write(&name, line, false);
                Ok(())
            }
        }
    }

    fn sync(&mut self) -> io::Result<()> {
        let mut inner = self.lock();
        inner.alive()?;
        let name = inner.selected_log()?;
        let landed = inner.live.get(&name).cloned().unwrap_or_default();
        if let Some(file) = inner.disk.get_mut(&name) {
            *file = landed;
        }
        if matches!(inner.fault, Some(Fault::FailSyncAfter { k }) if k + 1 == inner.append_lens.len())
        {
            inner.dead = true;
            return Err(injected());
        }
        inner.syncs += 1;
        Ok(())
    }

    fn truncate_tail(&mut self, len: u64) -> io::Result<()> {
        let mut inner = self.lock();
        inner.alive()?;
        let name = inner.selected_log()?;
        let len = usize::try_from(len).map_err(io::Error::other)?;
        if let Some(file) = inner.live.get_mut(&name) {
            file.truncate(len);
        }
        if let Some(file) = inner.disk.get_mut(&name) {
            file.truncate(len);
        }
        Ok(())
    }

    fn replace_snapshot(&mut self, generation: u64, bytes: &[u8]) -> io::Result<()> {
        let mut inner = self.lock();
        inner.alive()?;
        let name = snapshot_name(generation);
        inner.live.insert(name.clone(), bytes.to_vec());
        if inner.reflected(DirOp::ReplaceSnapshot(generation)) {
            inner.disk.insert(name, bytes.to_vec());
        }
        Ok(())
    }

    fn drop_generation(&mut self, generation: u64) -> io::Result<()> {
        let mut inner = self.lock();
        inner.alive()?;
        if inner.selected == Some(generation) {
            inner.selected = None;
        }
        let names = [log_name(generation), snapshot_name(generation)];
        for name in &names {
            inner.live.remove(name);
        }
        if inner.reflected(DirOp::DropGeneration(generation)) {
            for name in &names {
                inner.disk.remove(name);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selected(fault: Option<Fault>) -> FaultBackend {
        let mut backend = FaultBackend::new();
        if let Some(fault) = fault {
            backend = backend.with_fault(fault);
        }
        backend.select(0).unwrap();
        backend
    }

    #[test]
    fn unsynced_bytes_do_not_survive_a_crash() {
        let mut backend = selected(None);
        backend.append(b"a\n").unwrap();
        backend.sync().unwrap();
        backend.append(b"b\n").unwrap();
        assert_eq!(backend.file("log.0.jsonl").unwrap(), b"a\nb\n");
        assert_eq!(backend.crash().file("log.0.jsonl").unwrap(), b"a\n");
    }

    #[test]
    fn a_torn_append_leaves_its_prefix_on_disk_and_kills_the_backend() {
        let mut backend = selected(Some(Fault::TornAppend { k: 1, keep: 2 }));
        backend.append(b"a\n").unwrap();
        backend.sync().unwrap();
        assert!(backend.append(b"bcd\n").is_err());
        assert!(backend.sync().is_err());
        assert_eq!(backend.crash().file("log.0.jsonl").unwrap(), b"a\nbc");
    }

    #[test]
    fn a_failed_sync_still_lands_the_line() {
        let mut backend = selected(Some(Fault::FailSyncAfter { k: 0 }));
        backend.append(b"a\n").unwrap();
        assert!(backend.sync().is_err());
        assert!(backend.append(b"b\n").is_err());
        assert_eq!(backend.crash().file("log.0.jsonl").unwrap(), b"a\n");
    }

    #[test]
    fn after_a_power_cut_the_next_append_is_lost() {
        let mut backend = selected(Some(Fault::LoseUnsyncedAfter { k: 0 }));
        backend.append(b"a\n").unwrap();
        backend.sync().unwrap();
        assert!(backend.append(b"b\n").is_err());
        assert_eq!(backend.file("log.0.jsonl").unwrap(), b"a\nb\n");
        assert_eq!(backend.crash().file("log.0.jsonl").unwrap(), b"a\n");
    }

    #[test]
    fn lost_directory_operations_are_a_suffix() {
        let mut backend = FaultBackend::new().lose_dir_ops_from(DirOp::ReplaceSnapshot(1));
        backend.select(0).unwrap();
        backend.replace_snapshot(1, b"s").unwrap();
        backend.drop_generation(0).unwrap();
        assert_eq!(backend.generations().unwrap(), vec![1]);
        assert_eq!(backend.crash().generations().unwrap(), vec![0]);
    }
}
