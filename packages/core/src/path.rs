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
    canonicalize(path)
        .ok()?
        .strip_prefix(&root)
        .ok()
        .map(Path::to_path_buf)
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

    #[cfg(unix)]
    fn mode_of(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[cfg(unix)]
    #[test]
    fn an_owner_only_file_is_created_narrow_and_an_existing_one_is_narrowed() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let fresh = dir.path().join("fresh.log");
        open_owner_only(
            std::fs::OpenOptions::new().create(true).append(true),
            &fresh,
        )
        .unwrap();
        assert_eq!(mode_of(&fresh), 0o600);

        let old = dir.path().join("old.log");
        std::fs::write(&old, b"before").unwrap();
        std::fs::set_permissions(&old, std::fs::Permissions::from_mode(0o644)).unwrap();
        open_owner_only(std::fs::OpenOptions::new().append(true), &old).unwrap();
        assert_eq!(mode_of(&old), 0o600);
        assert_eq!(
            std::fs::read(&old).unwrap(),
            b"before",
            "appending keeps the bytes"
        );
    }

    #[test]
    fn a_path_is_the_same_as_itself_even_when_it_does_not_exist() {
        let gone = Path::new("/daruda/no/such/dir");
        assert!(same_path(gone, gone));
        assert!(!same_path(gone, Path::new("/daruda/no/other")));
    }

    /// The shape every caller has: one side through a symlink, the other
    /// resolved — FSEvents reporting `/private/tmp` for a root typed `/tmp`.
    #[cfg(unix)]
    #[test]
    fn a_symlinked_spelling_names_the_same_place() {
        let temp = tempfile::tempdir().unwrap();
        let real = temp.path().join("real");
        std::fs::create_dir(&real).unwrap();
        let link = temp.path().join("link");
        symlink(&real, &link).unwrap();
        let real = canonicalize(&real).unwrap();

        assert!(same_path(&link, &real));
        std::fs::write(real.join("f"), b"x").unwrap();
        assert_eq!(strip_root(&real.join("f"), &link), Some(PathBuf::from("f")));
        assert_eq!(strip_root(&link.join("f"), &real), Some(PathBuf::from("f")));
        // Removed already: only the root can still be resolved.
        assert_eq!(
            strip_root(&real.join("gone"), &link),
            Some(PathBuf::from("gone"))
        );
        assert!(is_within(&real, &link));
        assert!(!is_within(temp.path(), &link));
    }

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
