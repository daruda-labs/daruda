//! Durable publication of a file under its final name.

use std::fs::File;
use std::io;
use std::path::Path;

/// Persist `file` and its final `path` after a create or rename. The caller
/// holds a writable handle to that file and prevents concurrent path changes.
/// A failure does not establish publication, even if the path already exists.
///
/// Unix flushes the file, its containing directory, and ancestors so newly
/// created directory entries are durable too. Windows flushes the writable
/// file handle after the name change: `FlushFileBuffers` persists file-system
/// metadata, including an empty file's creation; no directory handle is used.
/// See [Microsoft's file caching contract][metadata] and [CreateFile][creation].
///
/// [metadata]: https://learn.microsoft.com/en-us/windows/win32/fileio/file-caching
/// [creation]: https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew#caching-behavior
pub fn sync_file_entry(file: &File, path: &Path) -> io::Result<()> {
    file.sync_all()?;
    sync_parent_entries(path)
}

#[cfg(unix)]
fn sync_parent_entries(path: &Path) -> io::Result<()> {
    let path = super::canonicalize(path)?;
    for directory in path.parent().into_iter().flat_map(Path::ancestors) {
        super::sync_dir(directory)?;
    }
    Ok(())
}

#[cfg(windows)]
fn sync_parent_entries(_: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn sync_parent_entries(_: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "durable file publication is unsupported on this platform",
    ))
}

#[cfg(test)]
mod tests {}
