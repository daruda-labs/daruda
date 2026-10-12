//! Which filesystem object a path names right now.
//!
//! A creation stamp cannot answer this: some filesystems keep none, and a
//! stamp says when, not what. Volume plus file index (Windows) or device plus
//! inode (Unix) changes whenever the name is handed to a different object.

use std::io;
use std::path::Path;

/// The object behind a path, comparable across lookups of the same name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileId {
    volume: u64,
    index: u64,
}

/// Identify what `path` names, following symlinks. Works for directories.
pub fn file_id(path: &Path) -> io::Result<FileId> {
    native(path)
}

#[cfg(unix)]
fn native(path: &Path) -> io::Result<FileId> {
    use std::os::unix::fs::MetadataExt as _;
    let metadata = std::fs::metadata(path)?;
    Ok(FileId {
        volume: metadata.dev(),
        index: metadata.ino(),
    })
}

#[cfg(windows)]
fn native(path: &Path) -> io::Result<FileId> {
    use std::os::windows::fs::OpenOptionsExt as _;
    use std::os::windows::io::AsRawHandle as _;
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES,
        GetFileInformationByHandle,
    };
    // A directory opens only with backup semantics; the query needs no more
    // than attribute access, and the default share mode leaves it renamable.
    let file = std::fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)?;
    // SAFETY: an all-zero struct of plain integers is a valid value.
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: `file` owns an open handle for the call's duration, and `info`
    // is a valid out-parameter.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(FileId {
        volume: info.dwVolumeSerialNumber.into(),
        index: (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
    })
}

#[cfg(not(any(unix, windows)))]
fn native(_path: &Path) -> io::Result<FileId> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "File identity unavailable",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_directory_keeps_its_identity() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(file_id(dir.path()).unwrap(), file_id(dir.path()).unwrap());
    }

    /// What the watcher relies on: a name reused for a new directory names
    /// a different object, however soon after the old one moved away.
    #[test]
    fn a_replacement_under_the_same_name_is_a_different_object() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("watched");
        std::fs::create_dir(&root).unwrap();
        let before = file_id(&root).unwrap();
        std::fs::rename(&root, dir.path().join("old")).unwrap();
        std::fs::create_dir(&root).unwrap();
        assert_ne!(file_id(&root).unwrap(), before);
        assert_eq!(file_id(&dir.path().join("old")).unwrap(), before);
    }

    #[test]
    fn a_missing_path_has_no_identity() {
        let dir = tempfile::tempdir().unwrap();
        assert!(file_id(&dir.path().join("absent")).is_err());
    }
}
