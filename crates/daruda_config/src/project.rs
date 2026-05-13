//! Project-local config layer.
//!
//! Each opened project (the repo root the user pointed daruda at) can
//! optionally carry its own `config.toml` that overrides selected
//! sections of the user-global config. Phase 1 supports the `[shell]`
//! section only — the rest of the schema continues to read from the
//! user layer. The on-disk shape lives at:
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
//!
//! See `Projects/daruda/Tasks/Project-Local-Config-Layer-Plan.md` for
//! the full design rationale (worktree-isolation use case, why
//! out-of-tree storage, layered priority).

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
    /// Read the project layer for `repo_root`, returning the default
    /// (all-`None`) when no file exists or it fails to parse — same
    /// permissive policy as [`crate::Config::load_from`].
    pub fn load_for(repo_root: &Path) -> Self {
        let Some(path) = project_config_path(repo_root) else {
            return Self::default();
        };
        Self::load_from(&path)
    }

    /// Read from an explicit path. Useful for tests that don't go
    /// through `dirs::config_dir()`.
    pub fn load_from(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }
}

/// Stable 16-character identifier for `repo_root`. Two opens of the
/// same path produce the same id (modulo canonicalisation); two
/// different paths produce different ids with overwhelming probability
/// (≈2⁻³² collision chance from FNV-1a truncated to 64 bits).
pub fn project_id(repo_root: &Path) -> String {
    let canonical = repo_root
        .canonicalize()
        .unwrap_or_else(|_| repo_root.to_path_buf());
    let bytes = canonical.to_string_lossy();
    format!("{:016x}", fnv1a_64(bytes.as_bytes()))
}

/// `~/.config/daruda/projects/<basename>-<hash>/`. Returns `None` only
/// when both `dirs::config_dir()` is unset (rare — no `$HOME`) and the
/// system has no usable config base; in that case callers fall back
/// to the default `ProjectConfig`.
pub fn project_config_dir(repo_root: &Path) -> Option<PathBuf> {
    let base = dirs::config_dir()?.join("daruda").join("projects");
    Some(base.join(project_dir_name(repo_root)))
}

/// `<project_config_dir>/config.toml`.
pub fn project_config_path(repo_root: &Path) -> Option<PathBuf> {
    Some(project_config_dir(repo_root)?.join("config.toml"))
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
    fn project_id_is_deterministic() {
        let p = PathBuf::from("/tmp/foo");
        assert_eq!(project_id(&p), project_id(&p));
    }

    #[test]
    fn project_id_differs_for_different_paths() {
        let a = PathBuf::from("/tmp/foo");
        let b = PathBuf::from("/tmp/bar");
        assert_ne!(project_id(&a), project_id(&b));
    }

    #[test]
    fn project_id_canonicalizes_dot_segments() {
        // `/tmp/./foo` canonicalises to `/tmp/foo` when both exist;
        // the test creates the dir so canonicalize succeeds and the
        // two paths collapse to one id.
        let temp = tempfile::tempdir().unwrap();
        let direct = temp.path().to_path_buf();
        let with_dot = temp.path().join(".").to_path_buf();
        assert_eq!(project_id(&direct), project_id(&with_dot));
    }

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

    #[test]
    fn load_for_returns_default_when_path_missing() {
        let temp = tempfile::tempdir().unwrap();
        // No config.toml written — load_for must yield default.
        let cfg = ProjectConfig::load_for(temp.path());
        assert!(cfg.shell.is_none());
    }

    #[test]
    fn load_from_returns_default_when_file_unreadable() {
        let cfg = ProjectConfig::load_from(Path::new("/nonexistent/config.toml"));
        assert!(cfg.shell.is_none());
    }

    #[test]
    fn load_from_parses_shell_section() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("project.toml");
        std::fs::write(
            &path,
            "[shell]\nprogram = \"/usr/local/bin/zsh\"\nclose_pane_on_exit = false\n",
        )
        .unwrap();
        let cfg = ProjectConfig::load_from(&path);
        let shell = cfg.shell.expect("shell section parses");
        assert_eq!(shell.program.as_deref(), Some("/usr/local/bin/zsh"));
        assert!(!shell.close_pane_on_exit);
    }

    #[test]
    fn load_from_corrupt_toml_falls_back_to_default() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("project.toml");
        std::fs::write(&path, "this is = not [valid toml").unwrap();
        let cfg = ProjectConfig::load_from(&path);
        assert!(cfg.shell.is_none(), "corrupt files must not panic");
    }
}
