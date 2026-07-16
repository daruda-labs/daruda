//! Atomic write / rename / delete for skill directories.
//!
//! Mutations hit the on-disk tree; the in-memory `SkillsState` is
//! rebuilt by the watcher afterwards. Atomicity matters because the
//! Skills tab and Claude Code read these files live — a torn write
//! would expose half-frontmatter mid-edit.

use std::io;
use std::path::{Path, PathBuf};

use tempfile::NamedTempFile;

use super::frontmatter::{SkillFrontmatter, render_skill_md};
use super::{SkillScope, scan};

/// Draft inputs collected by the CRUD modal; `write_skill` resolves
/// the target path and serialises the SKILL.md content.
#[derive(Clone, Debug)]
pub struct SkillDraft {
    pub name: String,
    pub scope: SkillScope,
    pub frontmatter: SkillFrontmatter,
    pub body: String,
}

/// Split from `io::Error` so the modal can route the duplicate-name
/// case to the validation banner instead of a generic FS error.
#[derive(Debug)]
pub enum PersistError {
    Io(io::Error),
    /// Target directory already contains a SKILL.md and the caller
    /// did not opt into `overwrite`.
    AlreadyExists(PathBuf),
    /// Rename target collides with an existing skill.
    RenameTargetExists(PathBuf),
    /// Skill scope is `Project` but the workspace has no project root.
    NoProjectRoot,
    /// Caller tried to mutate a read-only scope (currently
    /// [`SkillScope::Plugin`], owned by the plugin loader).
    ReadOnlyScope(SkillScope),
}

impl std::fmt::Display for PersistError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PersistError::Io(e) => write!(f, "{e}"),
            PersistError::AlreadyExists(p) => write!(f, "skill already exists at {}", p.display()),
            PersistError::RenameTargetExists(p) => {
                write!(f, "rename target already exists at {}", p.display())
            }
            PersistError::NoProjectRoot => write!(f, "no active project root"),
            PersistError::ReadOnlyScope(scope) => {
                write!(f, "scope `{}` is read-only", scope.slug())
            }
        }
    }
}

impl std::error::Error for PersistError {}

impl From<io::Error> for PersistError {
    fn from(e: io::Error) -> Self {
        PersistError::Io(e)
    }
}

/// Resolve the target skill directory for a given scope + name.
/// Project scope requires `project_root`; personal uses the standard
/// `~/.claude/skills` resolver. Plugin scope is read-only and
/// produces [`PersistError::ReadOnlyScope`].
pub fn resolve_skill_dir(
    scope: SkillScope,
    name: &str,
    project_root: Option<&Path>,
) -> Result<PathBuf, PersistError> {
    let root = match scope {
        SkillScope::Project => match project_root {
            Some(p) => scan::skills_project_dir(p),
            None => return Err(PersistError::NoProjectRoot),
        },
        SkillScope::Personal => scan::skills_personal_dir(),
        SkillScope::Plugin => return Err(PersistError::ReadOnlyScope(SkillScope::Plugin)),
    };
    Ok(root.join(name))
}

/// Create or overwrite the SKILL.md for a draft, written through a
/// NamedTempFile + `persist` so a crash mid-write leaves the previous
/// content intact.
///
/// `overwrite=false` returns [`PersistError::AlreadyExists`] when a
/// `SKILL.md` is present (Create flow); Edit flows pass `true`.
pub fn write_skill(
    draft: &SkillDraft,
    project_root: Option<&Path>,
    overwrite: bool,
) -> Result<PathBuf, PersistError> {
    if !draft.scope.is_writable() {
        return Err(PersistError::ReadOnlyScope(draft.scope));
    }
    let dir = resolve_skill_dir(draft.scope, &draft.name, project_root)?;
    let target = dir.join("SKILL.md");

    if !overwrite && target.exists() {
        return Err(PersistError::AlreadyExists(target));
    }
    std::fs::create_dir_all(&dir)?;

    let serialized = render_skill_md(&draft.frontmatter, &draft.body);

    // Atomic replace: temp file in the same dir, fsync, then rename.
    // macOS rename(2) is atomic, so partial reads are impossible.
    let mut tmp = NamedTempFile::new_in(&dir)?;
    use std::io::Write as _;
    tmp.as_file_mut().write_all(serialized.as_bytes())?;
    tmp.as_file().sync_all()?;
    tmp.persist(&target)
        .map_err(|e| PersistError::Io(e.error))?;

    Ok(target)
}

/// Move the skill directory to a new name within the same scope. The
/// caller validates the new name before invoking this.
///
/// Fails with [`PersistError::RenameTargetExists`] when the new
/// directory already exists, leaving the original untouched.
pub fn rename_skill(old_dir: &Path, new_name: &str) -> Result<PathBuf, PersistError> {
    let parent = old_dir
        .parent()
        .ok_or_else(|| io::Error::other("skill dir has no parent"))?;
    let new_dir = parent.join(new_name);
    if new_dir.exists() {
        return Err(PersistError::RenameTargetExists(new_dir));
    }
    std::fs::rename(old_dir, &new_dir)?;
    Ok(new_dir)
}

/// Delete the entire skill directory. The CRUD modal pairs this with
/// a confirm dialog — there is no trash / undo path.
pub fn delete_skill(dir: &Path) -> Result<(), PersistError> {
    std::fs::remove_dir_all(dir)?;
    Ok(())
}
