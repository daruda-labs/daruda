//! Skills data model — GPUI-free types describing the on-disk layout
//! of Claude Code Skills (`<root>/.claude/skills/<name>/SKILL.md`).
//!
//! Spec: <https://code.claude.com/docs/en/skills>. v2 of daruda's
//! Skills tab tracks the directory model (one folder per skill,
//! optional auxiliary files) rather than the legacy single-file
//! commands layout. Frontmatter is parsed losslessly: known keys
//! become typed fields, unknown keys are preserved in `extra` and
//! round-tripped on save.
//!
//! No GPUI imports here — `app/src/CLAUDE.md` G2 / G7 forbid them.
//! This module is consumed by the renderer (`workspace/right_panel/`),
//! the watcher (`hooks/skills_watcher.rs`), and the CRUD modals.

pub mod frontmatter;
pub mod global;
pub mod persist;
pub mod plugin_ops;
pub mod plugins;
pub mod scan;

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub use frontmatter::{
    SkillFrontmatter, parse_frontmatter, serialize_frontmatter, split_frontmatter,
};
pub use persist::{PersistError, SkillDraft, delete_skill, rename_skill, write_skill};
pub use scan::{SkillsRoots, body_preview, scan_scope, skills_personal_dir, skills_project_dir};

/// On-disk scope for a skill.
///
/// - `Project` — `<lane>/.claude/skills/<name>/SKILL.md`
/// - `Personal` — `~/.claude/skills/<name>/SKILL.md`
/// - `Plugin` — `<plugin-install-path>/skills/<name>/SKILL.md`,
///   typically under `~/.claude/plugins/cache/<marketplace>/<plugin>/<version>/`.
///   Read-only from daruda's perspective: marketplace updates own the
///   files, so Edit / Delete / Rename are disabled. Skills here are
///   namespaced (`/<plugin>:<skill>`) per spec.
///
/// The Enterprise scope is a fourth official tier but lives behind
/// managed settings; daruda doesn't surface it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SkillScope {
    Project,
    Personal,
    Plugin,
}

impl SkillScope {
    pub const ALL: [SkillScope; 3] = [
        SkillScope::Project,
        SkillScope::Personal,
        SkillScope::Plugin,
    ];

    /// Scopes the user can write to. Plugin is read-only.
    pub const WRITABLE: [SkillScope; 2] = [SkillScope::Project, SkillScope::Personal];

    /// Stable kebab slug used by config / serialization. Mirrors the
    /// spec's scope naming (`project`, `personal`, `plugin`).
    pub fn slug(self) -> &'static str {
        match self {
            SkillScope::Project => "project",
            SkillScope::Personal => "personal",
            SkillScope::Plugin => "plugin",
        }
    }

    /// True for scopes daruda can mutate. Edit / Delete / Rename
    /// modal entry points consult this before opening so the user
    /// can't accidentally try to edit a marketplace-managed file.
    pub fn is_writable(self) -> bool {
        matches!(self, SkillScope::Project | SkillScope::Personal)
    }
}

/// 4-state derived from `user-invocable` × `disable-model-invocation`.
/// Drives the badge colour in the panel row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkillInvocation {
    /// Default: user-invocable=true, disable-model-invocation=false.
    Both,
    /// user-invocable=true, disable-model-invocation=true.
    UserOnly,
    /// user-invocable=false, disable-model-invocation=false.
    ModelOnly,
    /// user-invocable=false, disable-model-invocation=true.
    Disabled,
}

impl SkillInvocation {
    /// Spec defaults: both flags off → `Both`.
    pub fn from_flags(user_invocable: bool, disable_model_invocation: bool) -> Self {
        match (user_invocable, disable_model_invocation) {
            (true, false) => Self::Both,
            (true, true) => Self::UserOnly,
            (false, false) => Self::ModelOnly,
            (false, true) => Self::Disabled,
        }
    }
}

/// One skill on disk. Built by [`scan::scan_scope`] and
/// [`scan::scan_plugins`].
#[derive(Clone, Debug)]
pub struct Skill {
    /// Display name. Frontmatter `name` if present, else the directory
    /// stem. The scanner guarantees this is non-empty.
    pub name: String,
    /// Absolute path to the skill directory (the parent of `SKILL.md`).
    pub dir: PathBuf,
    pub scope: SkillScope,
    pub frontmatter: SkillFrontmatter,
    /// First markdown paragraph (after frontmatter) trimmed to
    /// [`scan::PREVIEW_MAX_CHARS`] chars. Empty when no body present.
    pub body_preview: String,
    /// Files inside `dir` other than `SKILL.md` (recursive). Surfaces
    /// the `📎 N` chip on the row.
    pub aux_file_count: u32,
    /// `mtime` of `SKILL.md`. Used as a cheap "last edited" timestamp
    /// for sort order if needed downstream.
    pub modified_at: SystemTime,
    /// Owning plugin's namespace (e.g. `swift-lsp@claude-plugins-official`).
    /// Always `Some` for `SkillScope::Plugin`, always `None` otherwise.
    /// Used by the renderer to display the spec-mandated
    /// `<plugin>:<skill>` namespacing on plugin rows.
    pub plugin_id: Option<String>,
    /// Whether this plugin entry came from `installed_plugins.json`
    /// (active) or from a registered marketplace clone (catalog
    /// browse). Always `None` for non-Plugin scopes.
    pub plugin_availability: Option<plugins::PluginAvailability>,
}

impl Skill {
    /// Path to the skill's primary file.
    pub fn skill_md_path(&self) -> PathBuf {
        self.dir.join("SKILL.md")
    }

    /// 4-state from frontmatter flags.
    pub fn invocation(&self) -> SkillInvocation {
        SkillInvocation::from_flags(
            self.frontmatter.user_invocable,
            self.frontmatter.disable_model_invocation,
        )
    }
}

/// App-wide skills state. Lives as a GPUI Global registered at
/// bootstrap (`global::init`). User-scope vectors (`personal`,
/// `plugin`) are shared across every Workspace; project-scope skills
/// are partitioned by lane root path so multiple Workspace windows
/// observing different lanes never collide on a single
/// `project_root` field. Renderers read a per-lane
/// [`SkillsSnapshot`] via [`SkillsState::snapshot_for`].
///
/// Mirrors Zed's `SettingsStore::local_settings` pattern (a single
/// Global with a `BTreeMap` keyed by lane-relative location).
#[derive(Clone, Debug, Default)]
pub struct SkillsState {
    pub personal: Vec<Skill>,
    pub plugin: Vec<Skill>,
    /// Per-lane project-scope skills, keyed by the lane's
    /// absolute root path (what `Workspace::active_worktree_root`
    /// returns). An entry exists for every lane that has been
    /// scanned at least once; opening a different lane adds a new
    /// key without disturbing the others.
    pub project: BTreeMap<PathBuf, Vec<Skill>>,
    /// Last successful scan timestamp across any scope. `None` until
    /// first load lands.
    pub last_scanned: Option<SystemTime>,
}

impl SkillsState {
    /// Reload one scope from disk. `lane` is required for
    /// `SkillScope::Project` and ignored otherwise. Project entries
    /// are inserted into the `project` map at the lane's path.
    pub fn reload_scope(&mut self, scope: SkillScope, lane: Option<&Path>, personal_root: &Path) {
        match scope {
            SkillScope::Project => {
                if let Some(root) = lane {
                    let v = scan_scope(&skills_project_dir(root), SkillScope::Project);
                    self.project.insert(root.to_path_buf(), v);
                }
            }
            SkillScope::Personal => {
                self.personal = scan_scope(personal_root, SkillScope::Personal);
            }
            SkillScope::Plugin => {
                self.plugin = scan::scan_plugins();
            }
        }
        self.last_scanned = Some(SystemTime::now());
    }

    /// Drop a lane's project entry. Call when a lane is
    /// closed so the `BTreeMap` doesn't grow unbounded across the
    /// session.
    pub fn forget_lane(&mut self, root: &Path) {
        self.project.remove(root);
    }

    /// Build an owned per-lane view for the renderer / modals.
    /// Carrying it by value keeps the panel render closure off the
    /// Global (no re-entrancy hazard).
    pub fn snapshot_for(&self, root: Option<&Path>) -> SkillsSnapshot {
        let project = root
            .and_then(|r| self.project.get(r))
            .cloned()
            .unwrap_or_default();
        SkillsSnapshot {
            project,
            personal: self.personal.clone(),
            plugin: self.plugin.clone(),
            project_root: root.map(Path::to_path_buf),
            last_scanned: self.last_scanned,
        }
    }

    /// Duplicate-name check used by Create / Rename modals. Plugin
    /// scope is read-only and namespaced (`<plugin>:<skill>`); a bare
    /// duplicate query against it is meaningless, so `false`.
    pub fn name_exists(&self, scope: SkillScope, name: &str, lane: Option<&Path>) -> bool {
        let list: &[Skill] = match scope {
            SkillScope::Project => match lane.and_then(|r| self.project.get(r)) {
                Some(v) => v.as_slice(),
                None => return false,
            },
            SkillScope::Personal => self.personal.as_slice(),
            SkillScope::Plugin => return false,
        };
        let needle = name.to_ascii_lowercase();
        list.iter().any(|s| s.name.to_ascii_lowercase() == needle)
    }
}

/// Owned per-lane projection of [`SkillsState`] consumed by the
/// renderer and CRUD modals. Carries the project Vec for *one*
/// lane along with the user-global personal + plugin vectors.
#[derive(Clone, Debug, Default)]
pub struct SkillsSnapshot {
    pub project: Vec<Skill>,
    pub personal: Vec<Skill>,
    pub plugin: Vec<Skill>,
    /// Lane root whose project skills are carried in `project`.
    /// `None` when the workspace has no active lane.
    pub project_root: Option<PathBuf>,
    pub last_scanned: Option<SystemTime>,
}

impl SkillsSnapshot {
    /// True when `personal` contains a skill with the same name as
    /// `project_name`. Surfaces the `(overrides personal)` chip on
    /// project rows.
    pub fn project_overrides_personal(&self, project_name: &str) -> bool {
        let needle = project_name.to_ascii_lowercase();
        self.personal
            .iter()
            .any(|s| s.name.to_ascii_lowercase() == needle)
    }

    /// Duplicate-name check against the captured Project / Personal
    /// vectors. Plugin scope is read-only and namespaced
    /// (`<plugin>:<skill>`); a bare duplicate query is meaningless.
    pub fn name_exists(&self, scope: SkillScope, name: &str) -> bool {
        let list: &[Skill] = match scope {
            SkillScope::Project => self.project.as_slice(),
            SkillScope::Personal => self.personal.as_slice(),
            SkillScope::Plugin => return false,
        };
        let needle = name.to_ascii_lowercase();
        list.iter().any(|s| s.name.to_ascii_lowercase() == needle)
    }
}

/// Errors surfaced when validating a user-supplied skill name. The
/// modal renders them as inline banners; the regex matches the
/// official spec recommendation (≤ 64 chars, lowercase / digits /
/// `-` / `_`, must start with alphanumeric).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NameError {
    Empty,
    TooLong { max: usize, got: usize },
    InvalidChar { ch: char, position: usize },
    InvalidLeading { ch: char },
    DuplicateInScope { scope: SkillScope },
}

/// Maximum length per spec recommendation.
pub const MAX_NAME_LEN: usize = 64;

/// Validate a skill name against the spec's directory-name rules.
/// Duplicate detection (`DuplicateInScope`) is a separate check the
/// caller layers on top using [`SkillsState::name_exists`] — kept out
/// of this fn so it stays purely syntactic.
pub fn validate_name(name: &str) -> Result<(), NameError> {
    if name.is_empty() {
        return Err(NameError::Empty);
    }
    if name.len() > MAX_NAME_LEN {
        return Err(NameError::TooLong {
            max: MAX_NAME_LEN,
            got: name.len(),
        });
    }
    for (i, ch) in name.chars().enumerate() {
        let ok = ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-' || ch == '_';
        if !ok {
            return Err(NameError::InvalidChar { ch, position: i });
        }
        if i == 0 && (ch == '-' || ch == '_') {
            return Err(NameError::InvalidLeading { ch });
        }
    }
    Ok(())
}

/// Single source of truth for the YAML keys this build understands.
/// Anything not in this list is preserved through `extra` —
/// `parse_frontmatter` filters by this set.
pub(crate) const KNOWN_KEYS: &[&str] = &[
    "name",
    "description",
    "when_to_use",
    "argument-hint",
    "arguments",
    "allowed-tools",
    "paths",
    "context",
    "agent",
    "model",
    "effort",
    "shell",
    "disable-model-invocation",
    "user-invocable",
];

/// Default for `user-invocable` when the key is absent — spec says
/// skills are user-invocable unless explicitly opted out.
pub(crate) const DEFAULT_USER_INVOCABLE: bool = true;

/// All key fields, exposed for the modal so caller order matches the
/// frontmatter spec (and so the modal's tab-order is stable).
pub fn extra_keys(fm: &SkillFrontmatter) -> Vec<String> {
    fm.extra.keys().cloned().collect()
}

#[allow(dead_code)]
fn _ensure_btreemap_in_use() {
    let _: BTreeMap<String, serde_yaml::Value> = BTreeMap::new();
}
