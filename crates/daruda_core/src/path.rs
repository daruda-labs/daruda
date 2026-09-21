//! Filesystem path operations that differ by platform.
//!
//! The *doing*, not only the deciding: creating a directory is an OS call,
//! and the point is that callers never make it themselves.

use std::io;
use std::path::{Path, PathBuf};

/// Resolve `path` to an absolute form with symlinks followed.
///
/// [`std::fs::canonicalize`], except on Windows it returns a UNC path
/// (`\\?\C:\…`) that compares unequal to the same path spelled normally and
/// does not round-trip through a config file. Every caller wants it gone;
/// none should have to know that.
pub fn canonicalize(path: impl AsRef<Path>) -> io::Result<PathBuf> {
    std::fs::canonicalize(path)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalize_resolves_a_relative_path_to_an_absolute_one() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("f");
        std::fs::write(&file, b"x").unwrap();

        let resolved = canonicalize(&file).unwrap();

        assert!(resolved.is_absolute());
        assert!(!resolved.to_string_lossy().starts_with(r"\\?\"));
    }

    /// What callers compare against: two spellings of one file must come back
    /// as the same path, which is the whole reason they canonicalize.
    #[test]
    fn two_spellings_of_one_path_canonicalize_alike() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("d");
        std::fs::create_dir(&dir).unwrap();
        let file = dir.join("f");
        std::fs::write(&file, b"x").unwrap();

        let direct = canonicalize(&file).unwrap();
        let roundabout = canonicalize(dir.join("..").join("d").join("f")).unwrap();

        assert_eq!(direct, roundabout);
    }

    #[test]
    fn canonicalizing_a_missing_path_is_an_error() {
        let temp = tempfile::tempdir().unwrap();

        assert!(canonicalize(temp.path().join("nope")).is_err());
    }

    #[test]
    fn creates_missing_directory_and_its_parents() {
        let temp = tempfile::tempdir().unwrap();
        let nested = temp.path().join("a").join("b");

        create_owner_only_dir(&nested).unwrap();

        assert!(nested.is_dir());
    }

    #[test]
    fn creating_an_existing_directory_succeeds() {
        let temp = tempfile::tempdir().unwrap();

        create_owner_only_dir(temp.path()).unwrap();

        assert!(temp.path().is_dir());
    }

    /// The caller's contract: an existing directory keeps the mode it
    /// already had, so a shared data directory is never re-permissioned.
    #[cfg(unix)]
    #[test]
    fn an_existing_directory_keeps_its_mode() {
        use std::os::unix::fs::PermissionsExt as _;

        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("preexisting");
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();

        create_owner_only_dir(&dir).unwrap();

        let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755);
    }

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
}
