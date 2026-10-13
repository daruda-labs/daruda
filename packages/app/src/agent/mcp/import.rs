//! Parse pasted connection instructions without invoking a shell or resolving secrets.

use super::{McpServerDraft, McpTransport};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
mod registration;
mod unique_json;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImportError {
    Empty,
    Syntax,
    Unsupported,
    InvalidField,
    RegistrationCommand,
}

#[derive(Deserialize)]
struct Entry {
    #[serde(rename = "type")]
    transport: Option<String>,
    command: Option<String>,
    #[serde(default)]
    args: Vec<String>,
    url: Option<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    disabled: bool,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

/// Refuse lossy edits of known fields while allowing incomplete records to be repaired.
pub(crate) fn validate_existing(value: &Value) -> Result<(), ImportError> {
    let entry: Entry =
        serde_json::from_value(value.clone()).map_err(|_| ImportError::InvalidField)?;
    if entry
        .transport
        .as_deref()
        .is_some_and(|t| !matches!(t, "stdio" | "http" | "streamable-http" | "sse"))
    {
        return Err(ImportError::Unsupported);
    }
    Ok(())
}

pub(crate) fn parse_for(
    source: &str,
    provider: Option<crate::agent::configuration::Provider>,
) -> Result<Vec<McpServerDraft>, ImportError> {
    if provider == Some(crate::agent::configuration::Provider::Codex)
        && (source.trim_start().starts_with('[') || source.trim_start().starts_with('#'))
    {
        let doc = source.parse().map_err(|_| ImportError::Syntax)?;
        let servers = crate::agent::configuration::codex::servers(&doc, super::McpScope::User)
            .map_err(|_| ImportError::InvalidField)?;
        if servers.is_empty() {
            return Err(ImportError::Empty);
        }
        return Ok(servers.iter().map(McpServerDraft::from_server).collect());
    }
    if provider == Some(crate::agent::configuration::Provider::Codex) {
        parse_input(source, provider)
    } else {
        parse(source)
    }
}

pub(crate) fn parse(source: &str) -> Result<Vec<McpServerDraft>, ImportError> {
    parse_input(source, None)
}

fn parse_input(
    source: &str,
    provider: Option<crate::agent::configuration::Provider>,
) -> Result<Vec<McpServerDraft>, ImportError> {
    let source = source.trim();
    if source.is_empty() {
        return Err(ImportError::Empty);
    }
    if source.starts_with('{') {
        let unique_json::Unique(value) =
            serde_json::from_str(source).map_err(|_| ImportError::Syntax)?;
        let mut object = value
            .as_object()
            .cloned()
            .ok_or(ImportError::InvalidField)?;
        let entries = if let Some(servers) = object.remove("mcpServers") {
            // Full config documents may have siblings; importing selects only servers.
            servers
                .as_object()
                .cloned()
                .ok_or(ImportError::InvalidField)?
        } else if object.contains_key("command") || object.contains_key("url") {
            serde_json::Map::from_iter([(String::new(), Value::Object(object))])
        } else {
            object
        };
        if entries.is_empty() {
            return Err(ImportError::Empty);
        }
        return entries
            .into_iter()
            .map(|(name, value)| entry(name, value))
            .collect();
    }
    if source.starts_with('[') || source.starts_with('#') {
        return codex(source);
    }
    if source.starts_with("https://") || source.starts_with("http://") {
        return Ok(vec![entry(
            String::new(),
            serde_json::json!({"url":source}),
        )?]);
    }
    if let Some(result) = registration::parse(source, provider) {
        return result.map(|draft| vec![draft]);
    }
    // A launch command, never a shell program. Reject expansions and compounds,
    // including inside quotes, rather than silently changing their semantics.
    if source.contains(['\n', '\r', ';', '|', '&', '<', '>', '`', '$']) {
        return Err(ImportError::Unsupported);
    }
    let mut words = shell_words::split(source).map_err(|_| ImportError::Syntax)?;
    if words.is_empty() || words[0].contains('=') {
        return Err(ImportError::Unsupported);
    }
    let command = words.remove(0);
    Ok(vec![entry(
        String::new(),
        serde_json::json!({"command":command,"args":words}),
    )?])
}

fn entry(name: String, value: Value) -> Result<McpServerDraft, ImportError> {
    let e: Entry = serde_json::from_value(value).map_err(|_| ImportError::InvalidField)?;
    let mut header_names = std::collections::HashSet::new();
    if e.headers
        .keys()
        .any(|key| !header_names.insert(key.to_ascii_lowercase()))
    {
        return Err(ImportError::InvalidField);
    }
    let transport = match e.transport.as_deref() {
        Some("stdio") => McpTransport::Stdio,
        Some("http" | "streamable-http") => McpTransport::Http,
        Some("sse") => McpTransport::Sse,
        None if e.command.is_some() => McpTransport::Stdio,
        None if e.url.is_some() => McpTransport::Http,
        _ => return Err(ImportError::Unsupported),
    };
    if transport.is_remote() {
        if e.command.is_some() || !e.args.is_empty() {
            return Err(ImportError::InvalidField);
        }
    } else if e.url.is_some() || !e.headers.is_empty() {
        return Err(ImportError::InvalidField);
    }
    super::validate_command(e.command.as_deref().unwrap_or(""), transport)
        .map_err(|_| ImportError::InvalidField)?;
    super::validate_url(e.url.as_deref().unwrap_or(""), transport)
        .map_err(|_| ImportError::InvalidField)?;
    Ok(McpServerDraft {
        name,
        transport,
        command: e.command,
        args: e.args,
        url: e.url,
        env: e.env,
        headers: e.headers,
        disabled: e.disabled,
        extra: e.extra,
    })
}

fn codex(source: &str) -> Result<Vec<McpServerDraft>, ImportError> {
    let doc: toml::Value = toml::from_str(source).map_err(|_| ImportError::Syntax)?;
    let servers = doc
        .get("mcp_servers")
        .and_then(toml::Value::as_table)
        .ok_or(ImportError::InvalidField)?;
    if servers.is_empty() {
        return Err(ImportError::Empty);
    }
    servers
        .iter()
        .map(|(name, value)| {
            let mut obj = serde_json::to_value(value).map_err(|_| ImportError::InvalidField)?;
            let obj = obj.as_object_mut().ok_or(ImportError::InvalidField)?;
            // Only settings with a known Claude equivalent can be converted.
            if obj.keys().any(|k| {
                ![
                    "command",
                    "args",
                    "url",
                    "env",
                    "http_headers",
                    "env_http_headers",
                    "bearer_token_env_var",
                ]
                .contains(&k.as_str())
            }) {
                return Err(ImportError::Unsupported);
            }
            // Codex strings are literal; Claude expands these expressions.
            if obj.values().any(contains_expansion) {
                return Err(ImportError::Unsupported);
            }
            let mut headers: BTreeMap<String, String> = match obj.remove("http_headers") {
                Some(v) => serde_json::from_value(v).map_err(|_| ImportError::InvalidField)?,
                None => BTreeMap::new(),
            };
            if let Some(v) = obj.remove("env_http_headers") {
                let variables: BTreeMap<String, String> =
                    serde_json::from_value(v).map_err(|_| ImportError::InvalidField)?;
                for (key, variable) in variables {
                    if !valid_variable(&variable)
                        || headers.keys().any(|k| k.eq_ignore_ascii_case(&key))
                    {
                        return Err(ImportError::InvalidField);
                    }
                    headers.insert(key, format!("${{{variable}}}"));
                }
            }
            if let Some(v) = obj.remove("bearer_token_env_var") {
                let variable = v
                    .as_str()
                    .filter(|s| valid_variable(s))
                    .ok_or(ImportError::InvalidField)?;
                if headers
                    .keys()
                    .any(|k| k.eq_ignore_ascii_case("authorization"))
                {
                    return Err(ImportError::InvalidField);
                }
                headers.insert("Authorization".into(), format!("Bearer ${{{variable}}}"));
            }
            if !headers.is_empty() {
                obj.insert("headers".into(), serde_json::json!(headers));
            }
            entry(name.clone(), Value::Object(obj.clone()))
        })
        .collect()
}

fn contains_expansion(value: &Value) -> bool {
    match value {
        Value::String(s) => s.contains("${"),
        Value::Array(v) => v.iter().any(contains_expansion),
        Value::Object(v) => v.values().any(contains_expansion),
        _ => false,
    }
}

pub(crate) fn valid_variable(name: &str) -> bool {
    daruda_core::process::is_valid_env_name(name)
}

#[cfg(test)]
mod tests;
