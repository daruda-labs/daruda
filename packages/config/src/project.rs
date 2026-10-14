//! Project-local config layer.
//!
//! Each opened project (the repo root the user pointed daruda at) can
//! optionally carry its own `config.toml` that overrides the `[shell]`
//! section — the rest of the schema continues to read from the user layer.
//! The on-disk shape lives at:
//!
//! ```text
//! ~/.config/daruda/projects/<basename>-<hash>/config.toml
//! ```
//!
//! The `<hash>` is a stable 64-bit FNV-1a digest of the canonicalised
//! repo root path so two daruda windows that opened the same project
//! land on the same config file. The leading `<basename>` makes the
//! directory list human-readable when a user browses
//! `~/.config/daruda/projects/` directly.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::ShellConfig;

/// Project-layer override. Every field is `Option<_>`; absent fields
/// inherit from the user layer when [`crate::Config::resolve`] applies
/// the merge.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct ProjectConfig {
    /// Shell program / lifecycle override. `None` (the default) keeps
    /// the user layer's `[shell]` section.
    pub shell: Option<ShellConfig>,
}

impl ProjectConfig {
    /// Read the project layer for `repo_root` under the profile `data_dir`.
    pub fn load_in(data_dir: &Path, repo_root: &Path) -> Self {
        Self::load_from(&project_config_path_in(data_dir, repo_root))
    }

    /// Read from an explicit path; the default (all-`None`) when the file is
    /// missing or malformed. A parse error is logged, since nothing else says
    /// why the project's overrides stopped applying.
    pub fn load_from(path: &Path) -> Self {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        toml::from_str(&text).unwrap_or_else(|error| {
            report_parse_error(path, &error);
            Self::default()
        })
    }

    /// True when this layer changes anything [`crate::Config::resolve`]
    /// produces. A file holding only the commented template overrides nothing.
    pub fn overrides_anything(&self) -> bool {
        let Self { shell } = self;
        shell.is_some()
    }
}

/// The NDJSON-log half of a malformed project layer: config load has no toast
/// to reach, and the layer silently falls back to the user's values.
fn report_parse_error(path: &Path, error: &toml::de::Error) {
    use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
    use daruda_store::observability::log_writer::LogWriter;
    use daruda_store::observability::system_info::redact_home;
    LogWriter::log(
        ErrorReport::new("Project config could not be parsed")
            .severity(ErrorSeverity::Warning)
            .message(error.to_string())
            .with_context("path", redact_home(path))
            .at(file!(), line!())
            .dedup("config.project.parse")
            .build(),
    );
}

/// Initial contents of a freshly-created
/// `<data dir>/projects/<repo>-<hash>/config.toml`. The user edits it
/// directly; the recursive config watcher re-applies it on save.
pub const PROJECT_CONFIG_TEMPLATE: &str = "\
# daruda project-local config.
#
# Sections specified here override the user-global config
# (the config.toml two folders up from this one) for this project's
# daruda windows.
# Sections you don't write keep their user-layer values.
#
# Only the [shell] section is supported.

# [shell]
# program = \"/usr/local/bin/zsh\"
# close_pane_on_exit = true
";

/// Stable 16-character identifier for `repo_root`. Two opens of the
/// same path produce the same id (modulo canonicalisation); two
/// different paths produce different ids with overwhelming probability
/// (≈2⁻³² collision chance from FNV-1a truncated to 64 bits).
pub fn project_id(repo_root: &Path) -> String {
    // The id *is* this string's hash, so anything that changes how a path
    // resolves renames every project directory on disk with it.
    let canonical = daruda_core::path::canonicalize_or_self(repo_root);
    let bytes = canonical.to_string_lossy();
    format!("{:016x}", fnv1a_64(bytes.as_bytes()))
}

/// `<profile-scoped data dir>/projects/<basename>-<hash>/` — under the same
/// profile-scoped base as [`crate::config_path`], so a debug run's project
/// layer never lands in a release profile's.
pub fn project_config_dir_in(data_dir: &Path, repo_root: &Path) -> PathBuf {
    data_dir.join("projects").join(project_dir_name(repo_root))
}

/// `<project_config_dir_in>/config.toml`.
pub fn project_config_path_in(data_dir: &Path, repo_root: &Path) -> PathBuf {
    project_config_dir_in(data_dir, repo_root).join("config.toml")
}

/// Return `<safe-basename>-<hash>` with non-alphanumeric basename
/// chars (other than `-_.`) replaced by `_`, so the directory name is
/// always filesystem-safe regardless of repo path quirks.
fn project_dir_name(repo_root: &Path) -> String {
    let id = project_id(repo_root);
    let raw = repo_root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "project".to_string());
    let safe: String = raw
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{safe}-{id}")
}

const FNV_OFFSET_64: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME_64: u64 = 0x100_0000_01b3;

fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash = FNV_OFFSET_64;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME_64);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_dir_name_uses_basename_prefix() {
        let p = PathBuf::from("/tmp/myrepo");
        let name = project_dir_name(&p);
        assert!(
            name.starts_with("myrepo-"),
            "expected myrepo-<hash>, got {name:?}",
        );
        assert_eq!(
            name.len(),
            "myrepo-".len() + 16,
            "expected 16-char hex hash"
        );
    }

    #[test]
    fn project_dir_name_sanitises_unsafe_chars() {
        // Spaces and slashes must not leak into the directory name.
        let p = PathBuf::from("/tmp/has space/sub");
        let name = project_dir_name(&p);
        assert!(
            name.starts_with("sub-"),
            "basename should be `sub`, got {name:?}"
        );
        assert!(!name.contains(' '), "name should not contain spaces");
        assert!(!name.contains('/'), "name should not contain slashes");
    }

    #[test]
    fn project_dir_name_handles_empty_basename() {
        // Root or trailing-slash paths can produce an empty basename;
        // the helper falls back to `project`.
        let p = PathBuf::from("/");
        let name = project_dir_name(&p);
        assert!(
            name.starts_with("project-"),
            "fallback should be `project`, got {name:?}",
        );
    }
}
