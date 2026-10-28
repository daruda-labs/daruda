//! Filesystem path operations that differ by platform.
//!
//! The *doing*, not only the deciding: creating a directory is an OS call,
//! and the point is that callers never make it themselves.

use std::io;
use std::path::{Path, PathBuf};

pub mod directories;
mod identity;
pub mod io_error;
mod sync;

pub use identity::{FileId, file_id};
pub use sync::sync_file_entry;

/// Make a path absolute without requiring it to exist or following symlinks.
/// Resolves Windows drive-relative paths using the OS drive context as well.
pub fn absolute(path: impl AsRef<Path>) -> io::Result<PathBuf> {
    std::path::absolute(path)
}

/// Resolve `path` to an absolute form with symlinks followed.
///
/// On Windows, simplify extended paths when a regular path names the same
/// object. Keep the extended form where removing it changes the meaning
/// or prevents access, such as paths exceeding the legacy length limit.
pub fn canonicalize(path: impl AsRef<Path>) -> io::Result<PathBuf> {
    #[cfg(windows)]
    {
        dunce::canonicalize(path)
    }
    #[cfg(not(windows))]
    {
        std::fs::canonicalize(path)
    }
}

/// [`canonicalize`], falling back to the path exactly as given.
///
/// For callers resolving in order to *compare*: two spellings of one
/// directory have to land on one string, and a path that does not resolve
/// yet is still worth carrying — it may exist by the next event.
pub fn canonicalize_or_self(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Whether `a` and `b` name one place: equal as given, or once both resolve.
///
/// For paths from two producers that spell one directory two ways — a picked
/// root and a stored one, a lane path and what FSEvents or `lsof` report
/// (`/tmp` vs `/private/tmp`, a symlinked checkout, Windows letter case).
pub fn same_path(a: &Path, b: &Path) -> bool {
    a == b || matches!((canonicalize(a), canonicalize(b)), (Ok(a), Ok(b)) if a == b)
}

/// `path` relative to `root`, read the way [`same_path`] reads them.
///
/// The path need not exist — a removal event names what is already gone —
/// so the root is resolved first, and the path only if it still resolves.
pub fn strip_root(path: &Path, root: &Path) -> Option<PathBuf> {
    if let Ok(rel) = path.strip_prefix(root) {
        return Some(rel.to_path_buf());
    }
    let root = canonicalize(root).ok()?;
    if let Ok(rel) = path.strip_prefix(&root) {
        return Some(rel.to_path_buf());
    }
    // Windows simplification can remove the verbatim prefix from a short
    // root while retaining it on a long child. Compare OS-canonical forms
    // without stripping prefixes or changing special filename semantics.
    #[cfg(windows)]
    let (path, root) = (
        std::fs::canonicalize(path).ok()?,
        std::fs::canonicalize(root).ok()?,
    );
    #[cfg(not(windows))]
    let path = canonicalize(path).ok()?;
    path.strip_prefix(&root).ok().map(Path::to_path_buf)
}

/// Whether `path` is `root` or lies under it — see [`strip_root`].
pub fn is_within(path: &Path, root: &Path) -> bool {
    strip_root(path, root).is_some()
}

/// Mode for a directory only its owner may traverse.
#[cfg(unix)]
const OWNER_ONLY_DIR: u32 = 0o700;

/// Create `dir` and its parents so only the owner may traverse it. An
/// existing directory is left alone — never silently re-permissioned.
///
/// Windows has no equivalent mode: the user profile's inherited ACL
/// protects the same thing, but is not the guarantee `0o700` is.
pub fn create_owner_only_dir(dir: &Path) -> io::Result<()> {
    if dir.is_dir() {
        return Ok(());
    }
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(OWNER_ONLY_DIR);
    }
    builder.create(dir)
}

/// Mode for a file only its owner may read or write.
#[cfg(unix)]
const OWNER_ONLY_FILE: u32 = 0o600;

/// Open `path` through `options` so only its owner may read or write it. A
/// file created here never exists with a wider mode, and one that already
/// did is narrowed — a log written before this rule must not stay readable.
///
/// Windows: the user profile's inherited ACL, as for [`create_owner_only_dir`].
pub fn open_owner_only(
    options: &mut std::fs::OpenOptions,
    path: &Path,
) -> io::Result<std::fs::File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
        let file = options.mode(OWNER_ONLY_FILE).open(path)?;
        file.set_permissions(std::fs::Permissions::from_mode(OWNER_ONLY_FILE))?;
        Ok(file)
    }
    #[cfg(not(unix))]
    {
        options.open(path)
    }
}

/// Flush `dir`'s entries, so a rename or a new file inside it survives a
/// power cut.
///
/// Windows: a no-op. A directory handle needs `GENERIC_WRITE` and
/// `FILE_FLAG_BACKUP_SEMANTICS`, and Microsoft does not document that flushing
/// it persists a rename. Publish files through [`sync_file_entry`] instead.
pub fn sync_dir(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        std::fs::File::open(dir)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        Ok(())
    }
}

/// Create `link` pointing at `target`, which need not exist yet.
///
/// Windows fixes file-or-directory at creation and cannot change its mind, so
/// the kind is read from the target — resolved against the link's own
/// directory, since a relative target is relative to the link, not to us.
/// Creating one there also needs Developer Mode or elevation.
pub fn symlink(target: impl AsRef<Path>, link: impl AsRef<Path>) -> io::Result<()> {
    let (target, link) = (target.as_ref(), link.as_ref());
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
    }
    #[cfg(windows)]
    {
        let resolved = match link.parent() {
            Some(parent) if target.is_relative() => parent.join(target),
            _ => target.to_path_buf(),
        };
        if resolved.is_dir() {
            std::os::windows::fs::symlink_dir(target, link)
        } else {
            std::os::windows::fs::symlink_file(target, link)
        }
    }
}

/// Remove `link` without touching what it points at.
///
/// A link to a directory is a directory entry on Windows, and `remove_file`
/// refuses it with "access denied" — the same call that unlinks it everywhere
/// else. Anything that is not a link is refused, so a caller who guessed wrong
/// deletes nothing.
pub fn remove_symlink(link: impl AsRef<Path>) -> io::Result<()> {
    let link = link.as_ref();
    let meta = std::fs::symlink_metadata(link)?;
    if !meta.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a symbolic link",
        ));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileTypeExt as _;
        if meta.file_type().is_symlink_dir() {
            return std::fs::remove_dir(link);
        }
    }
    std::fs::remove_file(link)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn a_new_directory_is_owner_only() {
        use std::os::unix::fs::PermissionsExt as _;

        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("fresh");

        create_owner_only_dir(&dir).unwrap();

        let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, OWNER_ONLY_DIR);
    }

    #[test]
    fn syncing_a_directory_succeeds_and_a_missing_one_fails() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("entry"), b"x").unwrap();

        sync_dir(temp.path()).unwrap();
        assert!(sync_dir(&temp.path().join("missing")).is_err());
    }
}
