//! Decode registration commands as data, never as server executables.
use super::{ImportError, McpServerDraft, entry};
use crate::agent::configuration::Provider;
use serde_json::json;
use std::collections::BTreeMap;

pub(super) fn parse(
    source: &str,
    provider: Option<Provider>,
) -> Option<Result<McpServerDraft, ImportError>> {
    let words = shell_words::split(source).ok()?;
    let executable = words.first()?.rsplit(['/', '\\']).next()?;
    let is_client = |s: &str| {
        matches!(
            s.rsplit(['/', '\\']).next(),
            Some("claude" | "codex" | "claude.exe" | "codex.exe")
        )
    };
    if !is_client(executable) {
        // Wrappers must not turn registration into a server launch either.
        return words
            .windows(2)
            .any(|pair| is_client(&pair[0]) && pair[1] == "mcp")
            .then_some(Err(ImportError::RegistrationCommand));
    }
    if !words.iter().take_while(|s| *s != "--").any(|s| s == "mcp") {
        return None;
    }
    Some(decode(&words, source, provider).map_err(|_| ImportError::RegistrationCommand))
}

fn decode(
    words: &[String],
    source: &str,
    provider: Option<Provider>,
) -> Result<McpServerDraft, ImportError> {
    let invalid = ImportError::RegistrationCommand;
    // Accept shell line continuations, but not expansion or compound commands.
    let normalized = source.replace("\\\r\n", "").replace("\\\n", "");
    if normalized.contains(['\n', '\r', ';', '|', '&', '<', '>', '`', '$'])
        || words.get(1).map(String::as_str) != Some("mcp")
        || words.get(2).map(String::as_str) != Some("add")
    {
        return Err(invalid);
    }
    let codex = words[0]
        .rsplit(['/', '\\'])
        .next()
        .unwrap()
        .starts_with("codex");
    let mut name = None;
    let mut endpoint = None;
    let mut transport = None;
    let mut env = BTreeMap::new();
    let mut headers = BTreeMap::new();
    let mut bearer = None;
    let mut i = 3;
    while i < words.len() {
        let word = words[i].as_str();
        if word == "--" {
            if endpoint.is_some() || bearer.is_some() || transport.is_some_and(|t| t != "stdio") {
                return Err(invalid);
            }
            let command = words.get(i + 1).ok_or(invalid)?;
            return entry(
                name.ok_or(invalid)?,
                json!({
                    "command": command, "args": &words[i + 2..], "env": env,
                    "headers": headers,
                }),
            );
        }
        match word {
            "--transport" | "-t" if !codex => {
                if transport.is_some() {
                    return Err(invalid);
                }
                i += 1;
                let value = words.get(i).ok_or(invalid)?.as_str();
                if !matches!(value, "stdio" | "http" | "sse") {
                    return Err(invalid);
                }
                transport = Some(value);
            }
            "--env" | "-e" => {
                i += 1;
                let (key, value) = words
                    .get(i)
                    .and_then(|s| s.split_once('='))
                    .ok_or(invalid)?;
                if !super::valid_variable(key) || env.insert(key, value).is_some() {
                    return Err(invalid);
                }
            }
            "--header" | "-H" if !codex => {
                i += 1;
                let (key, value) = words
                    .get(i)
                    .and_then(|s| s.split_once(':'))
                    .ok_or(invalid)?;
                let key = key.trim();
                if key.is_empty()
                    || headers
                        .insert(key.to_owned(), value.trim().to_owned())
                        .is_some()
                {
                    return Err(invalid);
                }
            }
            "--bearer-token-env-var" if codex => {
                i += 1;
                let variable = words.get(i).ok_or(invalid)?;
                if !super::valid_variable(variable) || bearer.replace(variable).is_some() {
                    return Err(invalid);
                }
            }
            "--url" if codex => {
                i += 1;
                if endpoint.replace(words.get(i).ok_or(invalid)?).is_some() {
                    return Err(invalid);
                }
                transport = Some("http");
            }
            // Scope/account options cannot be silently replaced by the modal's target.
            value if value.starts_with('-') => return Err(invalid),
            value if name.is_none() => name = Some(value.to_owned()),
            _ if !codex && endpoint.is_none() => endpoint = Some(&words[i]),
            _ => return Err(invalid),
        }
        i += 1;
    }
    if !env.is_empty() || !matches!(transport, Some("http" | "sse")) {
        return Err(invalid);
    }
    let mut draft = entry(
        name.ok_or(invalid)?,
        json!({
            "type": transport.unwrap_or("http"),
            "url": endpoint.ok_or(invalid)?, "headers": headers,
        }),
    )?;
    if let Some(variable) = bearer {
        if provider == Some(Provider::Codex) {
            draft
                .extra
                .insert("bearer_token_env_var".into(), json!(variable));
        } else {
            draft
                .headers
                .insert("Authorization".into(), format!("Bearer ${{{variable}}}"));
        }
    }
    Ok(draft)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &str) -> Option<Result<McpServerDraft, ImportError>> {
        super::parse(source, None)
    }

    #[test]
    fn official_stdio_commands_become_launch_fields() {
        for cli in ["codex", "claude", "/usr/local/bin/claude", "codex.exe"] {
            let source = format!("{cli} mcp add docs --env TOKEN=value -- npx -y 'some package'");
            let draft = parse(&source).unwrap().unwrap();
            assert_eq!(draft.name, "docs");
            assert_eq!(draft.command.as_deref(), Some("npx"));
            assert_eq!(draft.args, ["-y", "some package"]);
            assert_eq!(draft.env["TOKEN"], "value");
        }
    }

    #[test]
    fn remote_registration_and_continuations_preserve_connection() {
        for source in [
            "claude mcp add --transport http docs https://example.com/mcp",
            "codex mcp add docs --url https://example.com/mcp",
            "claude mcp add --transport http \\\ndocs https://example.com/mcp",
        ] {
            let draft = parse(source).unwrap().unwrap();
            assert_eq!(draft.name, "docs");
            assert_eq!(draft.url.as_deref(), Some("https://example.com/mcp"));
            assert!(draft.command.is_none());
        }
        let draft = parse(
            "claude mcp add -t sse docs https://example.com/sse -H 'Authorization: Bearer token'",
        )
        .unwrap()
        .unwrap();
        assert_eq!(draft.headers["Authorization"], "Bearer token");
        let draft = parse(
            "codex mcp add docs --url https://example.com/mcp --bearer-token-env-var DOCS_TOKEN",
        )
        .unwrap()
        .unwrap();
        assert_eq!(draft.headers["Authorization"], "Bearer ${DOCS_TOKEN}");
    }

    #[test]
    fn unsupported_registration_never_falls_back_to_a_launch_command() {
        for source in [
            "codex mcp login docs",
            "claude mcp add docs --scope project -- npx server",
            "claude --debug mcp add docs -- npx server",
            "codex mcp add docs --url https://example.com --bearer-token-env-var bad-name",
            "env CLAUDE_CONFIG_DIR=/tmp/account claude mcp add docs -- npx server",
            "claude mcp add docs --env TOKEN=a --env TOKEN=b -- server",
            "claude mcp add -t http docs -- server",
            "claude mcp add docs -- server && echo done",
            "claude mcp add docs -- server $TOKEN",
            "codex mcp add docs --",
            "claude mcp add docs https://example.com/mcp",
        ] {
            assert_eq!(
                parse(source).unwrap().unwrap_err(),
                ImportError::RegistrationCommand,
                "{source}"
            );
        }
        assert!(parse("codex mcp-server").is_none());
        assert!(parse("npx -y server").is_none());
    }
}
