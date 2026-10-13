//! Launch-command inspection: env-prefix-aware tokenizing, the Node-launcher
//! check, and the value-free diagnostic form of a command.

/// `true` when launching `command` needs a Node.js runtime provisioned.
///
/// A JSON stdio config (trimmed string starts with `{`) is a self-contained
/// transport that names its own executable, so it needs nothing. Any other
/// binary is assumed already runnable. Only a leading `npx` / `node` token —
/// the Node-family launchers — triggers provisioning.
#[must_use]
pub fn command_needs_node(command: &str) -> bool {
    let trimmed = command.trim_start();
    if trimmed.starts_with('{') {
        return false;
    }
    matches!(
        launcher_after_env_prefix(trimmed).as_deref(),
        Some("npx" | "node")
    )
}

/// The command's first real token — the executable name, after stripping any
/// leading `NAME=value` env-prefix assignments (see `daruda_config`'s
/// `AgentLaunch::wrap_with_env`). Exposed for the Settings window's
/// local-PATH check, which needs the same env-prefix-aware tokenization
/// [`command_needs_node`] uses but tests the token against a different set
/// (`npx`/`uvx`) than the Node-only launcher check.
#[must_use]
pub fn first_command_token(command: &str) -> Option<String> {
    launcher_after_env_prefix(command)
}

/// A launch command reduced to what a spawn failure needs to be diagnosed:
/// the launcher that actually gets exec'd, plus the names of the variables
/// injected ahead of it.
///
/// Values never appear. A declared environment carries whatever the user
/// typed into Settings — an API key, a company token — so neither the shell
/// text nor a JSON config's own `env` map may reach a log. The remote forms
/// keep their assignments inside a quoted `sh -c` script, so only the
/// transport token (`ssh` / `docker`) is echoed there.
#[must_use]
pub fn command_diagnostic(command: &str) -> String {
    let trimmed = command.trim_start();
    if trimmed.starts_with('{') {
        return JSON_STDIO_DIAGNOSTIC.to_string();
    }
    let (assignments, tokens) = split_env_prefixed_tokens(trimmed);
    let launcher = tokens
        .first()
        .map_or(NO_LAUNCHER_DIAGNOSTIC, String::as_str);
    if assignments.is_empty() {
        return launcher.to_string();
    }
    let names: Vec<&str> = assignments.iter().map(|(name, _)| name.as_str()).collect();
    format!("{launcher} (env: {})", names.join(", "))
}

/// Stand-in for a JSON stdio config in [`command_diagnostic`]: that shape
/// carries command, args and env as structured fields, and the env half is
/// exactly what must not be logged.
const JSON_STDIO_DIAGNOSTIC: &str = "<json stdio config>";

/// Stand-in in [`command_diagnostic`] for a command that tokenized to nothing.
const NO_LAUNCHER_DIAGNOSTIC: &str = "<empty>";

fn launcher_after_env_prefix(command: &str) -> Option<String> {
    let (_, tokens) = split_env_prefixed_tokens(command);
    tokens.into_iter().next()
}

/// Split `command` into its leading `NAME=value` env-prefix assignments and
/// the remaining command tokens.
///
/// Tokenizes with [`shell_words::split`] (quote-aware) rather than
/// [`str::split_whitespace`] so a quoted assignment value with an embedded
/// space — e.g. `CLAUDE_CONFIG_DIR='/Users/x/Library/Application
/// Support/daruda/acc/alice'`, produced by `daruda_config`'s
/// `AgentLaunch::wrap_with_env` (`Raw` branch) — is tokenized as a single
/// word and parsed as one assignment with its value's quotes stripped,
/// instead of exploding into two command tokens partway through the path.
/// This mirrors how the eventual consumer, `AcpAgent::from_str`, re-tokenizes
/// the same string (see [`prefix_with_host_arch_env`]'s doc comment) — both
/// must agree on where the env prefix ends and the command begins. An
/// unquoted `NAME=value` token still parses exactly as before, since
/// `shell_words::split` treats it as one plain word when it contains no
/// whitespace to begin with.
///
/// Falls back to a plain whitespace split on a `shell_words` parse error
/// (e.g. an unmatched quote in a hand-written command) so a malformed string
/// still yields *some* tokens instead of dropping the command entirely.
pub(super) fn split_env_prefixed_tokens(command: &str) -> (Vec<(String, String)>, Vec<String>) {
    let words = shell_words::split(command)
        .unwrap_or_else(|_| command.split_whitespace().map(str::to_string).collect());

    let mut env = Vec::new();
    let mut command_tokens = Vec::new();
    let mut saw_command = false;

    for word in words {
        if !saw_command && let Some((name, value)) = parse_env_assignment(&word) {
            env.push((name.to_string(), value.to_string()));
            continue;
        }
        saw_command = true;
        command_tokens.push(word);
    }

    (env, command_tokens)
}

fn parse_env_assignment(token: &str) -> Option<(&str, &str)> {
    let eq_pos = token.find('=')?;
    if eq_pos == 0 {
        return None;
    }

    let name = &token[..eq_pos];
    let value = &token[eq_pos + 1..];

    let mut chars = name.chars();
    let first = chars.next()?;
    if !first.is_ascii_alphabetic() && first != '_' {
        return None;
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }

    Some((name, value))
}

#[cfg(test)]
mod tests;
