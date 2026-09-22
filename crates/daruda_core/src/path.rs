//! Filesystem path operations that differ by platform.
//!
//! The *doing*, not only the deciding: creating a directory is an OS call,
//! and the point is that callers never make it themselves.

use std::io;
use std::path::{Path, PathBuf};

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

    fn is_symlink(path: &Path) -> bool {
        std::fs::symlink_metadata(path)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
    }

    #[test]
    fn a_link_to_a_directory_resolves_through_it() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("d");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("f"), b"x").unwrap();
        let link = temp.path().join("link");

        symlink(&dir, &link).unwrap();

        assert!(is_symlink(&link));
        assert_eq!(std::fs::read(link.join("f")).unwrap(), b"x");
    }

    #[test]
    fn a_link_to_a_file_reads_as_the_file() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("f");
        std::fs::write(&file, b"x").unwrap();
        let link = temp.path().join("link");

        symlink(&file, &link).unwrap();

        assert!(is_symlink(&link));
        assert_eq!(std::fs::read(&link).unwrap(), b"x");
    }

    /// Callers plant links ahead of the file they point at, so a target that
    /// does not exist yet must still get its link.
    #[test]
    fn a_dangling_link_is_still_created() {
        let temp = tempfile::tempdir().unwrap();
        let link = temp.path().join("link");

        symlink(temp.path().join("never-written"), &link).unwrap();

        assert!(is_symlink(&link));
        assert!(
            !link.exists(),
            "`exists` follows the link, which goes nowhere"
        );
    }

    /// A relative target is relative to the link's directory — which is also
    /// where the platform that has to pick a kind must look.
    #[test]
    fn a_relative_target_resolves_against_the_link_s_directory() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("d");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("f"), b"x").unwrap();
        let link = temp.path().join("rel");

        symlink("d", &link).unwrap();

        assert_eq!(std::fs::read_link(&link).unwrap(), Path::new("d"));
        assert_eq!(std::fs::read(link.join("f")).unwrap(), b"x");
    }

    #[test]
    fn removing_a_directory_link_leaves_the_directory() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("d");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("f"), b"x").unwrap();
        let link = temp.path().join("link");
        symlink(&dir, &link).unwrap();

        remove_symlink(&link).unwrap();

        assert!(std::fs::symlink_metadata(&link).is_err());
        assert_eq!(std::fs::read(dir.join("f")).unwrap(), b"x");
    }

    #[test]
    fn removing_a_file_link_leaves_the_file() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("f");
        std::fs::write(&file, b"x").unwrap();
        let link = temp.path().join("link");
        symlink(&file, &link).unwrap();

        remove_symlink(&link).unwrap();

        assert!(std::fs::symlink_metadata(&link).is_err());
        assert_eq!(std::fs::read(&file).unwrap(), b"x");
    }

    #[test]
    fn a_dangling_link_is_removed_too() {
        let temp = tempfile::tempdir().unwrap();
        let link = temp.path().join("link");
        symlink(temp.path().join("never-written"), &link).unwrap();

        remove_symlink(&link).unwrap();

        assert!(std::fs::symlink_metadata(&link).is_err());
    }

    /// The guard: a real file handed to a link remover survives.
    #[test]
    fn a_real_file_is_refused_not_deleted() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("f");
        std::fs::write(&file, b"x").unwrap();

        let err = remove_symlink(&file).unwrap_err();

        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
        assert_eq!(std::fs::read(&file).unwrap(), b"x");
    }
}
