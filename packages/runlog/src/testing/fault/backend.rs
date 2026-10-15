//! Byte operations and durable publication for the fault-injecting backend.

use std::io;

use super::{DirOp, Fault, FaultBackend, injected, log_name, snapshot_name};
use crate::backend::{Generation, LogBackend};

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
        let snapshot = snapshot_name(generation);
        if generation != 0 && inner.live.contains_key(&snapshot) {
            inner.publish(&snapshot)?;
        }
        let name = log_name(generation);
        if !inner.live.contains_key(&name) {
            inner.live.insert(name.clone(), Vec::new());
            if inner.reflected(DirOp::CreateLog(generation)) {
                inner.disk.insert(name.clone(), Vec::new());
            }
        }
        inner.publish(&name)?;
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
            inner.disk.insert(name.clone(), bytes.to_vec());
        }
        inner.publish(&name)
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
mod tests {}
