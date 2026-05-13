//! Filesystem scanner for `.claude/skills/<name>/` directories.
//!
//! Pure I/O: walks the directory entries under a scope root, parses
//! each `SKILL.md`, and produces a `Vec<Skill>` sorted by name. No
//! GPUI types involved — callers (`Workspace::reload_skills`,
//! `SkillsState::reload_scope`) wrap this in `cx.background_executor`
//! when invoked from the UI thread.

use std::path::{Path, PathBuf};

use super::plugins::{
    MarketplacePluginSource, PluginAvailability, installed_plugins_manifest,
    known_marketplaces_manifest, read_installed_plugins, read_known_marketplaces,
    read_marketplace_plugins,
};
use super::{Skill, SkillScope, parse_frontmatter, split_frontmatter};

/// Maximum length of [`Skill::body_preview`]. Matches the row's
/// rendered width budget — anything longer is ellipsized in the UI
/// anyway.
pub const PREVIEW_MAX_CHARS: usize = 200;

/// Both root paths tracked by the watcher. Hosts construct this once
/// per Workspace and pass it through to `SkillsState::load`.
#[derive(Clone, Debug)]
pub struct SkillsRoots {
    pub project: Option<PathBuf>,
    pub personal: PathBuf,
}

/// Resolve `<worktree_root>/.claude/skills/`.
pub fn skills_project_dir(worktree_root: &Path) -> PathBuf {
    worktree_root.join(".claude").join("skills")
}

/// Resolve `~/.claude/skills/`. Falls back to `./.claude/skills` when
/// `dirs::home_dir()` is unavailable so tests / sandboxed runs keep a
/// deterministic path.
pub fn skills_personal_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claude")
        .join("skills")
}

/// Walk `scope_root` and collect every directory containing a
/// `SKILL.md`. Errors per entry are swallowed (the scanner must keep
/// going if one skill fails to read) — caller relies on the `Vec`
/// being non-fatal.
pub fn scan_scope(scope_root: &Path, scope: SkillScope) -> Vec<Skill> {
    let read_dir = match std::fs::read_dir(scope_root) {
        Ok(rd) => rd,
        Err(_) => return Vec::new(),
    };

    let mut skills: Vec<Skill> = Vec::new();
    for entry in read_dir.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(skill) = read_skill_dir(&path, scope, None) else {
            continue;
        };
        skills.push(skill);
    }
    skills.sort_by(|a, b| {
        a.name
            .to_ascii_lowercase()
            .cmp(&b.name.to_ascii_lowercase())
    });
    skills
}

/// Plugin-scope skills surfaced from two sources:
///
/// 1. **Installed** plugins — every entry in
///    `~/.claude/plugins/installed_plugins.json`. These are the
///    plugins Claude Code can invoke today (`/<plugin>:<skill>`).
/// 2. **Available** plugins — every plugin declared in any registered
///    marketplace (`known_marketplaces.json` →
///    `<install_location>/.claude-plugin/marketplace.json`) whose
///    `source` resolves to a path inside the marketplace clone. Lets
///    the user browse a catalog before running `/plugin install`.
///
/// Skills appear at most once: if a plugin id is in both lists, the
/// installed copy wins. Skill names are namespaced (`<plugin>:<skill>`)
/// so they line up with Claude Code's invocation form.
pub fn scan_plugins() -> Vec<Skill> {
    let mut out: Vec<Skill> = Vec::new();
    let mut seen_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Pass 1 — installed plugins.
    let installs = read_installed_plugins(&installed_plugins_manifest());
    for install in installs {
        let id_local = id_local_part(&install.id);
        seen_ids.insert(install.id.clone());
        scan_plugin_skills_dir(
            &install.install_path,
            &install.id,
            &id_local,
            PluginAvailability::Installed,
            &mut out,
        );
    }

    // Pass 2 — marketplace catalog. Skip plugins whose id is already
    // covered by the installed pass so the panel doesn't double up.
    let marketplaces = read_known_marketplaces(&known_marketplaces_manifest());
    for market in marketplaces {
        for plugin in read_marketplace_plugins(&market.install_location) {
            let id = format!("{}@{}", plugin.name, market.id);
            if seen_ids.contains(&id) {
                continue;
            }
            let plugin_root = match plugin.source {
                Some(MarketplacePluginSource::Path(rel)) => {
                    market.install_location.join(rel.trim_start_matches("./"))
                }
                // No path → single-plugin marketplace where the clone
                // root *is* the plugin (Addy's `addy-agent-skills`
                // marketplace is structured this way).
                None => market.install_location.clone(),
                // Other source kinds (github / url / git-subdir) mean
                // the plugin isn't bundled in this clone, so daruda
                // has nothing to preview.
                Some(MarketplacePluginSource::Other(_)) => continue,
            };
            scan_plugin_skills_dir(
                &plugin_root,
                &id,
                &plugin.name,
                PluginAvailability::Available,
                &mut out,
            );
            seen_ids.insert(id);
        }
    }

    out.sort_by(|a, b| {
        a.name
            .to_ascii_lowercase()
            .cmp(&b.name.to_ascii_lowercase())
    });
    out
}

/// Helper for `scan_plugins` — read `<plugin_root>/skills/` and push
/// each `SKILL.md` directory into `out` with the right scope, plugin
/// id, and availability stamped on.
fn scan_plugin_skills_dir(
    plugin_root: &Path,
    plugin_id: &str,
    id_local: &str,
    availability: PluginAvailability,
    out: &mut Vec<Skill>,
) {
    let skills_dir = plugin_root.join("skills");
    let read_dir = match std::fs::read_dir(&skills_dir) {
        Ok(rd) => rd,
        Err(_) => return,
    };
    for entry in read_dir.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if let Some(mut skill) = read_skill_dir(&path, SkillScope::Plugin, Some(plugin_id.into())) {
            skill.name = format!("{id_local}:{}", skill.name);
            skill.plugin_availability = Some(availability);
            out.push(skill);
        }
    }
}

/// Strip the trailing `@<marketplace>` from a plugin id so the
/// `<plugin>:<skill>` namespace matches Claude Code's slash-command
/// form. `swift-lsp@claude-plugins-official` → `swift-lsp`.
fn id_local_part(id: &str) -> String {
    id.split_once('@')
        .map(|(local, _)| local.to_string())
        .unwrap_or_else(|| id.to_string())
}

/// Read one `<scope_root>/<name>/` directory. Returns `None` when
/// `SKILL.md` is missing or unreadable — the scanner skips silently.
fn read_skill_dir(dir: &Path, scope: SkillScope, plugin_id: Option<String>) -> Option<Skill> {
    let skill_md = dir.join("SKILL.md");
    let raw = std::fs::read_to_string(&skill_md).ok()?;
    let (yaml, body) = split_frontmatter(&raw);
    let frontmatter = match yaml {
        Some(src) => parse_frontmatter(src).ok()?,
        None => super::SkillFrontmatter::empty(),
    };

    let dir_name = dir.file_name()?.to_string_lossy().to_string();
    if dir_name.is_empty() {
        return None;
    }
    let name = frontmatter
        .name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or(dir_name);

    let body_preview = body_preview(body);
    let aux_file_count = count_aux_files(dir);
    let modified_at = std::fs::metadata(&skill_md)
        .and_then(|m| m.modified())
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);

    Some(Skill {
        name,
        dir: dir.to_path_buf(),
        scope,
        frontmatter,
        body_preview,
        aux_file_count,
        modified_at,
        plugin_id,
        plugin_availability: None,
    })
}

/// First non-empty paragraph of `body`, trimmed and clamped to
/// [`PREVIEW_MAX_CHARS`]. A "paragraph" ends at the first blank line.
pub fn body_preview(body: &str) -> String {
    let mut start = 0usize;
    let mut chars = body.char_indices().peekable();
    // Skip leading blank lines.
    while let Some(&(idx, ch)) = chars.peek() {
        if ch == '\n' || ch == '\r' || ch == ' ' || ch == '\t' {
            chars.next();
            start = idx + ch.len_utf8();
        } else {
            break;
        }
    }
    let rest = &body[start..];
    let mut end = rest.len();
    let bytes = rest.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'\n' {
            // Look ahead for another newline — possibly with `\r` /
            // whitespace in between.
            let mut j = i + 1;
            while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t' || bytes[j] == b'\r') {
                j += 1;
            }
            if j >= bytes.len() || bytes[j] == b'\n' {
                end = i;
                break;
            }
        }
        i += 1;
    }

    let mut s = rest[..end].replace(['\n', '\r'], " ");
    s = s.split_whitespace().collect::<Vec<_>>().join(" ");

    if s.chars().count() > PREVIEW_MAX_CHARS {
        let trimmed: String = s.chars().take(PREVIEW_MAX_CHARS).collect();
        return format!("{trimmed}…");
    }
    s
}

/// Count files under `dir` excluding the top-level `SKILL.md`. Walks
/// recursively so `scripts/` / `examples.md` / nested dirs all count.
fn count_aux_files(dir: &Path) -> u32 {
    let mut count = 0u32;
    let mut stack = vec![dir.to_path_buf()];
    let skill_md = dir.join("SKILL.md");
    while let Some(cur) = stack.pop() {
        let read_dir = match std::fs::read_dir(&cur) {
            Ok(rd) => rd,
            Err(_) => continue,
        };
        for entry in read_dir.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path == skill_md {
                continue;
            }
            count = count.saturating_add(1);
        }
    }
    count
}
