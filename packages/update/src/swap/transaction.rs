//! Durable phase tracking prevents interrupted file swaps losing their rollback sources.

use super::{
    ASIDE_SUFFIX, PlannedFile, STAGED_SUFFIX, commit, free_aside, io, plan, stage, staged_path,
};
use crate::UpdateError;
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

const JOURNAL: &str = ".daruda-update-journal.json";

#[derive(Serialize, Deserialize)]
struct Entry {
    live: PathBuf,
    aside: Option<PathBuf>,
}

#[derive(Serialize, Deserialize)]
struct Journal {
    #[serde(flatten)]
    phase: Phase,
    files: Vec<Entry>,
}

/// In memory, a swap has exactly one phase. The wire format remains the
/// original two booleans so an older installer can recover this journal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "PhaseRecord", into = "PhaseRecord")]
enum Phase {
    Staging,
    Prepared,
    Committed,
}

#[derive(Serialize, Deserialize)]
struct PhaseRecord {
    committed: bool,
    #[serde(default)]
    prepared: bool,
}

impl From<PhaseRecord> for Phase {
    fn from(record: PhaseRecord) -> Self {
        // Old journals may omit prepared, including committed journals.
        // Committed has always taken precedence during recovery.
        if record.committed {
            Self::Committed
        } else if record.prepared {
            Self::Prepared
        } else {
            Self::Staging
        }
    }
}

impl From<Phase> for PhaseRecord {
    fn from(phase: Phase) -> Self {
        Self {
            committed: phase == Phase::Committed,
            prepared: phase != Phase::Staging,
        }
    }
}

fn lock(root: &Path) -> Result<std::fs::File, UpdateError> {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(root.join(".daruda-update.lock"))
        .map_err(io)?;
    file.try_lock()
        .map_err(|error| io(std::io::Error::from(error)))?;
    Ok(file)
}

fn publish(root: &Path, journal: &Journal) -> Result<(), UpdateError> {
    use std::io::Write as _;
    let mut tmp = tempfile::NamedTempFile::new_in(root).map_err(io)?;
    tmp.write_all(
        &serde_json::to_vec(journal).map_err(|error| UpdateError::Sync(error.to_string()))?,
    )
    .map_err(io)?;
    tmp.as_file().sync_all().map_err(io)?;
    tmp.persist(root.join(JOURNAL))
        .map_err(|error| io(error.error))?;
    Ok(())
}

fn checked(root: &Path, relative: &Path) -> Result<PathBuf, UpdateError> {
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(UpdateError::Sync(
            "Update journal contains an invalid relative path".into(),
        ));
    }
    let path = root.join(relative);
    // Refuse symlinked parents rather than letting a journal leave the install.
    let mut parent = path.parent();
    while let Some(directory) = parent {
        if directory == root {
            break;
        }
        let metadata = match std::fs::symlink_metadata(directory) {
            Ok(metadata) => Some(metadata),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(io(error)),
        };
        if metadata.is_some_and(|metadata| metadata.file_type().is_symlink()) {
            return Err(UpdateError::Sync(
                "Update journal traverses a symlink".into(),
            ));
        }
        parent = directory.parent();
    }
    Ok(path)
}

fn reserved(relative: &Path) -> bool {
    let name = relative.to_string_lossy();
    let name = if cfg!(windows) {
        name.to_ascii_lowercase()
    } else {
        name.into_owned()
    };
    name.contains(ASIDE_SUFFIX)
        || name.contains(STAGED_SUFFIX)
        || name == JOURNAL
        || name == ".daruda-update.lock"
}

pub(super) fn install(bundle: &Path, root: &Path) -> Result<(), UpdateError> {
    let _ownership = lock(root)?;
    recover_owned(root)?;
    let mut prepared = Vec::new();
    let mut files = Vec::new();
    for (source, live) in plan(bundle, root)? {
        let relative = live
            .strip_prefix(root)
            .map_err(|error| UpdateError::Sync(error.to_string()))?;
        checked(root, relative)?;
        if reserved(relative) {
            return Err(UpdateError::Sync(
                "Package contains reserved update paths".into(),
            ));
        }
        let aside = if entry_exists(&live)? {
            Some(free_aside(&live)?)
        } else {
            None
        };
        files.push(Entry {
            live: relative.to_path_buf(),
            aside: aside
                .as_ref()
                .map(|path| {
                    path.strip_prefix(root)
                        .map(Path::to_path_buf)
                        .map_err(|error| UpdateError::Sync(error.to_string()))
                })
                .transpose()?,
        });
        prepared.push(PlannedFile {
            source,
            staged: staged_path(&live),
            live,
            aside,
        });
    }
    let mut journal = Journal {
        phase: Phase::Staging,
        files,
    };
    // Publish before staging so an abandoned attempt always has an owner.
    publish(root, &journal)?;
    let result = stage(&prepared).and_then(|()| {
        journal.phase = Phase::Prepared;
        publish(root, &journal)?;
        commit(&prepared)
    });
    if let Err(error) = result {
        return match recover_owned(root) {
            Ok(()) => Err(error),
            Err(recovery) => Err(UpdateError::Sync(format!(
                "{error}; rollback incomplete: {recovery}"
            ))),
        };
    }
    journal.phase = Phase::Committed;
    if let Err(error) = publish(root, &journal) {
        recover_owned(root)?;
        return Err(error);
    }
    Ok(())
}

/// Recover an interrupted update before removing any old or staged files.
/// A failed recovery retains its journal and every remaining rollback source.
pub fn recover_update(root: &Path) -> Result<(), UpdateError> {
    let _ownership = lock(root)?;
    recover_owned(root)
}

fn recover_owned(root: &Path) -> Result<(), UpdateError> {
    use std::io::Read as _;
    let file = match std::fs::File::open(root.join(JOURNAL)) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(io(error)),
    };
    let mut bytes = Vec::new();
    file.take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(io)?;
    if bytes.len() > 1024 * 1024 {
        return Err(UpdateError::Sync("Update journal is too large".into()));
    }
    let journal: Journal =
        serde_json::from_slice(&bytes).map_err(|error| UpdateError::Sync(error.to_string()))?;
    // Validate every path before the first mutation.
    let mut seen = std::collections::HashSet::new();
    let files = journal
        .files
        .iter()
        .map(|entry| {
            let identity = if cfg!(windows) {
                PathBuf::from(entry.live.to_string_lossy().to_ascii_lowercase())
            } else {
                entry.live.clone()
            };
            if reserved(&entry.live)
                || !seen.insert(identity)
                || entry.aside.as_ref().is_some_and(|path| {
                    !(0..super::ASIDE_ATTEMPTS)
                        .any(|attempt| super::aside(&entry.live, attempt) == *path)
                })
            {
                return Err(UpdateError::Sync(
                    "Update journal contains an invalid rollback source".into(),
                ));
            }
            Ok((
                checked(root, &entry.live)?,
                entry
                    .aside
                    .as_ref()
                    .map(|path| checked(root, path))
                    .transpose()?,
            ))
        })
        .collect::<Result<Vec<_>, UpdateError>>()?;
    for (live, aside) in files.iter().rev() {
        let staged = staged_path(live);
        if journal.phase == Phase::Committed {
            if let Some(aside) = aside {
                remove_if_present(aside)?;
            }
        } else if let Some(aside) = aside {
            if entry_exists(aside)? {
                if entry_exists(live)? {
                    std::fs::rename(live, &staged).map_err(io)?;
                }
                std::fs::rename(aside, live).map_err(io)?;
            }
        } else if journal.phase == Phase::Prepared && !entry_exists(&staged)? && entry_exists(live)?
        {
            // This originally absent file was published by the interrupted swap.
            std::fs::rename(live, &staged).map_err(io)?;
        }
        remove_if_present(&staged)?;
    }
    std::fs::remove_file(root.join(JOURNAL)).map_err(io)
}

/// A symlink is an install entry even when its target no longer exists.
fn entry_exists(path: &Path) -> Result<bool, UpdateError> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(io(error)),
    }
}

fn remove_if_present(path: &Path) -> Result<(), UpdateError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io(error)),
    }
}

#[cfg(test)]
mod tests;
