//! DMG mount, app-bundle swap, and relaunch — the safety-critical half of the
//! update flow that replaces a running `.app` bundle on disk.
//!
//! Mirrors Zed's macOS updater: mount the downloaded disk image, `rsync` its
//! `.app` over the running bundle (never `--inplace`, so the OS keeps the old
//! inode alive for the still-running process instead of SIGBUS-ing it), then
//! hand off to a detached shell that waits for this process to exit before
//! reopening the (now-swapped) bundle.
//!
//! Everything here is blocking and shells out to `hdiutil` / `rsync` /
//! `xattr` / `open`. This crate stays GPUI-free and synchronous by design.

use crate::UpdateError;
use std::path::{Path, PathBuf};
use std::process::Command;

/// RAII guard that detaches the mounted disk image when dropped, so a
/// failure partway through installation never leaves a stray mount behind.
struct MountGuard {
    mount_path: PathBuf,
}

impl Drop for MountGuard {
    fn drop(&mut self) {
        // Best-effort cleanup: detach failures here have no good recovery
        // path and would just mask whatever error is already propagating.
        let _ = Command::new("hdiutil")
            .arg("detach")
            .arg(&self.mount_path)
            .output();
    }
}

/// Mount `dmg`, copy the `.app` it contains over `running_app` in place, and
/// clear quarantine on the result. `running_app` is the path to the
/// currently-running `.app` bundle being updated.
pub fn install_dmg(dmg: &Path, running_app: &Path) -> Result<(), UpdateError> {
    let tmp_root = tempfile::tempdir().map_err(|e| UpdateError::Io(e.to_string()))?;

    let output = Command::new("hdiutil")
        .arg("attach")
        .arg("-nobrowse")
        .arg("-mountroot")
        .arg(tmp_root.path())
        .arg(dmg)
        .output()
        .map_err(|e| UpdateError::Io(e.to_string()))?;
    if !output.status.success() {
        return Err(UpdateError::Mount(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }

    // A successful `attach` means a volume is now mounted somewhere under
    // `tmp_root`. With `-mountroot <tmp_root>`, hdiutil mounts the volume as a
    // single subdirectory of `tmp_root` (named after the volume label, which
    // we don't know ahead of time) — discover it rather than hardcoding it.
    //
    // The guard MUST be installed before any other fallible step, so discovery
    // is the only work between "attach succeeded" and "guard live". If it fails
    // there is no volume path to hand a `MountGuard`, so we best-effort detach
    // whatever is mounted under the root before propagating the error, leaving
    // no stray mount behind.
    let mount_path = match find_single_dir_entry(tmp_root.path()) {
        Ok(Some(path)) => path,
        other => {
            // A successful `hdiutil attach` mounts each volume at
            // <mountroot>/<label>, not at <mountroot> itself — so detach every
            // subdirectory we can see before propagating the discovery error,
            // or the volumes leak.
            if let Ok(entries) = std::fs::read_dir(tmp_root.path()) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        let _ = Command::new("hdiutil").arg("detach").arg(&path).output();
                    }
                }
            }
            return Err(match other {
                Ok(None) => UpdateError::Mount("no volume found under mount root".to_string()),
                Err(e) => UpdateError::Mount(e),
                Ok(Some(_)) => unreachable!("Ok(Some) handled above"),
            });
        }
    };

    // Live from here on: any failure below detaches the volume via Drop.
    let _mount_guard = MountGuard {
        mount_path: mount_path.clone(),
    };

    let mounted_app = find_app_bundle(&mount_path)
        .map_err(UpdateError::Mount)?
        .ok_or_else(|| UpdateError::Mount("no .app in mounted image".to_string()))?;

    // CRITICAL: `-av --delete`, never `--inplace`. Default rsync writes each
    // updated file to a temp name and renames it over the original, so the
    // old inode (and its mapped dylibs) stays valid for the process that is
    // still running out of `running_app` until it exits. `--inplace` would
    // overwrite those files' bytes directly and SIGBUS the running process.
    // Trailing slashes on both paths: copy the *contents* of `mounted_app`
    // into `running_app`, not `mounted_app` itself as a subdirectory.
    let mut source = mounted_app.into_os_string();
    source.push("/");
    let mut dest = running_app.as_os_str().to_os_string();
    dest.push("/");

    let output = Command::new("rsync")
        .arg("-av")
        .arg("--delete")
        .arg(&source)
        .arg(&dest)
        .output()
        .map_err(|e| UpdateError::Io(e.to_string()))?;
    if !output.status.success() {
        return Err(UpdateError::Sync(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }

    // Defensive quarantine clear: non-fatal if it fails (ureq-downloaded
    // DMGs typically carry no quarantine attribute in the first place, since
    // they didn't come through a browser download).
    let _ = Command::new("xattr").arg("-cr").arg(running_app).output();

    Ok(())
}

/// Spawn a detached helper that waits for the process identified by `pid` to
/// exit, then opens `app_path`. Does not wait on the child: it must outlive
/// this process. The caller is expected to quit the app immediately after
/// this returns.
pub fn relaunch(app_path: &Path) -> Result<(), UpdateError> {
    let pid = std::process::id();
    // Poll rather than a fixed sleep: `kill -0` just checks the pid exists,
    // so the helper reopens the app as soon as this process actually exits
    // instead of racing a guessed delay.
    //
    // The pid and app path are passed as positional args (`$1`, `$2`), never
    // interpolated into the script text, so a path containing shell
    // metacharacters (`"`, `` ` ``, `$`, `\`) is passed through as an opaque
    // `OsStr` and never re-parsed by the shell.
    Command::new("/bin/sh")
        .arg("-c")
        .arg(r#"while kill -0 "$1" 2>/dev/null; do sleep 0.1; done; open "$2""#)
        .arg("sh") // $0
        .arg(pid.to_string()) // $1
        .arg(app_path) // $2 — passed as an OsStr arg, never shell-parsed
        .spawn()
        .map_err(|e| UpdateError::Io(e.to_string()))?;

    Ok(())
}

/// Read `dir`'s entries and return the single subdirectory found in it. Used to
/// discover the volume `hdiutil` mounted under a fresh, otherwise-empty mount
/// root. Returns `Ok(None)` if there are no subdirectories, and `Err` if there
/// is MORE than one — a hybrid image mounting multiple volumes must surface as
/// an error rather than have us silently pick an arbitrary one.
fn find_single_dir_entry(dir: &Path) -> Result<Option<PathBuf>, String> {
    let mut dirs = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry.path().is_dir() {
            dirs.push(entry.path());
        }
    }
    if dirs.len() > 1 {
        return Err(format!(
            "expected exactly one volume under mount root, found {}",
            dirs.len()
        ));
    }
    Ok(dirs.into_iter().next())
}

/// Find the `.app` bundle directly inside `dir` (the mounted volume root).
fn find_app_bundle(dir: &Path) -> Result<Option<PathBuf>, String> {
    for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "app") {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_single_dir_entry_finds_the_only_subdirectory() {
        let tmp = tempfile::tempdir().unwrap();
        let sub = tmp.path().join("Daruda 0.3.0");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(tmp.path().join("not_a_dir.txt"), b"x").unwrap();

        assert_eq!(find_single_dir_entry(tmp.path()).unwrap(), Some(sub));
    }

    #[test]
    fn find_single_dir_entry_returns_none_when_empty() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(find_single_dir_entry(tmp.path()).unwrap(), None);
    }

    #[test]
    fn find_single_dir_entry_errors_when_more_than_one() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("Volume A")).unwrap();
        std::fs::create_dir(tmp.path().join("Volume B")).unwrap();

        assert!(find_single_dir_entry(tmp.path()).is_err());
    }

    #[test]
    fn find_app_bundle_finds_the_dot_app_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let app = tmp.path().join("Daruda.app");
        std::fs::create_dir(&app).unwrap();
        std::fs::write(tmp.path().join("Applications"), b"x").unwrap();

        assert_eq!(find_app_bundle(tmp.path()).unwrap(), Some(app));
    }

    #[test]
    fn find_app_bundle_returns_none_when_absent() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("Applications"), b"x").unwrap();

        assert_eq!(find_app_bundle(tmp.path()).unwrap(), None);
    }
}
