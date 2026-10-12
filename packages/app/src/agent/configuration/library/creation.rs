//! Creation destinations captured by discovery, before a form can be opened.

use super::{Configuration, PathBuf, SkillScope, resolve_write_path};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CreationTarget {
    pub scope: SkillScope,
    path: PathBuf,
    resolved: PathBuf,
}

impl CreationTarget {
    pub(super) fn capture(config: &Configuration) -> std::io::Result<Vec<Self>> {
        let mut roots = vec![(SkillScope::Personal, config.personal_skills.clone())];
        if let Some(cwd) = &config.cwd {
            roots.push((SkillScope::Project, config.project_skills(cwd)));
        }
        roots
            .into_iter()
            .map(|(scope, path)| {
                let resolved = resolve_write_path(&path)?;
                Ok(Self {
                    scope,
                    path,
                    resolved,
                })
            })
            .collect()
    }

    pub(crate) fn create(&self, name: &str, source: &str) -> std::io::Result<PathBuf> {
        if crate::agent::skills::validate_name(name).is_err()
            || resolve_write_path(&self.path)? != self.resolved
        {
            return Err(std::io::Error::other(
                "Skill destination changed; reopen before saving",
            ));
        }
        std::fs::create_dir_all(&self.resolved)?;
        let dir = self.resolved.join(name);
        // Retrying an interrupted create may find an empty directory, but
        // must never follow a pre-existing child symlink.
        match std::fs::create_dir(&dir) {
            Ok(()) => {}
            Err(error)
                if error.kind() == std::io::ErrorKind::AlreadyExists
                    && std::fs::symlink_metadata(&dir)?.is_dir() => {}
            Err(error) => return Err(error),
        }
        crate::agent::skills::persist::create_skill_source_at(&dir, source)
            .map_err(std::io::Error::other)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn creation_refuses_a_retargeted_root_and_an_existing_child_link() {
        let temp = tempfile::tempdir().unwrap();
        let a = temp.path().join("a");
        let b = temp.path().join("b");
        std::fs::create_dir(&a).unwrap();
        std::fs::create_dir(&b).unwrap();
        let link = temp.path().join("skills");
        daruda_core::path::symlink(&a, &link).unwrap();
        let target = CreationTarget {
            scope: SkillScope::Personal,
            path: link.clone(),
            resolved: daruda_core::path::canonicalize(&a).unwrap(),
        };
        std::fs::remove_file(&link).unwrap();
        daruda_core::path::symlink(&b, &link).unwrap();
        assert!(target.create("new", "draft").is_err());
        assert!(!b.join("new").exists());
        let target = CreationTarget {
            path: a.clone(),
            ..target
        };
        daruda_core::path::symlink(&b, a.join("child")).unwrap();
        assert!(target.create("child", "draft").is_err());
        assert!(!b.join("SKILL.md").exists());
    }

    #[test]
    fn creation_supports_a_missing_root() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("missing/skills");
        let target = CreationTarget {
            scope: SkillScope::Personal,
            resolved: resolve_write_path(&path).unwrap(),
            path,
        };
        let file = target.create("new", "draft").unwrap();
        assert_eq!(std::fs::read_to_string(file).unwrap(), "draft");
    }
}
