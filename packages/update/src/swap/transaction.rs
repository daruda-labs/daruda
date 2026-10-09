//! Durable phase tracking prevents interrupted file swaps losing their rollback sources.

use super::{ASIDE_SUFFIX, STAGED_SUFFIX, commit, free_aside, io, plan, stage, staged_path};
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
    use fs4::fs_std::FileExt as _;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(root.join(".daruda-update.lock"))
        .map_err(io)?;
    file.try_lock_exclusive().map_err(io)?;
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
    let plan = plan(bundle, root)?;
    let mut files = Vec::new();
    for (_, live) in &plan {
        let relative = live
            .strip_prefix(root)
            .map_err(|error| UpdateError::Sync(error.to_string()))?;
        checked(root, relative)?;
        if reserved(relative) {
            return Err(UpdateError::Sync(
                "Package contains reserved update paths".into(),
            ));
        }
        let aside = if live.try_exists().map_err(io)? {
            Some(
                free_aside(live)?
                    .strip_prefix(root)
                    .map_err(|error| UpdateError::Sync(error.to_string()))?
                    .to_path_buf(),
            )
        } else {
            None
        };
        files.push(Entry {
            live: relative.to_path_buf(),
            aside,
        });
    }
    let mut journal = Journal {
        phase: Phase::Staging,
        files,
    };
    // Publish before staging so an abandoned attempt always has an owner.
    publish(root, &journal)?;
    let result = stage(&plan).and_then(|staged| {
        journal.phase = Phase::Prepared;
        publish(root, &journal)?;
        commit(staged)
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
            if aside.try_exists().map_err(io)? {
                if live.try_exists().map_err(io)? {
                    std::fs::rename(live, &staged).map_err(io)?;
                }
                std::fs::rename(aside, live).map_err(io)?;
            }
        } else if journal.phase == Phase::Prepared
            && !staged.try_exists().map_err(io)?
            && live.try_exists().map_err(io)?
        {
            // This originally absent file was published by the interrupted swap.
            std::fs::rename(live, &staged).map_err(io)?;
        }
        remove_if_present(&staged)?;
    }
    std::fs::remove_file(root.join(JOURNAL)).map_err(io)
}

fn remove_if_present(path: &Path) -> Result<(), UpdateError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io(error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_journals_keep_their_recovery_phase() {
        for (flags, expected) in [
            (serde_json::json!({"committed": false}), Phase::Staging),
            (serde_json::json!({"committed": true}), Phase::Committed),
            (
                serde_json::json!({"committed": false, "prepared": false}),
                Phase::Staging,
            ),
            (
                serde_json::json!({"committed": false, "prepared": true}),
                Phase::Prepared,
            ),
            (
                serde_json::json!({"committed": true, "prepared": false}),
                Phase::Committed,
            ),
            (
                serde_json::json!({"committed": true, "prepared": true}),
                Phase::Committed,
            ),
        ] {
            let mut value = flags;
            value["files"] = serde_json::json!([]);
            let journal: Journal = serde_json::from_value(value).expect("legacy journal");
            assert_eq!(journal.phase, expected);
        }
    }

    #[test]
    fn new_journals_remain_readable_by_the_legacy_format() {
        for phase in [Phase::Staging, Phase::Prepared, Phase::Committed] {
            let encoded = serde_json::to_value(Journal {
                phase,
                files: vec![],
            })
            .expect("serialize journal");
            assert!(encoded.get("phase").is_none());
            assert_eq!(encoded["committed"], phase == Phase::Committed);
            assert_eq!(encoded["prepared"], phase != Phase::Staging);
            let decoded: Journal = serde_json::from_value(encoded).expect("round trip");
            assert_eq!(decoded.phase, phase);
        }
    }

    #[test]
    fn install_refuses_symlinked_parents_before_staging() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("install");
        let bundle = dir.path().join("bundle");
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(bundle.join("licenses")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("notice.txt"), "preserve").unwrap();
        std::fs::write(bundle.join("licenses/notice.txt"), "replacement").unwrap();
        let link = root.join("licenses");
        daruda_core::path::symlink(&outside, &link).unwrap();
        let result = install(&bundle, &root);
        daruda_core::path::remove_symlink(&link).unwrap();
        assert!(result.is_err());
        assert_eq!(
            std::fs::read_to_string(outside.join("notice.txt")).unwrap(),
            "preserve"
        );
        assert!(!outside.join("notice.txt.daruda-new").exists());
        assert!(!root.join(JOURNAL).exists());
    }

    #[test]
    fn staging_does_not_follow_an_existing_destination_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("install");
        let bundle = dir.path().join("bundle");
        let outside = dir.path().join("outside.txt");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&bundle).unwrap();
        std::fs::write(&outside, "preserve").unwrap();
        std::fs::write(bundle.join("app.exe"), "replacement").unwrap();
        let link = root.join("app.exe.daruda-new");
        daruda_core::path::symlink(&outside, &link).unwrap();
        let result = install(&bundle, &root);
        assert!(result.is_err());
        assert_eq!(std::fs::read_to_string(&outside).unwrap(), "preserve");
        assert!(!root.join("app.exe").exists());
    }

    #[cfg(windows)]
    #[test]
    fn windows_reserved_paths_are_case_insensitive() {
        let root = tempfile::tempdir().unwrap();
        let bundle = tempfile::tempdir().unwrap();
        std::fs::write(bundle.path().join(".DARUDA-UPDATE-JOURNAL.JSON"), "package").unwrap();
        assert!(install(bundle.path(), root.path()).is_err());
        assert!(!root.path().join(JOURNAL).exists());
        std::fs::write(root.path().join("app.exe"), "preserve").unwrap();
        publish(
            root.path(),
            &Journal {
                phase: Phase::Prepared,
                files: vec![
                    Entry {
                        live: "app.exe".into(),
                        aside: None,
                    },
                    Entry {
                        live: "APP.EXE".into(),
                        aside: None,
                    },
                ],
            },
        )
        .unwrap();
        assert!(recover_update(root.path()).is_err());
        assert_eq!(
            std::fs::read_to_string(root.path().join("app.exe")).unwrap(),
            "preserve"
        );
        assert!(root.path().join(JOURNAL).exists());
    }

    #[test]
    fn interrupted_mixed_install_restores_every_old_file() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("app.exe"), "new").unwrap();
        std::fs::write(root.path().join("app.exe.daruda-old"), "old").unwrap();
        std::fs::write(root.path().join("runtime.dll.daruda-new"), "new runtime").unwrap();
        std::fs::write(root.path().join("runtime.dll"), "old runtime").unwrap();
        publish(
            root.path(),
            &Journal {
                phase: Phase::Prepared,
                files: vec![
                    Entry {
                        live: "app.exe".into(),
                        aside: Some("app.exe.daruda-old".into()),
                    },
                    Entry {
                        live: "runtime.dll".into(),
                        aside: Some("runtime.dll.daruda-old".into()),
                    },
                ],
            },
        )
        .unwrap();
        recover_update(root.path()).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.path().join("app.exe")).unwrap(),
            "old"
        );
        assert_eq!(
            std::fs::read_to_string(root.path().join("runtime.dll")).unwrap(),
            "old runtime"
        );
        assert!(!root.path().join(JOURNAL).exists());
    }

    #[test]
    fn journal_cannot_modify_files_outside_the_install() {
        let root = tempfile::tempdir().unwrap();
        publish(
            root.path(),
            &Journal {
                phase: Phase::Prepared,
                files: vec![Entry {
                    live: "../escape".into(),
                    aside: None,
                }],
            },
        )
        .unwrap();
        assert!(recover_update(root.path()).is_err());
        assert!(root.path().join(JOURNAL).exists());
    }
}
