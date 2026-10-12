//! Provider configuration identities. Resolution never prepares an account.

pub(crate) mod codex;
pub(crate) mod library;

use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Provider {
    Claude,
    Codex,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Configuration {
    pub provider: Provider,
    pub home: PathBuf,
    pub root: PathBuf,
    pub user_file: PathBuf,
    pub personal_skills: PathBuf,
    pub cwd: Option<PathBuf>,
}

impl Configuration {
    pub fn resolve(
        provider: Provider,
        home: Option<PathBuf>,
        override_root: Option<PathBuf>,
        cwd: Option<PathBuf>,
    ) -> Option<Self> {
        let home = home.filter(|p| p.is_absolute())?;
        if cwd.as_ref().is_some_and(|p| !p.is_absolute()) {
            return None;
        }
        let explicit = override_root.is_some();
        let root = override_root.unwrap_or_else(|| {
            home.join(match provider {
                Provider::Claude => ".claude",
                Provider::Codex => ".codex",
            })
        });
        if !root.is_absolute() {
            return None;
        }
        let user_file = match provider {
            Provider::Claude if !explicit => home.join(".claude.json"),
            Provider::Claude => root.join(".claude.json"),
            Provider::Codex => root.join("config.toml"),
        };
        let personal_skills = match provider {
            Provider::Claude => root.join("skills"),
            Provider::Codex => home.join(".agents/skills"),
        };
        Some(Self {
            provider,
            home,
            root,
            user_file,
            personal_skills,
            cwd,
        })
    }

    pub fn project_file(&self, dir: &Path) -> PathBuf {
        dir.join(match self.provider {
            Provider::Claude => ".mcp.json",
            Provider::Codex => ".codex/config.toml",
        })
    }

    pub fn project_skills(&self, dir: &Path) -> PathBuf {
        dir.join(match self.provider {
            Provider::Claude => ".claude/skills",
            Provider::Codex => ".agents/skills",
        })
    }

    /// Lexical spelling is retained for writes and Local keys.
    pub fn project_dirs(&self) -> Vec<PathBuf> {
        let Some(cwd) = &self.cwd else {
            return Vec::new();
        };
        let mut dirs = Vec::new();
        for dir in cwd.ancestors() {
            dirs.push(dir.to_path_buf());
            if dir.join(".git").exists() {
                return dirs;
            }
        }
        vec![cwd.clone()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_default_claude_root_differs_from_ambient_default() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().to_path_buf();
        let default =
            Configuration::resolve(Provider::Claude, Some(home.clone()), None, None).unwrap();
        let explicit = Configuration::resolve(
            Provider::Claude,
            Some(home.clone()),
            Some(home.join(".claude")),
            None,
        )
        .unwrap();
        assert_eq!(default.user_file, home.join(".claude.json"));
        assert_eq!(explicit.user_file, home.join(".claude/.claude.json"));
        assert_ne!(default, explicit);
    }
    #[test]
    fn codex_account_does_not_relocate_personal_authoring_skills() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let account = temp.path().join("account");
        let config = Configuration::resolve(
            Provider::Codex,
            Some(home.clone()),
            Some(account.clone()),
            None,
        )
        .unwrap();
        assert_eq!(config.user_file, account.join("config.toml"));
        assert_eq!(config.personal_skills, home.join(".agents/skills"));
        assert!(Configuration::resolve(Provider::Claude, None, None, None).is_none());
        assert!(
            Configuration::resolve(Provider::Codex, Some(home), Some("relative".into()), None)
                .is_none()
        );
    }
}
