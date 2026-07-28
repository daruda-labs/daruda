//! Makes a managed Codex account's `CODEX_HOME` self-contained: shared
//! resources are symlinked in from the user's `~/.codex` and `config.toml`
//! is copied. The system home is only ever read, never modified.

use std::io;
use std::path::Path;

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;

/// Entries symlinked from the system codex home into a managed one.
/// Mirrors orca `codex-home-paths.ts` `CODEX_SYSTEM_RESOURCE_ENTRIES`.
const LINKED_SYSTEM_ENTRIES: &[&str] = &[
    "skills",
    "hooks",
    "plugins",
    "plugin-state",
    "profile-v2",
    "themes",
    "prompts",
    "AGENTS.md",
];

/// Copied rather than linked: codex rewrites this file in place, and a link
/// would write those edits back into the user's real `~/.codex`.
const CONFIG_FILE: &str = "config.toml";

/// System codex home, relative to `$HOME`. Declared as a macro so the
/// display hint below can be concatenated from the same single source.
macro_rules! system_home_dir {
    () => {
        ".codex"
    };
}

const SYSTEM_HOME_DIR: &str = system_home_dir!();

/// [`SYSTEM_HOME_DIR`] as a tilde path, for the Settings "System" choice.
pub(super) const SYSTEM_HOME_HINT: &str = concat!("~/", system_home_dir!());

/// The ambient codex home every unmanaged pane reads: `$CODEX_HOME` when the
/// user set it, else `~/.codex`. `None` only when there is no home directory
/// and no override — nothing to read from.
pub fn system_codex_home() -> Option<std::path::PathBuf> {
    system_codex_home_from(std::env::var_os("CODEX_HOME"), dirs::home_dir())
}

/// Pure core of [`system_codex_home`], split out so the override and the
/// default can be unit-tested without mutating the real process environment
/// (parallel `cargo test` runs share one).
fn system_codex_home_from(
    override_dir: Option<std::ffi::OsString>,
    home: Option<std::path::PathBuf>,
) -> Option<std::path::PathBuf> {
    if let Some(dir) = override_dir {
        return Some(std::path::PathBuf::from(dir));
    }
    Some(home?.join(SYSTEM_HOME_DIR))
}

pub fn prepare_codex_home(dest: &Path) -> io::Result<()> {
    match dirs::home_dir() {
        Some(home) => prepare_dir_from(&home.join(SYSTEM_HOME_DIR), dest),
        // Nothing to mirror from, but an empty `CODEX_HOME` still runs.
        None => std::fs::create_dir_all(dest),
    }
}

/// Hermetic seam for [`prepare_codex_home`]. Only an unusable `dest` is an
/// error: a per-entry link/copy failure is logged and skipped, since a
/// partially mirrored home still starts a session.
fn prepare_dir_from(source: &Path, dest: &Path) -> io::Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in LINKED_SYSTEM_ENTRIES {
        link_system_entry(source, dest, entry);
    }
    copy_system_config(source, dest);
    Ok(())
}

/// Link `name` from the system home. An entry missing at the source is a
/// silent skip, and an existing destination entry is left as-is so repeated
/// runs never clobber what the account already has. Caveat: that also means a
/// link is never re-pointed, so a source moved after linking stays dangling.
fn link_system_entry(source: &Path, dest: &Path, name: &str) {
    let src = source.join(name);
    let dst = dest.join(name);
    if std::fs::symlink_metadata(&src).is_err() || std::fs::symlink_metadata(&dst).is_ok() {
        return;
    }
    if let Err(e) = std::os::unix::fs::symlink(&src, &dst)
        && e.kind() != io::ErrorKind::AlreadyExists
    {
        log_mirror_failure("Failed to link a system Codex resource", name, &e);
    }
}

/// Copy `config.toml` only when the destination lacks it: codex rewrites the
/// file in place, so re-copying would discard the account's own settings.
fn copy_system_config(source: &Path, dest: &Path) {
    let src = source.join(CONFIG_FILE);
    let dst = dest.join(CONFIG_FILE);
    if !src.is_file() || std::fs::symlink_metadata(&dst).is_ok() {
        return;
    }
    if let Err(e) = std::fs::copy(&src, &dst) {
        log_mirror_failure("Failed to copy the system Codex config", CONFIG_FILE, &e);
    }
}

fn log_mirror_failure(message: &str, entry: &str, error: &io::Error) {
    LogWriter::log(
        ErrorReport::new(message)
            .severity(ErrorSeverity::Warning)
            .at(file!(), line!())
            .with_context("entry", entry)
            .with_context("error", format!("{error}"))
            .dedup("account.codex.mirror_entry_failed")
            .build(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// System-home fixture: only some of the linked entries exist, matching
    /// a real machine where most are absent.
    fn fake_system_home(dir: &Path) {
        std::fs::create_dir_all(dir.join("skills")).expect("skills");
        std::fs::write(dir.join("skills").join("a.md"), b"skill").expect("skill file");
        std::fs::create_dir_all(dir.join("plugins")).expect("plugins");
        std::fs::write(dir.join("AGENTS.md"), b"agents").expect("AGENTS.md");
        std::fs::write(dir.join(CONFIG_FILE), b"model = \"gpt\"\n").expect("config.toml");
    }

    #[test]
    fn links_present_entries_and_skips_absent_ones() {
        let source = tempfile::tempdir().expect("source");
        let dest = tempfile::tempdir().expect("dest");
        fake_system_home(source.path());

        prepare_dir_from(source.path(), dest.path()).expect("prepare");

        for present in ["skills", "plugins", "AGENTS.md"] {
            let link = dest.path().join(present);
            let meta = std::fs::symlink_metadata(&link).expect("linked entry exists");
            assert!(meta.file_type().is_symlink(), "{present} must be a symlink");
            assert_eq!(
                std::fs::read_link(&link).expect("read_link"),
                source.path().join(present)
            );
        }
        for absent in ["hooks", "plugin-state", "profile-v2", "themes", "prompts"] {
            assert!(
                std::fs::symlink_metadata(dest.path().join(absent)).is_err(),
                "{absent} is absent in the source and must be skipped"
            );
        }
    }

    #[test]
    fn copies_config_toml_verbatim_instead_of_linking() {
        let source = tempfile::tempdir().expect("source");
        let dest = tempfile::tempdir().expect("dest");
        fake_system_home(source.path());

        prepare_dir_from(source.path(), dest.path()).expect("prepare");

        let copied = dest.path().join(CONFIG_FILE);
        let meta = std::fs::symlink_metadata(&copied).expect("config.toml exists");
        assert!(!meta.file_type().is_symlink(), "config.toml must be a copy");
        assert_eq!(
            std::fs::read(&copied).expect("read copy"),
            std::fs::read(source.path().join(CONFIG_FILE)).expect("read source")
        );
    }

    #[test]
    fn leaves_the_source_home_unmodified() {
        let source = tempfile::tempdir().expect("source");
        let dest = tempfile::tempdir().expect("dest");
        fake_system_home(source.path());
        let before = snapshot(source.path());

        prepare_dir_from(source.path(), dest.path()).expect("prepare");

        assert_eq!(before, snapshot(source.path()));
    }

    #[test]
    fn is_idempotent_across_repeated_runs() {
        let source = tempfile::tempdir().expect("source");
        let dest = tempfile::tempdir().expect("dest");
        fake_system_home(source.path());

        prepare_dir_from(source.path(), dest.path()).expect("first prepare");
        let after_first = snapshot(dest.path());
        prepare_dir_from(source.path(), dest.path()).expect("second prepare");

        assert_eq!(after_first, snapshot(dest.path()));
        assert!(
            std::fs::symlink_metadata(dest.path().join("skills"))
                .expect("skills link")
                .file_type()
                .is_symlink()
        );
    }

    #[test]
    fn creates_a_missing_destination_dir() {
        let source = tempfile::tempdir().expect("source");
        let parent = tempfile::tempdir().expect("parent");
        fake_system_home(source.path());
        let dest = parent.path().join("account-uuid");

        prepare_dir_from(source.path(), &dest).expect("prepare");

        assert!(dest.is_dir());
    }

    #[test]
    fn keeps_an_account_owned_config_toml() {
        let source = tempfile::tempdir().expect("source");
        let dest = tempfile::tempdir().expect("dest");
        fake_system_home(source.path());
        std::fs::write(dest.path().join(CONFIG_FILE), b"model = \"edited\"\n").expect("write");

        prepare_dir_from(source.path(), dest.path()).expect("prepare");

        assert_eq!(
            std::fs::read(dest.path().join(CONFIG_FILE)).expect("read"),
            b"model = \"edited\"\n"
        );
    }

    /// Sorted `(relative path, kind + contents)` of a whole tree, without
    /// following symlinks — used to prove the source home is untouched.
    fn snapshot(dir: &Path) -> Vec<(String, String)> {
        let mut out = Vec::new();
        collect(dir, dir, &mut out);
        out.sort();
        out
    }

    fn collect(root: &Path, dir: &Path, out: &mut Vec<(String, String)>) {
        for entry in std::fs::read_dir(dir).expect("read_dir") {
            let entry = entry.expect("dir entry");
            let path = entry.path();
            let rel = path
                .strip_prefix(root)
                .expect("relative")
                .to_string_lossy()
                .into_owned();
            let kind = std::fs::symlink_metadata(&path)
                .expect("metadata")
                .file_type();
            if kind.is_symlink() {
                let target = std::fs::read_link(&path).expect("read_link");
                out.push((rel, format!("link:{}", target.display())));
            } else if kind.is_dir() {
                out.push((rel, "dir".to_string()));
                collect(root, &path, out);
            } else {
                let body = std::fs::read(&path).expect("read file");
                out.push((rel, format!("file:{}", String::from_utf8_lossy(&body))));
            }
        }
    }
}
