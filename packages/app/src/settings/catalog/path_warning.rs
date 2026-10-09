//! Advisory executable lookup, evaluated on edits rather than during rendering.

/// The token [`agent_command_path_warning`] should look up on `PATH`, or
/// `None` when `command` means no local-PATH check applies.
///
/// No check applies when: the command is a JSON stdio config (self-contained,
/// no executable name to look up — same discrimination `daruda_config`'s
/// `AgentLaunch` uses to gate its own shell-string edits); or the first real
/// token (after stripping any `NAME=value` env-prefix assignments) is
/// `npx`/`uvx` — daruda provisions Node.js itself, and `uvx` resolves its own
/// Python venvs, so neither names a binary the user is expected to have
/// installed locally. Transport (ssh/docker exempts the whole row) is not
/// considered here — that suppression needs no `which` call, so it is applied
/// at render time instead (`sections::agent_catalog::render_agent_catalog_row`).
pub(in crate::settings) fn path_check_token(command: &str) -> Option<String> {
    let trimmed = command.trim();
    if trimmed.is_empty() || trimmed.starts_with('{') {
        return None;
    }
    let token = daruda_acp::node::first_command_token(trimmed)?;
    (!matches!(token.as_str(), "npx" | "uvx")).then_some(token)
}

/// The catalog row's local-PATH warning: `Some(token)` when
/// [`path_check_token`] says a check applies and that token is not found on
/// `PATH`, `None` otherwise (no check applies, or the binary was found).
/// Advisory only — a missing command never blocks
/// `SettingsView::validate`, since registering an agent before
/// installing its CLI (or before adding it to `PATH`) is a legitimate flow.
pub(in crate::settings) fn agent_command_path_warning(command: &str) -> Option<String> {
    let token = path_check_token(command)?;
    which::which(&token).is_err().then_some(token)
}

#[cfg(test)]
mod tests;
