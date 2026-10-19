//! The storage floor under a [`RunLog`](crate::RunLog): byte-level file
//! operations with no knowledge of records, snapshots, or machines.

use std::collections::BTreeSet;
use std::fs::{File, OpenOptions};
use std::io::{self, Seek as _, SeekFrom, Write as _};
use std::path::{Path, PathBuf};

/// One generation as stored: generation 0 never has a snapshot, and a
/// missing log reads as empty.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Generation {
    pub generation: u64,
    pub snapshot: Option<Vec<u8>>,
    pub log: Vec<u8>,
}

/// Where a run log's bytes live. `append`, `sync`, and `truncate_tail` act
/// on the log of the generation last passed to `select`.
pub trait LogBackend {
    /// Generations with a snapshot or a log present, ascending.
    fn generations(&self) -> io::Result<Vec<u64>>;
    fn read(&self, generation: u64) -> io::Result<Generation>;
    /// Durably publish `generation`'s snapshot and log under their final names,
    /// then select its log, creating it empty if absent. Existing paths must
    /// also be published: an earlier failed publication may have left them.
    fn select(&mut self, generation: u64) -> io::Result<()>;
    fn append(&mut self, line: &[u8]) -> io::Result<()>;
    fn sync(&mut self) -> io::Result<()>;
    /// Cut the selected log to `len` bytes, durably.
    fn truncate_tail(&mut self, len: u64) -> io::Result<()>;
    /// Atomically replace `generation`'s snapshot with `bytes` and durably
    /// publish its final name before returning success.
    fn replace_snapshot(&mut self, generation: u64, bytes: &[u8]) -> io::Result<()>;
    fn drop_generation(&mut self, generation: u64) -> io::Result<()>;
}

/// `snapshot.<g>.json` and `log.<g>.jsonl` in a directory the caller chose
/// and created; this type never resolves paths itself.
#[derive(Debug)]
pub struct FileBackend {
    dir: PathBuf,
    selected: Option<(u64, File)>,
}

impl FileBackend {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            selected: None,
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The file holding `generation`'s log, for a person or a test that
    /// looks at the bytes. Reading records goes through
    /// [`RunLog::read_history`](crate::RunLog::read_history), which applies
    /// replay's rules.
    pub fn log_path(&self, generation: u64) -> PathBuf {
        self.dir.join(format!("log.{generation}.jsonl"))
    }

    /// Whether a log was ever opened here: any generation's file is present.
    /// A missing directory holds none.
    pub fn holds_log(&self) -> io::Result<bool> {
        match self.generations() {
            Ok(generations) => Ok(!generations.is_empty()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(err) => Err(err),
        }
    }

    fn snapshot_path(&self, generation: u64) -> PathBuf {
        self.dir.join(format!("snapshot.{generation}.json"))
    }

    fn selected(&mut self) -> io::Result<&mut (u64, File)> {
        self.selected
            .as_mut()
            .ok_or_else(|| io::Error::other("no run log generation is selected"))
    }
}

/// The generation a run-log file name belongs to; anything else, including
/// a leftover snapshot temp file, is not part of a generation.
fn generation_of(name: &str) -> Option<u64> {
    let number = name
        .strip_prefix("snapshot.")
        .and_then(|rest| rest.strip_suffix(".json"))
        .or_else(|| {
            name.strip_prefix("log.")
                .and_then(|rest| rest.strip_suffix(".jsonl"))
        })?;
    number.parse().ok()
}

fn read_if_present(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

fn remove_if_present(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Err(err) if err.kind() != io::ErrorKind::NotFound => Err(err),
        _ => Ok(()),
    }
}

impl LogBackend for FileBackend {
    fn generations(&self) -> io::Result<Vec<u64>> {
        let mut found = BTreeSet::new();
        for entry in std::fs::read_dir(&self.dir)? {
            let name = entry?.file_name();
            if let Some(generation) = name.to_str().and_then(generation_of) {
                found.insert(generation);
            }
        }
        Ok(found.into_iter().collect())
    }

    fn read(&self, generation: u64) -> io::Result<Generation> {
        let snapshot = if generation == 0 {
            None
        } else {
            read_if_present(&self.snapshot_path(generation))?
        };
        let log = read_if_present(&self.log_path(generation))?.unwrap_or_default();
        Ok(Generation {
            generation,
            snapshot,
            log,
        })
    }

    fn select(&mut self, generation: u64) -> io::Result<()> {
        if generation != 0 {
            let snapshot = self.snapshot_path(generation);
            match OpenOptions::new().write(true).open(&snapshot) {
                Ok(file) => daruda_core::path::sync_file_entry(&file, &snapshot)?,
                Err(err) if err.kind() == io::ErrorKind::NotFound => {}
                Err(err) => return Err(err),
            }
        }
        let path = self.log_path(generation);
        // Not append mode: on Windows it drops the write-data right that
        // flushing needs. One writer per log makes seek-to-end equivalent.
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;
        file.seek(SeekFrom::End(0))?;
        daruda_core::path::sync_file_entry(&file, &path)?;
        self.selected = Some((generation, file));
        Ok(())
    }

    fn append(&mut self, line: &[u8]) -> io::Result<()> {
        self.selected()?.1.write_all(line)
    }

    fn sync(&mut self) -> io::Result<()> {
        self.selected()?.1.sync_data()
    }

    fn truncate_tail(&mut self, len: u64) -> io::Result<()> {
        let file = &mut self.selected()?.1;
        file.set_len(len)?;
        // `set_len` leaves the cursor where it was; move it to the new end so
        // the next append does not leave a hole.
        file.seek(SeekFrom::End(0))?;
        file.sync_data()
    }

    fn replace_snapshot(&mut self, generation: u64, bytes: &[u8]) -> io::Result<()> {
        let target = self.snapshot_path(generation);
        let temp = self.dir.join(format!("snapshot.{generation}.json.tmp"));
        let mut file = File::create(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temp, &target)?;
        let file = OpenOptions::new().write(true).open(&target)?;
        daruda_core::path::sync_file_entry(&file, &target)
    }

    fn drop_generation(&mut self, generation: u64) -> io::Result<()> {
        if matches!(self.selected, Some((selected, _)) if selected == generation) {
            self.selected = None;
        }
        remove_if_present(&self.log_path(generation))?;
        remove_if_present(&self.snapshot_path(generation))?;
        daruda_core::path::sync_dir(&self.dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn append_sync_and_truncate_through_one_write_handle() {
        let temp = tempfile::tempdir().unwrap();
        let mut backend = FileBackend::new(temp.path());
        backend.select(0).unwrap();
        backend.append(b"one\n").unwrap();
        backend.append(b"tw").unwrap();
        backend.sync().unwrap();
        assert_eq!(backend.read(0).unwrap().log, b"one\ntw");

        backend.truncate_tail(4).unwrap();
        // The same handle keeps writing right after `len`, not past a hole.
        backend.append(b"two\n").unwrap();
        backend.sync().unwrap();
        assert_eq!(backend.read(0).unwrap().log, b"one\ntwo\n");
    }

    #[test]
    fn replacing_a_snapshot_leaves_no_temp_file_behind() {
        let temp = tempfile::tempdir().unwrap();
        let mut backend = FileBackend::new(temp.path());
        backend.replace_snapshot(1, b"first").unwrap();
        backend.replace_snapshot(1, b"second").unwrap();

        assert_eq!(names(temp.path()), vec!["snapshot.1.json"]);
        let generation = backend.read(1).unwrap();
        assert_eq!(generation.snapshot.as_deref(), Some(&b"second"[..]));
        assert!(generation.log.is_empty());
    }

    #[test]
    fn generations_ignore_unrelated_files_and_drop_removes_both_files() {
        let temp = tempfile::tempdir().unwrap();
        let mut backend = FileBackend::new(temp.path());
        backend.select(0).unwrap();
        backend.replace_snapshot(2, b"s").unwrap();
        backend.select(2).unwrap();
        std::fs::write(temp.path().join("snapshot.3.json.tmp"), b"x").unwrap();
        std::fs::write(temp.path().join("notes.txt"), b"x").unwrap();
        assert_eq!(backend.generations().unwrap(), vec![0, 2]);

        backend.drop_generation(2).unwrap();
        assert_eq!(backend.generations().unwrap(), vec![0]);
        assert!(
            backend.append(b"x").is_err(),
            "the dropped log is no target"
        );
        backend.drop_generation(5).unwrap();
    }

    #[test]
    fn a_log_is_held_once_any_generation_is_present() {
        let temp = tempfile::tempdir().unwrap();
        assert!(
            !FileBackend::new(temp.path().join("absent"))
                .holds_log()
                .unwrap()
        );
        let mut backend = FileBackend::new(temp.path());
        std::fs::write(temp.path().join("notes.txt"), b"x").unwrap();
        assert!(!backend.holds_log().unwrap());
        backend.select(0).unwrap();
        assert!(backend.holds_log().unwrap());
        assert_eq!(backend.log_path(0), temp.path().join("log.0.jsonl"));
    }

    #[test]
    fn generation_zero_never_reads_a_snapshot() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("snapshot.0.json"), b"stray").unwrap();
        let backend = FileBackend::new(temp.path());
        assert_eq!(backend.read(0).unwrap(), Generation::default());
    }
}
