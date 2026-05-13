//! Blocking directory read for the file tree.
//!
//! Produces one `LoadedEntry` per direct child of `abs_path`. The
//! function never follows symlinks: `entry.file_type()` is `lstat` on
//! Unix, so a symlink to a directory reports `is_symlink = true` and
//! `is_dir = false`, surfacing as a leaf row that the user can still
//! click to open as raw text. Per-entry errors (a single unreadable
//! child) are skipped silently; only failure to open the directory
//! itself is propagated.
//!
//! Callers must run this on `cx.background_executor()`; it blocks for
//! the entire `read_dir` walk.

use std::fs;
use std::io;
use std::path::Path;

use super::tree::{EntryKind, FileTreeError, LoadedEntry};

/// OS and editor noise files that are never useful in a project file tree.
/// Excluded at load time regardless of the "show hidden files" toggle.
/// Matches Zed's `file_scan_exclusions` defaults for the same names.
const ALWAYS_EXCLUDED_NAMES: &[&str] = &[
    ".DS_Store",   // macOS extended-attribute metadata sidecar
    "Thumbs.db",   // Windows thumbnail cache
    "desktop.ini", // Windows folder display settings
    ".localized",  // macOS folder-name localization marker
];

pub fn load_dir(abs_path: &Path) -> Result<Vec<LoadedEntry>, FileTreeError> {
    let read = fs::read_dir(abs_path).map_err(map_open_error)?;

    let mut out: Vec<LoadedEntry> = Vec::with_capacity(64);
    for entry in read {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let file_type = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        let name = match entry.file_name().into_string() {
            Ok(n) => n,
            Err(_) => continue, // non-UTF-8 names — rare on macOS, skip
        };
        if ALWAYS_EXCLUDED_NAMES.contains(&name.as_str()) {
            continue;
        }
        let is_symlink = file_type.is_symlink();
        let is_dir = file_type.is_dir();
        let kind = if is_dir {
            EntryKind::UnloadedDir
        } else {
            EntryKind::File
        };
        out.push(LoadedEntry {
            name,
            kind,
            is_symlink,
        });
    }
    Ok(out)
}

fn map_open_error(e: io::Error) -> FileTreeError {
    match e.kind() {
        io::ErrorKind::NotFound => FileTreeError::NotFound,
        io::ErrorKind::PermissionDenied => FileTreeError::PermissionDenied,
        io::ErrorKind::NotADirectory => FileTreeError::NotADir,
        _ => FileTreeError::Io(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(entries: &[LoadedEntry]) -> Vec<String> {
        let mut v: Vec<String> = entries.iter().map(|e| e.name.clone()).collect();
        v.sort();
        v
    }

    #[test]
    fn load_dir_basic_files_and_dirs() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::write(root.join("file.txt"), b"hi").unwrap();
        fs::create_dir(root.join("subdir")).unwrap();

        let entries = load_dir(root).unwrap();
        let by_name: std::collections::HashMap<String, &LoadedEntry> =
            entries.iter().map(|e| (e.name.clone(), e)).collect();
        assert_eq!(entries.len(), 2);
        assert_eq!(by_name["file.txt"].kind, EntryKind::File);
        assert!(!by_name["file.txt"].is_symlink);
        assert_eq!(by_name["subdir"].kind, EntryKind::UnloadedDir);
        assert!(!by_name["subdir"].is_symlink);
    }

    #[test]
    fn load_dir_includes_dotfiles() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::write(root.join(".env"), b"").unwrap();
        fs::write(root.join("visible"), b"").unwrap();

        let n = names(&load_dir(root).unwrap());
        assert_eq!(n, vec![".env".to_string(), "visible".to_string()]);
    }

    #[test]
    fn load_dir_excludes_os_noise_files() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::write(root.join(".DS_Store"), b"").unwrap();
        fs::write(root.join("Thumbs.db"), b"").unwrap();
        fs::write(root.join("desktop.ini"), b"").unwrap();
        fs::write(root.join(".localized"), b"").unwrap();
        fs::write(root.join("visible.txt"), b"").unwrap();

        let n = names(&load_dir(root).unwrap());
        assert_eq!(n, vec!["visible.txt".to_string()]);
    }

    #[test]
    fn load_dir_nonexistent_returns_notfound() {
        let result = load_dir(Path::new("/no/such/path/_w7_load_test"));
        assert!(matches!(result, Err(FileTreeError::NotFound)));
    }

    #[test]
    fn load_dir_on_file_path_returns_notadir_or_io() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("just_a_file");
        fs::write(&file, b"x").unwrap();
        let result = load_dir(&file);
        // Modern std on macOS surfaces ErrorKind::NotADirectory.
        // Older platforms may report a generic Io error; accept either.
        assert!(matches!(
            result,
            Err(FileTreeError::NotADir) | Err(FileTreeError::Io(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn load_dir_symlink_to_dir_lstat_treated_as_symlink() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::create_dir(root.join("real_dir")).unwrap();
        symlink(root.join("real_dir"), root.join("link_to_dir")).unwrap();

        let entries = load_dir(root).unwrap();
        let link = entries
            .iter()
            .find(|e| e.name == "link_to_dir")
            .expect("symlink entry");
        assert!(link.is_symlink, "is_symlink must be true for symlink");
        // lstat: we don't follow, so the kind reflects the link itself,
        // which is_dir() returns false for. The MVP renders the row as
        // a leaf; the user can still click to read it as text.
        assert_eq!(link.kind, EntryKind::File);
    }

    #[cfg(unix)]
    #[test]
    fn load_dir_permission_denied_returns_error() {
        use std::os::unix::fs::PermissionsExt;
        // SAFETY: `libc_geteuid` only invokes `geteuid()`, a thread-safe
        // POSIX call with no preconditions and no allocation.
        // Skip when running as root — chmod 0 is bypassed there.
        let is_root = unsafe { libc_geteuid() } == 0;
        if is_root {
            return;
        }

        let temp = tempfile::tempdir().unwrap();
        let no_access = temp.path().join("no_access");
        fs::create_dir(&no_access).unwrap();
        fs::set_permissions(&no_access, fs::Permissions::from_mode(0o0)).unwrap();

        let result = load_dir(&no_access);

        // Restore permissions so tempdir can clean up.
        let _ = fs::set_permissions(&no_access, fs::Permissions::from_mode(0o755));

        assert!(matches!(result, Err(FileTreeError::PermissionDenied)));
    }

    // Tiny libc shim so the test does not pull `libc` as a dep just to
    // detect "running as root" on Unix. The `unsafe fn` wrapper inherits
    // the FFI safety obligation from `geteuid` (a no-precondition POSIX
    // call) so callers only need a matching SAFETY note at the call site.
    #[cfg(unix)]
    unsafe fn libc_geteuid() -> u32 {
        unsafe extern "C" {
            fn geteuid() -> u32;
        }
        // SAFETY: POSIX `geteuid` is documented as always succeeding,
        // is thread-safe, takes no arguments, and has no preconditions.
        unsafe { geteuid() }
    }
}
