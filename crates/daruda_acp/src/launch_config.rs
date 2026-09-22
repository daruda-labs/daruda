//! Parse external launch formats and apply environment removal exactly once.

use crate::connection::AdapterCommand;
use agent_client_protocol::AcpAgentConfig;
use agent_client_protocol::schema::v1::McpServer;
use std::path::PathBuf;

/// `env(1)`, the only portable way to *remove* a variable from a spawned
/// process's environment. Absolute on both supported targets (macOS, Linux).
pub(crate) const ENV_BIN: &str = "/usr/bin/env";

/// The program that removes `strip_env` from a launch, plus whatever it needs
/// before `env(1)`'s own `-u` grammar starts.
///
/// WORKAROUND: the SDK only adds to a child's environment, so removing one
/// takes a wrapper. Lifts when it can remove an inherited entry itself.
/// Windows has no `env(1)` without Git for Windows, so daruda wraps itself.
pub(crate) fn env_launcher() -> (PathBuf, Vec<String>) {
    env_launcher_for(cfg!(windows), std::env::current_exe().ok())
}

/// [`env_launcher`] with the host as a value, so the Windows answer is
/// checked from a macOS run rather than only by CI. A host that cannot name
/// its own binary keeps the plain name — `PATH` is what is left, and failing
/// here would take a launch down over a lookup.
fn env_launcher_for(windows: bool, own_exe: Option<PathBuf>) -> (PathBuf, Vec<String>) {
    if windows {
        let daruda = own_exe.unwrap_or_else(|| PathBuf::from("daruda"));
        (daruda, vec![DARUDA_ENV_SUBCOMMAND.to_owned()])
    } else {
        (ENV_BIN.into(), Vec::new())
    }
}

/// The app subcommand that stands in for `env(1)`. Spelled here because this
/// is where the command line that uses it is built; the app routes it.
pub(crate) const DARUDA_ENV_SUBCOMMAND: &str = "--env";

#[cfg(test)]
pub(crate) fn env_program() -> PathBuf {
    env_launcher().0
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
/// A bash-style command only gets an [`env_launcher`] prefix, and only when
/// something is stripped — an empty `strip_env` returns it byte-identical.
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

/// Prefix a bash-style `command` with [`env_launcher`] and `-u NAME …`, or
/// return it unchanged when `strip_env` is empty.
///
/// The `-u` flags go ahead of `command` — including its leading `NAME=value`
/// assignments — because option parsing stops at the first operand, so a `-u`
/// after an assignment is taken as the utility to run. Names need no quoting:
/// `node` accepts only `[A-Za-z_][A-Za-z0-9_]*` there.
pub(crate) fn prefix_with_env_unsets(command: &str, strip_env: &[String]) -> String {
    if strip_env.is_empty() {
        return command.to_string();
    }
    let (program, leading) = env_launcher();
    prefixed(&program, &leading, command, strip_env)
}

/// The prefixed line, with the host's answer already in hand — so a Windows
/// program path, which routinely carries a space, is checked from anywhere.
fn prefixed(
    program: &std::path::Path,
    leading: &[String],
    command: &str,
    strip_env: &[String],
) -> String {
    let mut argv = vec![shell_words::quote(&program.to_string_lossy()).into_owned()];
    argv.extend(leading.iter().cloned());
    argv.extend(env_unset_args(strip_env));
    format!("{} {command}", argv.join(" "))
}

/// The `(command, args)` to spawn so `launcher` runs with `strip_env`
/// removed. Unchanged when `strip_env` is empty; otherwise the wrapper takes
/// over as the executable and `launcher` moves into its argv — the JSON stdio
/// form has no shell to hold a prefix.
pub(crate) fn with_env_unsets_argv(
    launcher: PathBuf,
    args: Vec<String>,
    strip_env: &[String],
) -> (PathBuf, Vec<String>) {
    if strip_env.is_empty() {
        return (launcher, args);
    }
    let (program, mut argv) = env_launcher();
    argv.extend(env_unset_args(strip_env));
    argv.push(launcher.to_string_lossy().into_owned());
    argv.extend(args);
    (program, argv)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The wrapper Windows uses is daruda itself, reached through its own
    /// path — not a name `PATH` has to resolve while the app is mid-launch.
    #[test]
    fn windows_wraps_with_darudas_own_subcommand() {
        let exe = PathBuf::from(r"C:\Program Files\daruda\daruda.exe");
        let (program, leading) = env_launcher_for(true, Some(exe.clone()));

        assert_eq!(program, exe);
        assert_eq!(leading, [DARUDA_ENV_SUBCOMMAND]);
    }

    #[test]
    fn unix_wraps_with_the_system_env() {
        let (program, leading) = env_launcher_for(false, Some(PathBuf::from("/anything")));

        assert_eq!(program, PathBuf::from(ENV_BIN));
        assert!(leading.is_empty(), "env(1) needs nothing before its own -u");
    }

    /// A host that cannot name its own binary still launches: the bare name
    /// is something `PATH` can answer, where a hard failure answers nothing.
    #[test]
    fn a_nameless_executable_falls_back_to_the_bare_name() {
        let (program, _) = env_launcher_for(true, None);

        assert_eq!(program, PathBuf::from("daruda"));
    }

    /// `C:\Program Files` is where an installer puts daruda, and the line
    /// built here is re-split by `AcpAgent::from_str`. A path that does not
    /// survive that round trip launches half a program.
    #[test]
    fn a_windows_program_path_with_a_space_survives_the_round_trip() {
        let exe = r"C:\Program Files\daruda\daruda.exe";
        let (program, leading) = env_launcher_for(true, Some(PathBuf::from(exe)));

        let line = prefixed(
            &program,
            &leading,
            "npx -y pkg",
            &["ANTHROPIC_API_KEY".to_owned()],
        );

        assert_eq!(
            shell_words::split(&line).expect("the SDK re-splits this line"),
            [
                exe,
                DARUDA_ENV_SUBCOMMAND,
                "-u",
                "ANTHROPIC_API_KEY",
                "npx",
                "-y",
                "pkg"
            ]
        );
    }

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
