//! Replacing a portable install in place, where one ships as an archive.
//!
//! Windows refuses to delete or overwrite a file mapped into a running
//! process — this `.exe`, and every CRT DLL beside it — but will rename one.
//! So nothing is written over: the old file moves aside and the new one takes
//! the vacated name.

use std::path::{Path, PathBuf};

use crate::UpdateError;

mod transaction;
pub use transaction::recover_update;

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
    transaction::install(bundle, root)
}

/// Marks a file copied into place but not yet live.
const STAGED_SUFFIX: &str = ".daruda-new";

/// One validated swap entry, including its journaled rollback source.
struct PlannedFile {
    source: PathBuf,
    live: PathBuf,
    staged: PathBuf,
    aside: Option<PathBuf>,
}

/// Copy every replacement before any live entry is renamed.
/// The journal owns cleanup, including a partially written staged file.
fn stage(plan: &[PlannedFile]) -> Result<(), UpdateError> {
    for file in plan {
        if let Some(parent) = file.live.parent() {
            std::fs::create_dir_all(parent).map_err(io)?;
        }
        let mut source = std::fs::File::open(&file.source).map_err(io)?;
        let mut staged = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&file.staged)
            .map_err(io)?;
        std::io::copy(&mut source, &mut staged).map_err(io)?;
        staged
            .set_permissions(source.metadata().map_err(io)?.permissions())
            .map_err(io)?;
        staged.sync_all().map_err(io)?;
    }
    Ok(())
}

/// Publish using the exact rollback sources recorded before staging.
/// The transaction recovers any failure from its Prepared journal.
fn commit(plan: &[PlannedFile]) -> Result<(), UpdateError> {
    for file in plan {
        // Rename rather than overwrite: live files may be mapped into
        // the running process, including its executable and runtime DLLs.
        if let Some(aside) = &file.aside {
            std::fs::rename(&file.live, aside).map_err(io)?;
        }
        #[cfg(test)]
        tests::remove_staged_for_failure(file)?;
        std::fs::rename(&file.staged, &file.live).map_err(io)?;
    }
    Ok(())
}

fn staged_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(STAGED_SUFFIX);
    PathBuf::from(name)
}

/// Unpack `zip` and swap what it holds over `install_root`.
///
/// The published archive carries one top directory holding the whole install,
/// so that directory — not the archive root — is what gets swapped.
pub fn install_zip(zip: &Path, install_root: &Path) -> Result<(), UpdateError> {
    install_zip_verified(zip, install_root, &|_| Ok(()))
}

/// Validate the extracted application before the first install mutation.
pub fn install_zip_verified(
    zip: &Path,
    install_root: &Path,
    verify: &impl Fn(&Path) -> Result<(), UpdateError>,
) -> Result<(), UpdateError> {
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
    let bundle = sole_bundle(staging.path())?;
    verify(&bundle)?;
    swap_into(&bundle, install_root)
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

/// Remove what a previous swap left behind — a file moved aside, or one
/// staged by an attempt that never committed. Best effort by nature: a file
/// the last run had open is free by now, but one *this* run opened is not.
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
            } else if is_swap_leftover(&path) {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
}

/// Whether `path` is a file a swap moved aside or staged. Judged by the
/// file's own name: the whole path would also match an install directory
/// that merely contains the marker, and sweep every file under it.
fn is_swap_leftover(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    if name.ends_with(STAGED_SUFFIX) {
        return true;
    }
    // `<name>.daruda-old`, or `<name>.daruda-old.<n>` — see [`aside`].
    name.rsplit_once(ASIDE_SUFFIX)
        .is_some_and(|(stem, attempt)| {
            !stem.is_empty()
                && (attempt.is_empty()
                    || attempt
                        .strip_prefix('.')
                        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())))
        })
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
mod tests;
