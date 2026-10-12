//! Configuration discovery and immutable mutation targets shared by dock forms.

pub(crate) mod creation;
pub(crate) mod readonly;

use super::{Configuration, Provider, codex};
use crate::agent::{
    mcp::{self, McpLocation, McpScope, McpServerDraft, McpSnapshot},
    skills::{self, SkillScope, SkillsSnapshot},
};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Target {
    pub config: Configuration,
    pub scope: McpScope,
    pub path: PathBuf,
    pub location: McpLocation,
    pub owner: Option<String>,
    pub resolved: Option<PathBuf>,
    enablement_resolved: Option<PathBuf>,
}

impl Target {
    pub fn new(config: &Configuration, scope: McpScope, dir: Option<&Path>) -> Option<Self> {
        let path = match scope {
            McpScope::User => config.user_file.clone(),
            McpScope::Local if config.provider == Provider::Claude => config.user_file.clone(),
            McpScope::Project => config.project_file(dir.or(config.cwd.as_deref())?),
            _ => return None,
        };
        let location = if scope == McpScope::Local {
            McpLocation::project(config.cwd.as_deref()?)
        } else {
            McpLocation::TopLevel
        };
        Some(Self {
            config: config.clone(),
            scope,
            path,
            location,
            owner: None,
            resolved: None,
            enablement_resolved: None,
        })
    }

    pub fn label(&self) -> String {
        match &self.location {
            McpLocation::TopLevel => self.path.display().to_string(),
            McpLocation::ProjectChild(key) => {
                format!("{} → projects[\"{key}\"]", self.path.display())
            }
        }
    }

    pub fn mutate(
        &self,
        name: &str,
        expected: Option<&McpServerDraft>,
        draft: Option<&McpServerDraft>,
    ) -> Result<(), mcp::McpPersistError> {
        if self.owner.is_some() {
            return Err(std::io::Error::other("This configuration is managed by its owner").into());
        }
        if let Some(expected) = &self.resolved
            && resolve_write_path(&self.path)? != *expected
        {
            return Err(std::io::Error::other(
                "Configuration source was replaced; reload before saving",
            )
            .into());
        }
        if self.config.provider == Provider::Codex {
            return codex::write(&self.path, name, expected, draft)
                .map_err(mcp::McpPersistError::Io);
        }
        let path = physical(&self.path)?;
        let mut raw = mcp::read_json_or_empty(&path)?;
        check_local_key(&raw, &self.location)?;
        let record = match &self.location {
            McpLocation::TopLevel => raw.get("mcpServers"),
            McpLocation::ProjectChild(key) => raw
                .get("projects")
                .and_then(|p| p.get(key))
                .and_then(|p| p.get("mcpServers")),
        }
        .and_then(|m| m.get(name));
        if let Some(record) = record {
            mcp::import::validate_existing(record).map_err(|_| invalid())?;
        }
        let servers = claude_servers(&raw, &self.location, self.scope);
        let current = servers
            .iter()
            .find(|s| s.name == name)
            .map(McpServerDraft::from_server);
        // Enablement belongs to the captured cwd, not the server record.
        let normalize = |draft: &McpServerDraft| {
            let mut draft = draft.clone();
            draft.disabled = false;
            draft.to_value()
        };
        if current.as_ref().map(&normalize) != expected.map(&normalize) {
            return Err(std::io::Error::other("MCP source changed; reload before saving").into());
        }
        match draft {
            Some(draft) => {
                let mut value = draft.clone();
                value.disabled = false;
                if expected.is_some() {
                    mcp::update_server(&mut raw, &path, self.scope, &self.location, &value)?;
                } else {
                    mcp::write_server(&mut raw, &path, self.scope, &self.location, &value)?;
                }
            }
            None => mcp::delete_server(&mut raw, &path, self.scope, &self.location, name)?,
        }
        Ok(())
    }

    pub fn toggle(&self, expected: &McpServerDraft) -> Result<(), mcp::McpPersistError> {
        if self.owner.is_some() {
            return Err(std::io::Error::other("This configuration is managed by its owner").into());
        }
        if let Some(expected) = &self.resolved
            && resolve_write_path(&self.path)? != *expected
        {
            return Err(std::io::Error::other(
                "Configuration source was replaced; reload before saving",
            )
            .into());
        }
        if self.config.provider == Provider::Codex {
            let mut draft = expected.clone();
            draft.disabled = !draft.disabled;
            return self.mutate(&draft.name, Some(expected), Some(&draft));
        }
        if let Some(pinned) = &self.enablement_resolved
            && resolve_write_path(&self.config.user_file)? != *pinned
        {
            return Err(std::io::Error::other(
                "Configuration source was replaced; reload before saving",
            )
            .into());
        }
        let source = mcp::read_json_or_empty(&self.path)?;
        let current = claude_servers(&source, &self.location, self.scope)
            .iter()
            .find(|server| server.name == expected.name)
            .map(McpServerDraft::from_server);
        let normalize = |draft: &McpServerDraft| {
            let mut draft = draft.clone();
            draft.disabled = false;
            draft.to_value()
        };
        if current.as_ref().map(normalize) != Some(normalize(expected)) {
            return Err(std::io::Error::other("MCP source changed; reload before saving").into());
        }
        let cwd = self
            .config
            .cwd
            .as_deref()
            .ok_or(mcp::McpPersistError::NoProjectRoot)?;
        let path = physical(&self.config.user_file)?;
        let mut raw = mcp::read_json_or_empty(&path)?;

        let key = local_key(&raw, &cwd.to_string_lossy())?;
        let projects = raw
            .as_object_mut()
            .ok_or_else(invalid)?
            .entry("projects")
            .or_insert_with(|| serde_json::json!({}))
            .as_object_mut()
            .ok_or_else(invalid)?;
        let project = projects
            .entry(key)
            .or_insert_with(|| serde_json::json!({}))
            .as_object_mut()
            .ok_or_else(invalid)?;
        let list = project
            .entry("disabledMcpServers")
            .or_insert_with(|| serde_json::json!([]))
            .as_array_mut()
            .ok_or_else(invalid)?;
        if !list.iter().all(|v| v.is_string()) {
            return Err(invalid().into());
        }
        let disabled = list.iter().any(|v| v.as_str() == Some(&expected.name));
        if disabled != expected.disabled {
            return Err(
                std::io::Error::other("MCP enablement changed; reload before saving").into(),
            );
        }
        list.retain(|v| v.as_str() != Some(&expected.name));
        if !disabled {
            list.push(expected.name.clone().into());
        }
        daruda_store::persistence::save_json_atomic(path.parent().ok_or_else(invalid)?, &path, &raw)
            .map_err(mcp::McpPersistError::Io)
    }
}

fn invalid() -> std::io::Error {
    std::io::Error::other("Invalid configuration structure")
}
fn resolve_write_path(path: &Path) -> std::io::Result<PathBuf> {
    for anchor in path.ancestors() {
        match daruda_core::path::canonicalize(anchor) {
            Ok(root) => return Ok(root.join(path.strip_prefix(anchor).map_err(|_| invalid())?)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        }
    }
    Err(invalid())
}
fn physical(path: &Path) -> std::io::Result<PathBuf> {
    match daruda_core::path::canonicalize(path) {
        Ok(path) => Ok(path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(path.to_path_buf()),
        Err(e) => Err(e),
    }
}

fn local_key(raw: &serde_json::Value, key: &str) -> std::io::Result<String> {
    let Some(projects) = raw.get("projects").and_then(serde_json::Value::as_object) else {
        return Ok(key.into());
    };
    if projects.contains_key(key) {
        return Ok(key.into());
    }
    let mut aliases = projects
        .keys()
        .filter(|candidate| daruda_core::path::same_path(Path::new(candidate), Path::new(key)));
    let first = aliases.next();
    if aliases.next().is_some() {
        return Err(std::io::Error::other(
            "Ambiguous Local cwd keys; choose the exact source spelling",
        ));
    }
    Ok(first.map_or_else(|| key.to_string(), Clone::clone))
}
fn check_local_key(raw: &serde_json::Value, location: &McpLocation) -> std::io::Result<()> {
    if let McpLocation::ProjectChild(key) = location
        && local_key(raw, key)? != *key
    {
        return Err(std::io::Error::other(
            "Local configuration key changed; reload before saving",
        ));
    }
    Ok(())
}
fn claude_servers(
    raw: &serde_json::Value,
    location: &McpLocation,
    scope: McpScope,
) -> Vec<mcp::McpServer> {
    let mut servers = mcp::extract_servers_at(raw, location, scope);
    let map = match location {
        McpLocation::TopLevel => raw.get("mcpServers"),
        McpLocation::ProjectChild(key) => raw
            .get("projects")
            .and_then(|p| p.get(key))
            .and_then(|p| p.get("mcpServers")),
    };
    for server in &mut servers {
        if let Some(legacy) = map
            .and_then(|m| m.get(&server.name))
            .and_then(|s| s.get("disabled"))
        {
            server.extra.insert("disabled".into(), legacy.clone());
        }
    }
    servers
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Group {
    pub target: Target,
    pub servers: Vec<mcp::McpServer>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Inventory {
    pub creation_targets: Vec<creation::CreationTarget>,
    pub skills: SkillsSnapshot,
    pub mcp: McpSnapshot,
    pub groups: Vec<Group>,
    pub trust: codex::Trust,
    pub managed_exclusive: bool,
    pub shared_paths: Vec<(PathBuf, PathBuf)>,
}

pub(crate) fn discover(config: &Configuration) -> Result<Inventory, String> {
    discover_inner(config).map_err(|_| {
        "Configuration could not be read; the last successful snapshot is retained".into()
    })
}

fn scan_skills(root: &Path, scope: SkillScope) -> std::io::Result<Vec<skills::Skill>> {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    for entry in entries {
        let path = entry?.path().join("SKILL.md");
        match std::fs::read_to_string(&path) {
            Ok(source) => {
                skills::import::parse(&source).map_err(|_| invalid())?;
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                ) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(skills::scan::scan_scope(root, scope))
}

fn discover_inner(config: &Configuration) -> Result<Inventory, Box<dyn std::error::Error>> {
    let dirs = config.project_dirs();
    let mut out = Inventory {
        creation_targets: creation::CreationTarget::capture(config)?,
        ..Default::default()
    };
    out.skills.project_root = config.cwd.clone();
    out.skills.personal = scan_skills(&config.personal_skills, SkillScope::Personal)?;
    for dir in &dirs {
        out.skills.project.extend(scan_skills(
            &config.project_skills(dir),
            SkillScope::Project,
        )?);
    }
    if config.provider == Provider::Claude {
        out.skills.plugin =
            skills::scan::scan_plugins_at(&config.root.join("plugins"), &config.personal_skills);
    } else {
        for root in [
            PathBuf::from("/etc/codex/skills"),
            config.root.join("skills/.system"),
        ] {
            if root.is_absolute() {
                let mut entries = scan_skills(&root, SkillScope::Plugin)?;
                for skill in &mut entries {
                    skill.plugin_id = Some(root.display().to_string());
                }
                out.skills.plugin.extend(entries);
            }
        }
    }
    let claude = if config.provider == Provider::Claude {
        Some(mcp::read_json_or_empty(&config.user_file)?)
    } else {
        None
    };
    if config.provider == Provider::Codex {
        out.trust = codex::trust(&codex::read(&config.user_file)?, &dirs);
    }
    let mut targets = vec![Target::new(config, McpScope::User, None).unwrap()];
    for dir in &dirs {
        targets.push(Target::new(config, McpScope::Project, Some(dir)).unwrap());
    }
    if let Some(local) = Target::new(config, McpScope::Local, None) {
        targets.push(local);
    }
    for mut target in targets {
        if let (Some(raw), McpLocation::ProjectChild(key)) = (&claude, &target.location) {
            target.location = McpLocation::ProjectChild(local_key(raw, key)?);
        }
        let mut servers = match config.provider {
            Provider::Claude => {
                let raw = if target.scope == McpScope::Project {
                    mcp::read_json_or_empty(&target.path)?
                } else {
                    claude.clone().unwrap()
                };
                check_local_key(&raw, &target.location)?;
                claude_servers(&raw, &target.location, target.scope)
            }
            Provider::Codex => codex::servers(&codex::read(&target.path)?, target.scope)?,
        };
        if let (Some(raw), Some(cwd)) = (&claude, &config.cwd) {
            let key = local_key(raw, &cwd.to_string_lossy())?;
            let disabled = raw
                .get("projects")
                .and_then(|v| v.get(&key))
                .and_then(|v| v.get("disabledMcpServers"))
                .and_then(serde_json::Value::as_array);
            for server in &mut servers {
                server.disabled =
                    disabled.is_some_and(|v| v.iter().any(|s| s.as_str() == Some(&server.name)));
            }
        }
        match target.scope {
            McpScope::User => out.mcp.user.extend(servers.clone()),
            McpScope::Local => out.mcp.local.extend(servers.clone()),
            McpScope::Project => out.mcp.project.extend(servers.clone()),
        }
        target.resolved = Some(resolve_write_path(&target.path)?);
        target.enablement_resolved = Some(resolve_write_path(&config.user_file)?);
        out.groups.push(Group { target, servers });
    }
    out.mcp.project_root = config.cwd.clone();
    out.mcp.project_mcp_path = config.cwd.as_ref().map(|p| config.project_file(p));
    out.mcp.claude_json_path = config.user_file.clone();
    out.mcp.last_scanned = Some(std::time::SystemTime::now());
    out.skills.last_scanned = out.mcp.last_scanned;
    for path in [&config.user_file, &config.personal_skills] {
        if let Ok(real) = daruda_core::path::canonicalize(path)
            && real != *path
        {
            out.shared_paths.push((path.clone(), real));
        }
    }
    readonly::append(config, &mut out)?;
    Ok(out)
}

#[cfg(test)]
mod tests;
