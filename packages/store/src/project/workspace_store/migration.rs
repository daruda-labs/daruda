//! Copy a small workspace snapshot, then atomically publish its whole directory.
//! Interrupted attempts leave the source authoritative and can be retried.

use std::fs::{self, OpenOptions, TryLockError};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::project::{
    ProjectState, RecentEntry, WORKSPACE_SCHEMA_VERSION, WorkspaceState, persistence,
};

const MARKER: &str = ".workspace-layout.json";
const VERSION: u32 = 1;

#[derive(Deserialize, Serialize)]
struct LayoutVersion {
    version: u32,
}

pub(super) struct WorkspaceMigration {
    source: PathBuf,
    destination: PathBuf,
}

struct Record {
    relative: PathBuf,
    bytes: Vec<u8>,
}

enum RecordKind {
    Workspace,
    Project,
    Recent,
}

impl WorkspaceMigration {
    pub(super) fn new(source: PathBuf, destination: PathBuf) -> Self {
        Self {
            source,
            destination,
        }
    }

    pub(super) fn run(&self) -> io::Result<()> {
        self.run_inner().map_err(|error| {
            io::Error::new(
                error.kind(),
                format!(
                    "Workspace storage preparation from {} to {} failed: {error}",
                    self.source.display(),
                    self.destination.display(),
                ),
            )
        })
    }

    fn run_inner(&self) -> io::Result<()> {
        let parent = self
            .destination
            .parent()
            .ok_or_else(|| invalid("State storage needs a parent"))?;
        daruda_core::path::create_owner_only_dir(parent)?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(parent.join(".workspace-migration.lock"))?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "Workspace storage migration is already running",
                ));
            }
            Err(TryLockError::Error(error)) => return Err(error),
        }
        if self.is_published()? {
            return Ok(());
        }
        let records = self.snapshot()?;
        let staging = tempfile::Builder::new()
            .prefix(".workspace-migration-")
            .tempdir_in(parent)?;
        for record in records {
            let path = staging.path().join(record.relative);
            if let Some(parent) = path.parent() {
                daruda_core::path::create_owner_only_dir(parent)?;
            }
            let mut file = daruda_core::path::open_owner_only(
                OpenOptions::new().create_new(true).write(true),
                &path,
            )?;
            file.write_all(&record.bytes)?;
            file.sync_all()?;
            if fs::read(&path)? != record.bytes {
                return Err(invalid("Workspace migration copy verification failed"));
            }
        }
        let marker = staging.path().join(MARKER);
        let mut file = daruda_core::path::open_owner_only(
            OpenOptions::new().create_new(true).write(true),
            &marker,
        )?;
        serde_json::to_writer(&mut file, &LayoutVersion { version: VERSION }).map_err(invalid)?;
        file.sync_all()?;
        drop(file);
        // INVARIANT: the target was absent under the per-destination lock.
        // Only a fully verified snapshot and its marker become visible.
        fs::rename(staging.path(), &self.destination)?;
        Ok(())
    }

    fn is_published(&self) -> io::Result<bool> {
        let metadata = match fs::symlink_metadata(&self.destination) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error),
        };
        if !metadata.file_type().is_dir() {
            return Err(invalid("Workspace destination is not a regular directory"));
        }
        let marker = self.destination.join(MARKER);
        let marker_metadata = fs::symlink_metadata(&marker).map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                invalid(format!(
                    "Workspace destination {} already exists without a layout marker; destination preserved",
                    self.destination.display(),
                ))
            } else {
                error
            }
        })?;
        if !marker_metadata.file_type().is_file() {
            return Err(invalid("Workspace layout marker is not a regular file"));
        }
        let layout: LayoutVersion = serde_json::from_slice(&fs::read(marker)?).map_err(invalid)?;
        if layout.version != VERSION {
            return Err(invalid(
                "Unsupported workspace storage layout; destination preserved",
            ));
        }
        Ok(true)
    }

    fn snapshot(&self) -> io::Result<Vec<Record>> {
        let mut records = Vec::new();
        for (directory, kind) in [
            (
                persistence::workspaces_dir_in(&self.source),
                RecordKind::Workspace,
            ),
            (
                persistence::projects_dir_in(&self.source),
                RecordKind::Project,
            ),
        ] {
            let metadata = match fs::symlink_metadata(&directory) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            if !metadata.file_type().is_dir() {
                return Err(invalid(
                    "Legacy workspace storage is not a regular directory",
                ));
            }
            for entry in fs::read_dir(directory)? {
                let entry = entry?;
                let path = entry.path();
                let selected = path.extension().is_some_and(|ext| ext == "json")
                    && path
                        .file_stem()
                        .and_then(|stem| stem.to_str())
                        .is_some_and(persistence::is_uuid_filename_stem);
                if selected {
                    records.push(self.read_record(&path, &kind)?);
                }
            }
        }
        let recent = persistence::recent_path_in(&self.source);
        match fs::symlink_metadata(&recent) {
            Ok(_) => records.push(self.read_record(&recent, &RecordKind::Recent)?),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        Ok(records)
    }

    fn read_record(&self, path: &Path, kind: &RecordKind) -> io::Result<Record> {
        self.read_record_inner(path, kind).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("Cannot import workspace record {}: {error}", path.display()),
            )
        })
    }

    fn read_record_inner(&self, path: &Path, kind: &RecordKind) -> io::Result<Record> {
        if !fs::symlink_metadata(path)?.file_type().is_file() {
            return Err(invalid("Legacy state record is not a regular file"));
        }
        let bytes = fs::read(path)?;
        let (version, id) = match kind {
            RecordKind::Workspace => {
                let state: WorkspaceState = serde_json::from_slice(&bytes).map_err(invalid)?;
                (
                    state.schema_version,
                    Some(state.uuid.as_inner().to_string()),
                )
            }
            RecordKind::Project => {
                let state: ProjectState = serde_json::from_slice(&bytes).map_err(invalid)?;
                (
                    state.schema_version,
                    Some(state.uuid.as_inner().to_string()),
                )
            }
            RecordKind::Recent => {
                let _: Vec<RecentEntry> = serde_json::from_slice(&bytes).map_err(invalid)?;
                (WORKSPACE_SCHEMA_VERSION, None)
            }
        };
        if version > WORKSPACE_SCHEMA_VERSION {
            return Err(invalid(
                "Legacy state uses a newer schema; migration stopped",
            ));
        }
        if let Some(id) = id
            && path.file_stem().and_then(|stem| stem.to_str()) != Some(id.as_str())
        {
            return Err(invalid("Legacy state UUID does not match its file name"));
        }
        let relative = path
            .strip_prefix(&self.source)
            .map_err(invalid)?
            .to_path_buf();
        Ok(Record { relative, bytes })
    }
}

fn invalid(error: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}

#[cfg(test)]
mod tests;
