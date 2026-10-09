//! Retain the last readable generation and preserve damaged state before recovery.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Serialize, de::DeserializeOwned};

use super::write_json_atomic;
use crate::observability::error_report::{ErrorReport, ErrorSeverity};
use crate::observability::log_writer::LogWriter;
use crate::observability::system_info::redact_home;

// Bound allocation before parsing even if a damaged file advertises no useful schema.
pub(super) const MAX_RECORD_BYTES: usize = 64 * 1024 * 1024;

fn read_record(path: &Path) -> io::Result<Vec<u8>> {
    use io::Read as _;
    let file = std::fs::File::open(path)?;
    if file.metadata()?.len() > MAX_RECORD_BYTES as u64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "State record is too large",
        ));
    }
    let mut bytes = Vec::new();
    file.take((MAX_RECORD_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "State record is too large",
        ));
    }
    Ok(bytes)
}

fn backup_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".bak");
    PathBuf::from(name)
}

/// Remove the recovery generation first, so an intentional deletion cannot revive.
pub fn delete_recoverable_json(path: &Path) -> io::Result<()> {
    for target in [backup_path(path), path.to_owned()] {
        match std::fs::remove_file(target) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn parse<T: DeserializeOwned>(bytes: &[u8]) -> io::Result<T> {
    serde_json::from_slice(bytes).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

/// Persist a record while retaining its previous readable generation.
/// An unreadable existing record is never replaced by a fresh default.
pub fn save_recoverable_json<T: Serialize + DeserializeOwned>(
    dir: &Path,
    target: &Path,
    value: &T,
) -> io::Result<()> {
    match read_record(target) {
        Ok(bytes) => {
            let _: T = parse(&bytes)?;
            let previous: serde_json::Value = parse(&bytes)?;
            let next = serde_json::to_value(value).map_err(io::Error::other)?;
            refuse_schema_downgrade(&previous, &next)?;
            if bytes == serde_json::to_vec_pretty(value).map_err(io::Error::other)? {
                return Ok(());
            }
            write_json_atomic(dir, &backup_path(target), &previous)?;
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    write_json_atomic(dir, target, value)
}

fn refuse_schema_downgrade(
    previous: &serde_json::Value,
    next: &serde_json::Value,
) -> io::Result<()> {
    if previous
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        .zip(
            next.get("schema_version")
                .and_then(serde_json::Value::as_u64),
        )
        .is_some_and(|(old, new)| old > new)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "State belongs to a newer schema and cannot be overwritten",
        ));
    }
    Ok(())
}

pub(super) fn save_json_with_backup<T: Serialize>(
    dir: &Path,
    target: &Path,
    value: &T,
) -> io::Result<()> {
    let next = serde_json::to_value(value).map_err(io::Error::other)?;
    save_recoverable_json(dir, target, &next)
}

/// Recover a missing or damaged record from its last readable generation.
/// A missing record with no backup is a normal absence; corruption is an error.
pub fn load_recoverable_json<T: DeserializeOwned>(
    subsystem: &str,
    path: &Path,
) -> io::Result<Option<T>> {
    let original = match read_record(path) {
        Ok(bytes) => match parse::<T>(&bytes) {
            Ok(value) => return Ok(Some(value)),
            Err(_) => Some(bytes),
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let backup = match read_record(&backup_path(path)) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound && original.is_none() => {
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    let recovered: T = parse(&backup)?;
    let dir = path
        .parent()
        .ok_or_else(|| io::Error::other("State has no parent directory"))?;
    let quarantine = if let Some(bytes) = original {
        let mut file = tempfile::Builder::new()
            .prefix("corrupt-state-")
            .tempfile_in(dir)?;
        use std::io::Write as _;
        file.write_all(&bytes)?;
        file.as_file().sync_all()?;
        Some(file.keep().map_err(|error| error.error)?.1)
    } else {
        None
    };
    // Keep fields not requested by a version probe or by an older reader.
    let raw: serde_json::Value = parse(&backup)?;
    write_json_atomic(dir, path, &raw)?;
    let mut report = ErrorReport::new(format!("Recovered {subsystem} state from backup"))
        .severity(ErrorSeverity::Warning)
        .at(file!(), line!())
        .with_context("path", redact_home(path))
        .dedup(format!("store.{subsystem}.recovered"));
    if let Some(quarantine) = quarantine {
        report = report.with_context("preserved", redact_home(&quarantine));
    }
    LogWriter::log(report.build());
    Ok(Some(recovered))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq, Serialize, serde::Deserialize)]
    struct State {
        value: u32,
    }

    #[test]
    fn oversized_records_are_refused_without_replacing_the_original() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let file = std::fs::File::create(&path).unwrap();
        file.set_len((MAX_RECORD_BYTES + 1) as u64).unwrap();
        assert_eq!(
            load_recoverable_json::<State>("test", &path)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
        assert!(save_recoverable_json(dir.path(), &path, &State { value: 1 }).is_err());
        assert_eq!(
            std::fs::metadata(path).unwrap().len(),
            (MAX_RECORD_BYTES + 1) as u64
        );
    }

    #[test]
    fn schema_probes_recover_without_discarding_unknown_fields() {
        #[derive(serde::Deserialize)]
        struct Probe {
            schema_version: u32,
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let original =
            serde_json::json!({"schema_version": 2, "unknown": ["preserve", "all", "rows"]});
        super::super::save_json_atomic(dir.path(), &path, &original).unwrap();
        super::super::save_json_atomic(
            dir.path(),
            &path,
            &serde_json::json!({"schema_version": 2, "unknown": []}),
        )
        .unwrap();
        std::fs::write(&path, "corrupt").unwrap();
        assert_eq!(
            load_recoverable_json::<Probe>("probe", &path)
                .unwrap()
                .unwrap()
                .schema_version,
            2
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&std::fs::read(&path).unwrap()).unwrap(),
            original
        );
        assert!(
            super::super::save_json_atomic(
                dir.path(),
                &path,
                &serde_json::json!({"schema_version": 1})
            )
            .is_err()
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&std::fs::read(&path).unwrap()).unwrap(),
            original
        );
    }

    #[test]
    fn damaged_source_is_preserved_and_last_good_generation_is_restored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        save_recoverable_json(dir.path(), &path, &State { value: 1 }).unwrap();
        save_recoverable_json(dir.path(), &path, &State { value: 2 }).unwrap();
        std::fs::write(&path, b"damaged").unwrap();
        assert_eq!(
            load_recoverable_json::<State>("test", &path).unwrap(),
            Some(State { value: 1 })
        );
        let preserved = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .find(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("corrupt-state-")
            })
            .unwrap();
        assert_eq!(std::fs::read(preserved.path()).unwrap(), b"damaged");
    }

    #[test]
    fn corrupt_records_cannot_be_overwritten_or_mistaken_for_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        assert!(
            load_recoverable_json::<State>("test", &path)
                .unwrap()
                .is_none()
        );
        std::fs::write(&path, b"damaged").unwrap();
        assert!(load_recoverable_json::<State>("test", &path).is_err());
        assert!(save_recoverable_json(dir.path(), &path, &State { value: 1 }).is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"damaged");
    }

    #[test]
    fn intentional_deletion_does_not_restore_a_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        save_recoverable_json(dir.path(), &path, &State { value: 1 }).unwrap();
        save_recoverable_json(dir.path(), &path, &State { value: 2 }).unwrap();
        delete_recoverable_json(&path).unwrap();
        assert!(
            load_recoverable_json::<State>("test", &path)
                .unwrap()
                .is_none()
        );
    }
}
