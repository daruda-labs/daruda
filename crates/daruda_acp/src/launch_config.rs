//! Parse external launch formats and apply environment removal exactly once.

use crate::connection::AdapterCommand;
use agent_client_protocol::AcpAgentConfig;
use agent_client_protocol::schema::v1::McpServer;
use std::path::PathBuf;

/// `env(1)`, the only portable way to *remove* a variable from a spawned
/// process's environment. Absolute on both supported targets (macOS, Linux).
pub(crate) const ENV_BIN: &str = "/usr/bin/env";

pub(crate) fn env_program() -> PathBuf {
    // WORKAROUND: the ACP SDK cannot remove inherited environment entries.
    // Git for Windows supplies env(1) until the SDK exposes that capability.
    if cfg!(windows) {
        daruda_core::shell::posix_tool("env")
    } else {
        ENV_BIN.into()
    }
}

/// `env(1)`'s remove-a-variable flag.
pub(crate) const ENV_UNSET_FLAG: &str = "-u";

/// Leading character marking an adapter command as a JSON launch config rather
/// than a shell command line — the same discrimination `AcpAgent::from_str`
/// makes to pick its parse.
pub(crate) const JSON_LAUNCH_PREFIX: char = '{';

/// The last rewrite before spawn: remove `strip_env`, and put a JSON command
/// into the one shape `AcpAgent::from_str` accepts.
///
/// A bash-style command only gets an [`ENV_BIN`] prefix, and only when there is
/// something to strip — an empty `strip_env` returns it byte-identical.
///
/// A JSON command is normalized **even with nothing to strip**: `from_str`
/// deserializes JSON as [`AcpAgentConfig`] (object `env`, `deny_unknown_fields`),
/// so the registry `distribution` shape daruda accepts — see
/// [`parse_json_launch`] — has to be translated here or the adapter never
/// launches. Unparseable JSON passes through so the SDK owns the error message.
pub(crate) fn finalize_command(adapter: AdapterCommand, strip_env: &[String]) -> AdapterCommand {
    let trimmed = adapter.0.trim_start();
    if !trimmed.starts_with(JSON_LAUNCH_PREFIX) {
        if strip_env.is_empty() {
            return adapter;
        }
        return AdapterCommand(prefix_with_env_unsets(&adapter.0, strip_env));
    }
    let Some(config) = parse_json_launch(trimmed) else {
        return adapter;
    };
    let normalized = finalize_config(config, strip_env);
    AdapterCommand(serde_json::to_string(&normalized).expect("AcpAgentConfig serializes to JSON"))
}

pub(crate) fn finalize_config(config: AcpAgentConfig, strip_env: &[String]) -> AcpAgentConfig {
    // The JSON form has no shell to hold an `/usr/bin/env` prefix, so the
    // unsets have to be real argv entries. `env`'s own map only *sets* vars;
    // removing an inherited one still needs `-u`.
    let (spawn_command, spawn_args) = with_env_unsets_argv(
        config.command().to_path_buf(),
        config.arguments().to_vec(),
        strip_env,
    );
    AcpAgentConfig::new(spawn_command)
        .args(spawn_args)
        .envs(config.environment().clone())
}

/// The launch config a JSON adapter command describes, in the SDK's own shape.
///
/// Two forms are accepted. [`AcpAgentConfig`] itself (`env` as an object) is
/// what the SDK emits and parses. The agent-registry `distribution` shape
/// (`{"type":"stdio","name":..,"env":[{"name":..,"value":..}]}`) is what agent
/// registries publish and what daruda documents in `[[agents]]`; it is an
/// external format that does not track the Rust SDK, so daruda owns the
/// translation rather than pushing the churn onto users' configs.
///
/// `None` for a non-stdio transport (HTTP/SSE — no local child spawns there)
/// and for JSON matching neither form.
pub(crate) fn parse_json_launch(json: &str) -> Option<AcpAgentConfig> {
    if let Ok(config) = serde_json::from_str::<AcpAgentConfig>(json) {
        return Some(config);
    }
    match serde_json::from_str::<McpServer>(json) {
        Ok(McpServer::Stdio(stdio)) => Some(
            AcpAgentConfig::new(stdio.command)
                .args(stdio.args)
                .envs(stdio.env.into_iter().map(|e| (e.name, e.value))),
        ),
        _ => None,
    }
}

/// `-u NAME` pairs for `strip_env`, in order — [`ENV_BIN`]'s argv form of
/// "remove this variable". Empty when nothing is stripped.
pub(crate) fn env_unset_args(strip_env: &[String]) -> Vec<String> {
    strip_env
        .iter()
        .flat_map(|name| [ENV_UNSET_FLAG.to_string(), name.clone()])
        .collect()
}

/// Prefix a bash-style `command` with `/usr/bin/env -u NAME …`, or return it
/// unchanged when `strip_env` is empty.
///
/// The `-u` flags go ahead of `command` — including its leading `NAME=value`
/// assignments — because `env` stops option parsing at its first operand, so
/// a `-u` placed after an assignment is taken as the utility to run. Var
/// names need no quoting: `node`'s env-assignment parser only ever accepts
/// `[A-Za-z_][A-Za-z0-9_]*`.
pub(crate) fn prefix_with_env_unsets(command: &str, strip_env: &[String]) -> String {
    if strip_env.is_empty() {
        return command.to_string();
    }
    format!(
        "{} {} {command}",
        shell_words::quote(&env_program().to_string_lossy()),
        env_unset_args(strip_env).join(" ")
    )
}

/// The `(command, args)` to spawn so `launcher` runs with `strip_env`
/// removed. Unchanged when `strip_env` is empty; otherwise [`ENV_BIN`] takes
/// over as the executable and `launcher` moves into its argv.
///
/// The JSON stdio form has no shell to hold an `/usr/bin/env` prefix, so the
/// unsets have to be real argv entries.
pub(crate) fn with_env_unsets_argv(
    launcher: PathBuf,
    args: Vec<String>,
    strip_env: &[String],
) -> (PathBuf, Vec<String>) {
    if strip_env.is_empty() {
        return (launcher, args);
    }
    let mut argv = env_unset_args(strip_env);
    argv.push(launcher.to_string_lossy().into_owned());
    argv.extend(args);
    (env_program(), argv)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sdk_and_registry_stdio_share_one_normalization() {
        for json in [
            r#"{"command":"node","args":["entry.js"],"env":{"TEST":"value"}}"#,
            r#"{"type":"stdio","name":"adapter","command":"node","args":["entry.js"],"env":[{"name":"TEST","value":"value"}]}"#,
        ] {
            let config = parse_json_launch(json).unwrap();
            let prepared = finalize_config(config, &["SECRET".into()]);
            assert_eq!(prepared.command(), env_program());
            assert_eq!(prepared.arguments(), ["-u", "SECRET", "node", "entry.js"]);
            assert_eq!(prepared.environment()["TEST"], "value");
        }
    }
}
