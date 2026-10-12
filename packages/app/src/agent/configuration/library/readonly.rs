//! Locally inspectable policy and plugin sources. Never policy mutation.
use super::*;

pub(crate) fn managed_dir() -> Option<PathBuf> {
    match std::env::consts::OS {
        "macos" => Some("/Library/Application Support/ClaudeCode".into()),
        "linux" => Some("/etc/claude-code".into()),
        "windows" => Some(r"C:\Program Files\ClaudeCode".into()),
        _ => None,
    }
}

pub(super) fn append(
    config: &Configuration,
    out: &mut Inventory,
) -> Result<(), Box<dyn std::error::Error>> {
    if config.provider != Provider::Claude {
        return Ok(());
    }
    if let Some(dir) = managed_dir() {
        let path = dir.join("managed-mcp.json");
        if path.exists() {
            let raw = mcp::read_json_or_empty(&path)?;
            out.managed_exclusive = true;
            group(config, path, "Managed".into(), &raw, out);
        }
        let path = dir.join("managed-settings.json");
        let raw = mcp::read_json_or_empty(&path)?;
        if let Some(servers) = raw.get("managedMcpServers") {
            group(
                config,
                path,
                "Managed".into(),
                &serde_json::json!({"mcpServers":servers}),
                out,
            );
        }
    }
    for plugin in
        skills::plugins::read_installed_plugins(&config.root.join("plugins/installed_plugins.json"))
    {
        let path = plugin.install_path.join(".mcp.json");
        if path.exists() {
            group(
                config,
                path.clone(),
                plugin.id,
                &mcp::read_json_or_empty(&path)?,
                out,
            );
        }
    }
    Ok(())
}
fn group(
    config: &Configuration,
    path: PathBuf,
    owner: String,
    raw: &serde_json::Value,
    out: &mut Inventory,
) {
    let mut target = Target::new(config, McpScope::User, None).unwrap();
    target.path = path;
    target.owner = Some(owner);
    let servers = claude_servers(raw, &McpLocation::TopLevel, McpScope::User);
    out.groups.push(Group { target, servers });
}
