//! Replacing a portable install in place, where one ships as an archive.
//!
//! Windows refuses to delete or overwrite a file mapped into a running
//! process — this `.exe`, and every CRT DLL beside it — but will rename one.
//! So nothing is written over: the old file moves aside and the new one takes
//! the vacated name.

use std::path::{Path, PathBuf};

use crate::UpdateError;

/// Marks a file the swap moved out of the way. Left behind on purpose: the
/// running process still has it open, so only a later run can remove it.
pub const ASIDE_SUFFIX: &str = ".daruda-old";

/// Move `bundle`'s contents over `root`, renaming anything already there.
///
/// `bundle` is the directory an archive unpacked to; `root` is the live
/// install. Writability is proved before the first rename, so a read-only
/// install (`C:\Program Files` without elevation) fails with everything
/// untouched rather than half-swapped.
pub fn swap_into(bundle: &Path, root: &Path) -> Result<(), UpdateError> {
    prove_writable(root)?;
    for (from, to) in plan(bundle, root)? {
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(io)?;
        }
        // Rename rather than overwrite: `to` may be mapped into this very
        // process, which is the case this module exists for.
        if std::fs::symlink_metadata(&to).is_ok() {
            std::fs::rename(&to, free_aside(&to)?).map_err(io)?;
        }
        std::fs::copy(&from, &to).map_err(io)?;
    }
    Ok(())
}

/// Unpack `zip` and swap what it holds over `install_root`.
///
/// The published archive carries one top directory holding the whole install,
/// so that directory — not the archive root — is what gets swapped.
pub fn install_zip(zip: &Path, install_root: &Path) -> Result<(), UpdateError> {
    let staging = tempfile::tempdir().map_err(io)?;
    let extracted = daruda_core::process::archive_command()
        .arg("-xf")
        .arg(zip)
        .arg("-C")
        .arg(staging.path())
        .output()
        .map_err(io)?;
    if !extracted.status.success() {
        return Err(UpdateError::Sync(
            String::from_utf8_lossy(&extracted.stderr).into_owned(),
        ));
    }
    swap_into(&sole_bundle(staging.path())?, install_root)
}

/// The one directory an unpacked archive should contain.
///
/// More than one, or none, means the archive is not the package this expects
/// — better to refuse than to swap an arbitrary part of it over an install.
fn sole_bundle(staging: &Path) -> Result<PathBuf, UpdateError> {
    let mut dirs = Vec::new();
    for entry in std::fs::read_dir(staging).map_err(io)? {
        let path = entry.map_err(io)?.path();
        if path.is_dir() {
            dirs.push(path);
        }
    }
    match dirs.len() {
        1 => Ok(dirs.remove(0)),
        found => Err(UpdateError::Sync(format!(
            "expected one directory in the package, found {found}"
        ))),
    }
}

/// Every file in `bundle`, paired with where it lands under `root`.
///
/// Files only: directories are created on demand by [`swap_into`], and an
/// empty one carries nothing an install needs.
fn plan(bundle: &Path, root: &Path) -> Result<Vec<(PathBuf, PathBuf)>, UpdateError> {
    let mut pairs = Vec::new();
    let mut stack = vec![bundle.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).map_err(io)? {
            let path = entry.map_err(io)?.path();
            // `symlink_metadata`, so a link in the archive is copied as the
            // file it names rather than walked into.
            if std::fs::symlink_metadata(&path).map_err(io)?.is_dir() {
                stack.push(path);
                continue;
            }
            let relative = path
                .strip_prefix(bundle)
                .map_err(|_| UpdateError::Io(format!("{} escaped the bundle", path.display())))?;
            pairs.push((path.clone(), root.join(relative)));
        }
    }
    pairs.sort();
    Ok(pairs)
}

/// Where a file being replaced is moved to.
///
/// The suffix is appended to the whole name, never substituted for the
/// extension: `daruda.exe` must not become `daruda.daruda-old`, which Windows
/// would still happily execute.
fn aside(path: &Path, attempt: usize) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(ASIDE_SUFFIX);
    if attempt > 0 {
        name.push(format!(".{attempt}"));
    }
    PathBuf::from(name)
}

/// How many aside names to try before giving up.
const ASIDE_ATTEMPTS: usize = 64;

/// An aside name nothing occupies.
///
/// A second update before the app restarts finds the first one still there
/// *and* still locked — the running executable was moved into it — so the
/// next name is taken only once the one before it refuses to go.
fn free_aside(path: &Path) -> Result<PathBuf, UpdateError> {
    for attempt in 0..ASIDE_ATTEMPTS {
        let candidate = aside(path, attempt);
        if std::fs::symlink_metadata(&candidate).is_err()
            || std::fs::remove_file(&candidate).is_ok()
        {
            return Ok(candidate);
        }
    }
    Err(UpdateError::Io(format!(
        "{} has no free name to move aside to",
        path.display()
    )))
}

/// Remove what a previous swap left behind. Best effort by nature: a file the
/// last run had open is free by now, but one *this* run opened is not.
pub fn sweep_aside(root: &Path) {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for path in entries.flatten().map(|entry| entry.path()) {
            // Recursive because the swap is: a bundle carries `licenses/`,
            // and an aside file under it would never be collected otherwise.
            if path.is_dir() {
                stack.push(path);
            } else if path.to_string_lossy().contains(ASIDE_SUFFIX) {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
}

/// The subcommand that waits out the process it replaced. Spelled here
/// because this is where the relaunch command line is built; the app routes
/// it next to its other non-GUI entry points.
pub const AWAIT_EXIT_SUBCOMMAND: &str = "--await-exit";

/// Start the freshly installed binary once this process is gone.
///
/// Windows has no `exec` and no `open`: the new executable is spawned with
/// [`AWAIT_EXIT_SUBCOMMAND`] so it waits on this pid before starting the app
/// proper. Two daruda instances must never overlap — they contend for the
/// control socket and the flow locks — which is what the wait buys.
pub fn relaunch_from(install_root: &Path) -> Result<(), UpdateError> {
    daruda_core::process::command(install_root.join(EXECUTABLE))
        .arg(AWAIT_EXIT_SUBCOMMAND)
        .arg(std::process::id().to_string())
        .spawn()
        .map(|_| ())
        .map_err(io)
}

/// The executable inside a portable install, as `package-windows.ps1` names
/// it. Extension included: this only ever runs where one is required.
const EXECUTABLE: &str = "daruda.exe";

/// Fail before touching anything if the install cannot be written.
fn prove_writable(root: &Path) -> Result<(), UpdateError> {
    let probe = root.join(format!(".daruda-update-probe-{}", std::process::id()));
    std::fs::write(&probe, b"").map_err(|e| {
        UpdateError::Io(format!(
            "{} cannot be written ({e}) — move the install somewhere writable",
            root.display()
        ))
    })?;
    let _ = std::fs::remove_file(&probe);
    Ok(())
}

fn io(error: std::io::Error) -> UpdateError {
    UpdateError::Io(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, body).unwrap();
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap()
    }

    /// A bundle shaped like the published one: the executable, a runtime
    /// library beside it, and a licenses directory.
    fn bundle(root: &Path) -> PathBuf {
        let bundle = root.join("bundle");
        write(&bundle.join("daruda.exe"), "new exe");
        write(&bundle.join("vcruntime140.dll"), "new dll");
        write(
            &bundle.join("licenses").join("third-party.md"),
            "new notice",
        );
        bundle
    }

    #[test]
    fn every_file_lands_and_the_old_one_is_kept_aside() {
        let tmp = tempfile::tempdir().unwrap();
        let bundle = bundle(tmp.path());
        let install = tmp.path().join("install");
        write(&install.join("daruda.exe"), "old exe");
        write(&install.join("vcruntime140.dll"), "old dll");

        swap_into(&bundle, &install).unwrap();

        assert_eq!(read(&install.join("daruda.exe")), "new exe");
        assert_eq!(read(&install.join("vcruntime140.dll")), "new dll");
        assert_eq!(
            read(&install.join("licenses").join("third-party.md")),
            "new notice"
        );
        assert_eq!(
            read(&install.join(format!("daruda.exe{ASIDE_SUFFIX}"))),
            "old exe",
            "the running executable has to survive under another name"
        );
    }

    /// The point of renaming rather than writing: on Windows the destination
    /// may be mapped into this process, and only the *name* is free.
    #[test]
    fn nothing_is_written_over_in_place() {
        let tmp = tempfile::tempdir().unwrap();
        let bundle = bundle(tmp.path());
        let install = tmp.path().join("install");
        let exe = install.join("daruda.exe");
        write(&exe, "old exe");
        let before = std::fs::metadata(&exe).unwrap().len();

        swap_into(&bundle, &install).unwrap();

        // The old bytes are intact at the aside name — proof the file was
        // moved, not truncated and rewritten.
        let aside = install.join(format!("daruda.exe{ASIDE_SUFFIX}"));
        assert_eq!(std::fs::metadata(&aside).unwrap().len(), before);
        assert_eq!(read(&aside), "old exe");
    }

    #[test]
    fn a_first_install_needs_nothing_to_move_aside() {
        let tmp = tempfile::tempdir().unwrap();
        let bundle = bundle(tmp.path());
        let install = tmp.path().join("install");
        std::fs::create_dir_all(&install).unwrap();

        swap_into(&bundle, &install).unwrap();

        assert_eq!(read(&install.join("daruda.exe")), "new exe");
        assert!(!install.join(format!("daruda.exe{ASIDE_SUFFIX}")).exists());
    }

    /// A read-only install is the common Windows case (`C:\\Program Files`
    /// without elevation). It has to fail before the first rename, or the
    /// user is left with an install missing its executable.
    #[cfg(unix)]
    #[test]
    fn a_read_only_install_is_refused_with_nothing_moved() {
        use std::os::unix::fs::PermissionsExt as _;

        let tmp = tempfile::tempdir().unwrap();
        let bundle = bundle(tmp.path());
        let install = tmp.path().join("install");
        write(&install.join("daruda.exe"), "old exe");
        std::fs::set_permissions(&install, std::fs::Permissions::from_mode(0o500)).unwrap();

        let refused = swap_into(&bundle, &install);

        std::fs::set_permissions(&install, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(refused.is_err());
        assert_eq!(
            read(&install.join("daruda.exe")),
            "old exe",
            "a refused swap must leave the install exactly as it was"
        );
    }

    #[test]
    fn the_aside_name_keeps_the_extension_it_had() {
        let named = aside(Path::new("/x/daruda.exe"), 0);

        assert_eq!(
            named,
            PathBuf::from(format!("/x/daruda.exe{ASIDE_SUFFIX}")),
            "replacing the extension would leave a second runnable .exe"
        );
    }

    /// The published shape, end to end: a real archive unpacked and swapped.
    /// bsdtar reads zip on every host daruda builds for, so this exercises
    /// the Windows path from a macOS run.
    #[test]
    fn a_packaged_archive_replaces_the_install_it_is_pointed_at() {
        let tmp = tempfile::tempdir().unwrap();
        bundle(tmp.path());
        let zip = tmp.path().join("daruda-9.9.9-windows-x86_64.zip");
        let packed = daruda_core::process::command("tar")
            .arg("-a")
            .arg("-cf")
            .arg(&zip)
            .arg("-C")
            .arg(tmp.path())
            .arg("bundle")
            .status()
            .unwrap();
        assert!(packed.success(), "the fixture archive must be written");

        let install = tmp.path().join("install");
        write(&install.join("daruda.exe"), "old exe");

        install_zip(&zip, &install).unwrap();

        assert_eq!(read(&install.join("daruda.exe")), "new exe");
        assert_eq!(
            read(&install.join(format!("daruda.exe{ASIDE_SUFFIX}"))),
            "old exe"
        );
    }

    /// An archive that is not the package refuses rather than swapping some
    /// arbitrary part of itself over a working install.
    #[test]
    fn an_archive_without_a_single_bundle_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let staging = tmp.path().join("staging");
        std::fs::create_dir_all(staging.join("one")).unwrap();
        std::fs::create_dir_all(staging.join("two")).unwrap();

        assert!(sole_bundle(&staging).is_err());
        assert!(sole_bundle(&tmp.path().join("empty-missing")).is_err());
    }

    /// A second update before the app restarts: the first aside still holds
    /// the running executable, so it cannot be reused or removed.
    #[cfg(unix)]
    #[test]
    fn a_locked_leftover_gets_the_next_name_rather_than_failing() {
        use std::os::unix::fs::PermissionsExt as _;

        let tmp = tempfile::tempdir().unwrap();
        let guarded = tmp.path().join("guarded");
        let exe = guarded.join("daruda.exe");
        write(&exe, "second old");
        // Stands in for a Windows lock: the directory refuses unlink, so the
        // existing aside can be neither reused nor deleted.
        write(
            &guarded.join(format!("daruda.exe{ASIDE_SUFFIX}")),
            "first old",
        );
        std::fs::set_permissions(&guarded, std::fs::Permissions::from_mode(0o500)).unwrap();

        let taken = free_aside(&exe);

        std::fs::set_permissions(&guarded, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            taken.unwrap(),
            guarded.join(format!("daruda.exe{ASIDE_SUFFIX}.1")),
            "the occupied name must not be handed out again"
        );
    }

    /// An aside file under a subdirectory is one the swap itself created.
    #[test]
    fn a_sweep_reaches_the_nested_ones_too() {
        let tmp = tempfile::tempdir().unwrap();
        let nested = tmp.path().join("licenses");
        write(
            &nested.join(format!("third-party.md{ASIDE_SUFFIX}")),
            "stale",
        );
        write(&nested.join("third-party.md"), "live");

        sweep_aside(tmp.path());

        assert!(
            !nested
                .join(format!("third-party.md{ASIDE_SUFFIX}"))
                .exists()
        );
        assert_eq!(read(&nested.join("third-party.md")), "live");
    }

    #[test]
    fn a_sweep_removes_what_an_earlier_swap_left() {
        let tmp = tempfile::tempdir().unwrap();
        let install = tmp.path();
        write(&install.join(format!("daruda.exe{ASIDE_SUFFIX}")), "stale");
        write(&install.join("daruda.exe"), "live");

        sweep_aside(install);

        assert!(!install.join(format!("daruda.exe{ASIDE_SUFFIX}")).exists());
        assert_eq!(
            read(&install.join("daruda.exe")),
            "live",
            "the live install is untouched"
        );
    }
}
