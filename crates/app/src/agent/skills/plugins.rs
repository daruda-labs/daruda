//! Parser for Claude Code's installed plugin manifest.
//!
//! `~/.claude/plugins/installed_plugins.json` is mirrored into the Skills tab's
//! Plugin section. Plugin ids are manifest keys (`<plugin>@<marketplace>`), and
//! one id can map to multiple scoped installs.

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// One install record for a plugin id; unrelated manifest fields are ignored.
#[derive(Clone, Debug, Deserialize)]
pub struct PluginInstall {
    /// `<plugin>@<marketplace>` — the manifest's dictionary key,
    /// filled in by the caller since it lives on the parent map.
    #[serde(skip)]
    pub id: String,
    /// `"user"` (global) or `"project"` (repo-local), kept as manifest data.
    #[serde(default)]
    pub scope: String,
    /// Resolved on-disk plugin path; `<install_path>/skills/<name>/SKILL.md`
    /// is what we scan.
    #[serde(rename = "installPath")]
    pub install_path: PathBuf,
    /// Version string, for display only; daruda doesn't compare versions.
    #[serde(default)]
    pub version: String,
}

/// Top-level wrapper; version is ignored so known fields keep parsing.
#[derive(Debug, Deserialize)]
struct InstalledPluginsFile {
    #[serde(default)]
    plugins: std::collections::BTreeMap<String, Vec<PluginInstall>>,
}

/// Default manifest path with deterministic fallback when home is unavailable.
pub fn installed_plugins_manifest() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claude")
        .join("plugins")
        .join("installed_plugins.json")
}

/// Default plugin cache root for watchers before any plugin is installed.
pub fn plugins_cache_root() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claude")
        .join("plugins")
        .join("cache")
}

/// Parent of both installed plugin cache and registered marketplaces.
pub fn plugins_root() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claude")
        .join("plugins")
}

/// `known_marketplaces.json` path. Lists every marketplace the user
/// has registered with `/plugin marketplace add` — the source repos
/// are cloned to `installLocation`, but the plugins inside aren't
/// activated until the user runs `/plugin install`.
pub fn known_marketplaces_manifest() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claude")
        .join("plugins")
        .join("known_marketplaces.json")
}

/// Whether a plugin row in the panel comes from an active install or
/// just from a registered marketplace (catalog browse).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PluginAvailability {
    /// In `installed_plugins.json`. Claude Code can invoke its skills
    /// today.
    Installed,
    /// Discovered via a marketplace clone but never `/plugin install`-ed.
    /// Surfacing it lets the user browse the catalog before installing.
    Available,
}

/// Per-marketplace registration in `known_marketplaces.json`.
#[derive(Clone, Debug)]
pub struct MarketplaceRegistration {
    /// Marketplace id (`addy-agent-skills`, `claude-plugins-official`).
    pub id: String,
    /// Where the marketplace repo is cloned. Used to locate
    /// `<install_location>/.claude-plugin/marketplace.json` and the
    /// per-plugin source directories.
    pub install_location: PathBuf,
}

#[derive(Clone, Debug, Deserialize)]
struct KnownMarketplaceEntry {
    #[serde(rename = "installLocation")]
    install_location: PathBuf,
}

/// Parse `known_marketplaces.json`. Errors collapse to `Vec::new()` —
/// the panel just shows no marketplace plugins, which is the right
/// UX for "no marketplaces registered".
pub fn read_known_marketplaces(path: &Path) -> Vec<MarketplaceRegistration> {
    let raw = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let parsed: std::collections::BTreeMap<String, KnownMarketplaceEntry> =
        match serde_json::from_str(&raw) {
            Ok(p) => p,
            Err(_) => return Vec::new(),
        };
    parsed
        .into_iter()
        .map(|(id, entry)| MarketplaceRegistration {
            id,
            install_location: entry.install_location,
        })
        .collect()
}

/// One plugin entry inside a marketplace's `.claude-plugin/marketplace.json`.
#[derive(Clone, Debug, Deserialize)]
pub struct MarketplacePluginEntry {
    /// Plugin id within the marketplace (e.g. `agent-skills`). The
    /// fully-qualified id daruda surfaces is `<plugin>@<marketplace>`.
    pub name: String,
    /// Optional path under the marketplace clone where this plugin
    /// lives. When omitted, single-plugin marketplaces (Addy's repo
    /// is one example) keep the plugin at the marketplace root.
    #[serde(default)]
    pub source: Option<MarketplacePluginSource>,
}

/// Source spec for a marketplace plugin. The format here is the
/// minimal subset daruda cares about — enough to figure out where to
/// scan for `skills/`. Anything not in this enum (`github`, `git`,
/// `url`, etc.) means "the plugin isn't bundled inside this clone, so
/// daruda can't preview its skills".
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum MarketplacePluginSource {
    /// `"./relative/path"` — bundled inside the marketplace clone.
    Path(String),
    /// Fallback for source kinds daruda doesn't preview (github / url
    /// / etc.). Captured as a generic value so parsing doesn't fail.
    Other(serde_json::Value),
}

#[derive(Clone, Debug, Deserialize)]
struct MarketplaceManifest {
    #[serde(default)]
    plugins: Vec<MarketplacePluginEntry>,
}

/// Read `<install_location>/.claude-plugin/marketplace.json` and
/// return the embedded plugin entries with the marketplace id stamped
/// onto each (so the caller has the namespace prefix it needs).
///
/// Single-plugin marketplaces frequently omit `source`; in that case
/// the plugin lives at the marketplace root. The caller resolves the
/// final filesystem path by joining `install_location` with the
/// (optional) `source` path.
pub fn read_marketplace_plugins(install_location: &Path) -> Vec<MarketplacePluginEntry> {
    let manifest_path = install_location
        .join(".claude-plugin")
        .join("marketplace.json");
    let raw = match std::fs::read_to_string(&manifest_path) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let parsed: MarketplaceManifest = match serde_json::from_str(&raw) {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };
    parsed.plugins
}

/// Parse the manifest at `path` and flatten it into a single
/// `Vec<PluginInstall>` with `id` populated from the dictionary key.
/// Errors (missing file, malformed JSON) collapse to an empty vec —
/// the panel just shows no plugin skills, which is the right UX for
/// "no plugins installed".
pub fn read_installed_plugins(path: &Path) -> Vec<PluginInstall> {
    let raw = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let parsed: InstalledPluginsFile = match serde_json::from_str(&raw) {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for (id, installs) in parsed.plugins {
        for mut install in installs {
            install.id = id.clone();
            out.push(install);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn parses_real_world_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let manifest = tmp.path().join("installed_plugins.json");
        fs::write(
            &manifest,
            r#"{
              "version": 2,
              "plugins": {
                "swift-lsp@claude-plugins-official": [{
                  "scope": "user",
                  "installPath": "/Users/x/.claude/plugins/cache/x/swift-lsp/1.0.0",
                  "version": "1.0.0",
                  "installedAt": "2026-02-06T09:30:57.914Z"
                }],
                "another@some-market": [{
                  "scope": "project",
                  "installPath": "/Users/x/.claude/plugins/cache/another/0.0.1",
                  "version": "0.0.1"
                }]
              }
            }"#,
        )
        .unwrap();
        let mut installs = read_installed_plugins(&manifest);
        installs.sort_by(|a, b| a.id.cmp(&b.id));
        assert_eq!(installs.len(), 2);
        assert_eq!(installs[0].id, "another@some-market");
        assert_eq!(installs[1].id, "swift-lsp@claude-plugins-official");
        assert_eq!(installs[1].scope, "user");
        assert_eq!(installs[1].version, "1.0.0");
    }

    #[test]
    fn missing_file_returns_empty_vec() {
        let tmp = tempfile::tempdir().unwrap();
        let installs = read_installed_plugins(&tmp.path().join("does_not_exist.json"));
        assert!(installs.is_empty());
    }

    #[test]
    fn malformed_json_is_swallowed() {
        let tmp = tempfile::tempdir().unwrap();
        let manifest = tmp.path().join("installed_plugins.json");
        fs::write(&manifest, "{ this isn't json").unwrap();
        assert!(read_installed_plugins(&manifest).is_empty());
    }

    #[test]
    fn one_id_with_multiple_installs_yields_one_entry_each() {
        let tmp = tempfile::tempdir().unwrap();
        let manifest = tmp.path().join("installed_plugins.json");
        fs::write(
            &manifest,
            r#"{
              "version": 2,
              "plugins": {
                "shared@m": [
                  {"scope": "user", "installPath": "/a", "version": "1"},
                  {"scope": "project", "installPath": "/b", "version": "1"}
                ]
              }
            }"#,
        )
        .unwrap();
        let installs = read_installed_plugins(&manifest);
        assert_eq!(installs.len(), 2);
        assert!(installs.iter().all(|p| p.id == "shared@m"));
    }
}
