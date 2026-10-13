//! Environment variables in the block form a user edits them in — one
//! `KEY=VALUE` per line. Shared by the MCP server editor and the agent
//! catalog's Environment field.

use std::collections::BTreeMap;

/// A line that is not `KEY=VALUE`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvLineError {
    pub line: String,
}

/// Parse `KEY=VALUE` lines (one per line) into a sorted map. Empty
/// lines and lines starting with `#` are treated as comments. Both
/// the key and the value are trimmed of surrounding whitespace —
/// `TOKEN=  abc  ` lands in the map as `("TOKEN", "abc")`. Embedded
/// whitespace (`KEY=foo bar`) is preserved verbatim. An empty value
/// (`KEY=`) is allowed and stored as the empty string — Claude Code
/// writes a literal empty env var in this case.
pub fn parse_env_lines(text: &str) -> Result<BTreeMap<String, String>, EnvLineError> {
    let mut map = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (k, v) = line.split_once('=').ok_or_else(|| EnvLineError {
            line: line.to_string(),
        })?;
        let k = k.trim();
        if k.is_empty() {
            return Err(EnvLineError {
                line: line.to_string(),
            });
        }
        map.insert(k.to_string(), v.trim().to_string());
    }
    Ok(map)
}

/// Render an env map back to the block form [`parse_env_lines`] reads.
pub fn format_env_lines(env: &BTreeMap<String, String>) -> String {
    env.iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("\n")
}
