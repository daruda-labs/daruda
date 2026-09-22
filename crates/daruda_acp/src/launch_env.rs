//! Prepare a configured adapter without starting its ACP process.
//!
//! Known npm adapters retain a concrete Node runtime and an installation lease.
//! Custom launchers keep their execution path; environment stripping is shared.

use crate::PreparedAdapter;
use crate::connection::{AcpClientError, AdapterCommand, LaunchSpec};
use crate::node::{NodeProgress, command_needs_node, ensure_node_with_context};
use crate::preparation::PreparationContext;
use std::path::Path;

/// The adapter launch for `launch`: a runtime is provisioned only when the
/// command needs one, then [`LaunchSpec::strip_env`] is applied to whatever
/// shape that produced — the one strip site, so no branch can skip it.
///
/// The strip must run *after* runtime selection: the wrapper prefix it emits
/// hides the launcher token from [`command_needs_node`].
pub fn prepare_adapter_command(
    launch: &LaunchSpec,
    install_root: &Path,
    progress: &mut dyn FnMut(NodeProgress),
) -> Result<PreparedAdapter, AcpClientError> {
    prepare_adapter(
        launch,
        install_root,
        progress,
        &PreparationContext::default(),
    )
}

/// Prepare once, then retain this value across all sessions in a flow run.
pub fn prepare_adapter(
    launch: &LaunchSpec,
    install_root: &Path,
    progress: &mut dyn FnMut(NodeProgress),
    context: &PreparationContext<'_>,
) -> Result<PreparedAdapter, AcpClientError> {
    context.check()?;
    let npm_adapter = crate::npm_adapter::NpmAdapter::parse(&launch.command);
    let selected = if let Some(adapter) = npm_adapter {
        adapter.prepare(install_root, &launch.strip_env, progress, context)?
    } else if command_needs_node(&launch.command) {
        ensure_node_with_context(install_root, progress, context)?
            .wrap_command(&launch.command, install_root)
            .into()
    } else {
        AdapterCommand(launch.command.clone()).into()
    };
    context.check()?;
    Ok(selected.finalize(&launch.strip_env))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::ADAPTER_NPM_PACKAGE;
    use crate::launch_config::*;
    use crate::node::{NodeRuntime, node_platform};
    use agent_client_protocol::AcpAgent;
    use agent_client_protocol::AcpAgentConfig;
    use agent_client_protocol::schema::v1::McpServer;
    use agent_client_protocol::schema::v1::{EnvVariable, McpServerStdio};
    use std::path::PathBuf;
    use std::str::FromStr;

    /// Fixed test `install_root`, distinct from `node_dir` (which itself
    /// lives under a real one in production) so assertions can tell the two
    /// paths apart.
    fn test_install_root() -> PathBuf {
        PathBuf::from("/data/daruda/node")
    }

    /// A [`LaunchSpec`] from a command and a borrowed strip list.
    fn spec(command: &str, strip_env: &[&str]) -> LaunchSpec {
        LaunchSpec {
            command: command.to_string(),
            strip_env: strip_env.iter().map(|s| (*s).to_string()).collect(),
        }
    }

    /// A [`LaunchSpec`] with nothing to strip — the System-account path.
    fn plain(command: &str) -> LaunchSpec {
        spec(command, &[])
    }

    /// The two steps [`prepare_adapter_command`] performs once a runtime is
    /// selected, so a per-runtime assertion sees the same final command a
    /// real launch would.
    fn wrap_and_strip(
        runtime: &NodeRuntime,
        launch: &LaunchSpec,
        install_root: &Path,
    ) -> AdapterCommand {
        finalize_command(
            runtime.wrap_command(&launch.command, install_root),
            &launch.strip_env,
        )
    }

    /// [`prepare_adapter_command`] with a no-op progress sink.
    fn prepared(launch: &LaunchSpec, install_root: &Path) -> AdapterCommand {
        prepare_adapter_command(launch, install_root, &mut |_| {})
            .expect("no runtime needed")
            .command()
    }

    /// A JSON config config in the agent-registry `distribution` shape — the
    /// self-contained form a user's `[[agents]]` entry can supply instead of a
    /// bash-style command. Deliberately *not* the SDK's own shape: this is the
    /// external format [`parse_json_launch`] has to keep accepting.
    fn json_stdio(command: &str, args: &[&str]) -> String {
        serde_json::to_string(&McpServer::Stdio(
            McpServerStdio::new("acp-agent", command)
                .args(args.iter().map(|a| (*a).to_string()).collect())
                .env(vec![EnvVariable::new("EXISTING", "1")]),
        ))
        .expect("config config serializes")
    }

    /// The same launch in the SDK's own shape (object `env`, no `name`/`type`).
    fn json_sdk(command: &str, args: &[&str]) -> String {
        serde_json::to_string(
            &AcpAgentConfig::new(command)
                .args(args.iter().map(|a| (*a).to_string()))
                .env("EXISTING", "1"),
        )
        .expect("agent config serializes")
    }

    /// The launch config a finalized command parses back into — through
    /// `AcpAgent::from_str`, so every assertion below is pinned to what the SDK
    /// actually accepts rather than to our own re-parse.
    fn config_of(command: &AdapterCommand) -> AcpAgentConfig {
        AcpAgent::from_str(&command.0)
            .expect("wrapped command parses")
            .into_config()
    }

    #[test]
    fn system_runtime_strips_auth_override_env_before_the_command() {
        let install_root = test_install_root();
        let cmd = format!("npx -y {ADAPTER_NPM_PACKAGE}");
        let wrapped = wrap_and_strip(
            &NodeRuntime::System,
            &spec(&cmd, &["ANTHROPIC_API_KEY", "CLAUDE_CODE_OAUTH_TOKEN"]),
            &install_root,
        );

        // One `-u` per var, ahead of every `NAME=value` operand: `env` stops
        // option parsing at the first operand.
        assert!(
            wrapped.0.starts_with(&format!(
                "{} -u ANTHROPIC_API_KEY -u CLAUDE_CODE_OAUTH_TOKEN npm_config_cpu=",
                crate::launch_config::env_prefix_tokens().join(" ")
            )),
            "{}",
            wrapped.0
        );
        assert!(wrapped.0.ends_with(&cmd), "{}", wrapped.0);

        let config = config_of(&wrapped);
        assert_eq!(config.command(), env_program());
        let unsets = crate::launch_config::env_argv([
            "-u",
            "ANTHROPIC_API_KEY",
            "-u",
            "CLAUDE_CODE_OAUTH_TOKEN",
        ]);
        assert_eq!(&config.arguments()[..unsets.len()], unsets);
        assert_eq!(
            &config.arguments()[config.arguments().len() - 3..],
            ["npx", "-y", ADAPTER_NPM_PACKAGE]
        );
    }

    #[test]
    fn system_runtime_with_an_empty_strip_is_byte_identical() {
        // Pins the majority path (System account / Codex) as untouched: no
        // `/usr/bin/env`, no `-u`, exactly the arch+cache prefix as before.
        let cmd = "npx -y @agentclientprotocol/claude-agent-acp@latest";
        let (os, arch) = node_platform().expect("supported test platform");
        let install_root = test_install_root();
        let expected = format!(
            "npm_config_cpu={arch} npm_config_os={os} npm_config_cache={} {cmd}",
            shell_words::quote(&install_root.join("npx-cache").to_string_lossy())
        );
        assert_eq!(
            wrap_and_strip(&NodeRuntime::System, &plain(cmd), &install_root).0,
            expected
        );
    }

    #[test]
    fn managed_runtime_strips_auth_override_env_via_an_explicit_env_argv() {
        let node_dir = PathBuf::from("/data/daruda/node/node-v24.11.0-darwin-arm64");
        let install_root = test_install_root();
        let cmd = format!("npx -y {ADAPTER_NPM_PACKAGE}");
        let command = wrap_and_strip(
            &NodeRuntime::Managed {
                node_dir: node_dir.clone(),
            },
            &spec(&cmd, &["ANTHROPIC_API_KEY", "AWS_BEARER_TOKEN_BEDROCK"]),
            &install_root,
        );

        let config = config_of(&command);
        assert_eq!(config.command(), env_program());
        assert_eq!(
            config.arguments(),
            crate::launch_config::env_argv(vec![
                "-u".to_string(),
                "ANTHROPIC_API_KEY".to_string(),
                "-u".to_string(),
                "AWS_BEARER_TOKEN_BEDROCK".to_string(),
                crate::node::managed_launcher(&node_dir, "npx")
                    .to_string_lossy()
                    .into_owned(),
                "-y".to_string(),
                ADAPTER_NPM_PACKAGE.to_string(),
            ])
        );
        // The env list is applied by the downstream spawner via `Command::env`
        // on the `env` process and inherited by the launcher — unaffected.
        assert!(config.environment().contains_key("PATH"));
        assert_eq!(
            config
                .environment()
                .get("npm_config_cache")
                .map(String::as_str),
            Some(&*install_root.join("npx-cache").to_string_lossy())
        );
    }

    #[test]
    fn managed_runtime_with_an_empty_strip_is_byte_identical() {
        let node_dir = PathBuf::from("/data/daruda/node/node-v24.11.0-darwin-arm64");
        let cmd = format!("npx -y {ADAPTER_NPM_PACKAGE}");
        let command = wrap_and_strip(
            &NodeRuntime::Managed {
                node_dir: node_dir.clone(),
            },
            &plain(&cmd),
            &test_install_root(),
        );

        assert!(!command.0.contains("/usr/bin/env"), "{}", command.0);
        assert!(!command.0.contains("\"-u\""), "{}", command.0);
        let config = config_of(&command);
        assert_eq!(
            config.command(),
            crate::node::managed_launcher(&node_dir, "npx")
        );
        assert_eq!(config.arguments(), vec!["-y", ADAPTER_NPM_PACKAGE]);
    }

    #[test]
    fn the_strip_prefix_lands_after_node_detection_not_before() {
        // The whole design hinges on this: an `/usr/bin/env` prefix hides the
        // `npx` launcher from `command_needs_node`, so the managed runtime
        // would never be provisioned if the strip were applied earlier.
        let launch = spec("npx -y pkg", &["ANTHROPIC_API_KEY"]);
        assert!(command_needs_node(&launch.command));
        let wrapped = wrap_and_strip(&NodeRuntime::System, &launch, &test_install_root());
        assert!(
            !command_needs_node(&wrapped.0),
            "the wrapped form is deliberately opaque to node detection: {}",
            wrapped.0
        );
    }

    #[test]
    fn a_local_adapter_binary_still_gets_the_env_strip() {
        // A `[[agents]]` entry pointing at an installed adapter binary needs
        // no Node.js, so it skips `wrap_command` entirely — the strip still
        // has to reach it, or an exported API key beats the account's OAuth.
        let launch = spec(
            "/usr/local/bin/claude-agent-acp --acp",
            &["ANTHROPIC_API_KEY", "CLAUDE_CODE_OAUTH_TOKEN"],
        );
        let prepared = prepared(&launch, &test_install_root());
        assert_eq!(
            prepared.0,
            format!(
                "{} -u ANTHROPIC_API_KEY -u CLAUDE_CODE_OAUTH_TOKEN /usr/local/bin/claude-agent-acp --acp",
                crate::launch_config::env_prefix_tokens().join(" ")
            )
        );

        let config = config_of(&prepared);
        assert_eq!(config.command(), env_program());
        assert_eq!(
            config.arguments(),
            crate::launch_config::env_argv([
                "-u",
                "ANTHROPIC_API_KEY",
                "-u",
                "CLAUDE_CODE_OAUTH_TOKEN",
                "/usr/local/bin/claude-agent-acp",
                "--acp",
            ])
        );
    }

    #[test]
    fn a_json_stdio_config_still_gets_the_env_strip() {
        // The other no-Node shape: a self-contained JSON transport has no
        // shell to hold an `/usr/bin/env` prefix, so the unsets become argv.
        let command = json_stdio("/usr/local/bin/claude-agent-acp", &["--acp"]);
        let launch = spec(&command, &["ANTHROPIC_API_KEY"]);
        let config = config_of(&prepared(&launch, &test_install_root()));

        assert_eq!(config.command(), env_program());
        assert_eq!(
            config.arguments(),
            crate::launch_config::env_argv([
                "-u",
                "ANTHROPIC_API_KEY",
                "/usr/local/bin/claude-agent-acp",
                "--acp",
            ])
        );
        // The config's own env list is untouched — only the argv is rewritten.
        assert_eq!(
            config.environment().get("EXISTING").map(String::as_str),
            Some("1")
        );
    }

    #[test]
    fn both_accepted_json_shapes_normalize_to_the_same_launch() {
        // The registry `distribution` shape and the SDK's own shape describe
        // the same launch, so they must finalize identically — this is what
        // keeps an existing `[[agents]]` JSON config working under SDK 2.0,
        // whose `from_str` accepts only the latter.
        let args = ["--acp"];
        let root = test_install_root();
        let registry = prepared(
            &plain(&json_stdio("/usr/local/bin/claude-agent-acp", &args)),
            &root,
        );
        let sdk = prepared(
            &plain(&json_sdk("/usr/local/bin/claude-agent-acp", &args)),
            &root,
        );
        assert_eq!(registry.0, sdk.0);

        let config = config_of(&registry);
        assert_eq!(
            config.command(),
            Path::new("/usr/local/bin/claude-agent-acp")
        );
        assert_eq!(config.arguments(), ["--acp"]);
        assert_eq!(
            config.environment().get("EXISTING").map(String::as_str),
            Some("1")
        );
    }

    #[test]
    fn a_non_stdio_json_transport_passes_through() {
        // No local child to inherit anything, so there is nothing to strip.
        let command = r#"{"type":"http","name":"remote","url":"https://example.test/acp"}"#;
        let launch = spec(command, &["ANTHROPIC_API_KEY"]);
        assert_eq!(prepared(&launch, &test_install_root()).0, command);
    }

    /// Every shape a launch can arrive at [`finalize_command`] in: both
    /// runtimes' node rewrites, plus the no-Node pass-through in its bash and
    /// JSON forms. Structural coverage for "the strip is applied once, after
    /// runtime selection, whatever the branch produced".
    fn every_produced_shape() -> Vec<AdapterCommand> {
        let root = test_install_root();
        let node_dir = PathBuf::from("/data/daruda/node/node-v24.11.0-darwin-arm64");
        let local_binary = "/usr/local/bin/claude-agent-acp --acp";
        vec![
            NodeRuntime::System.wrap_command("npx -y pkg", &root),
            NodeRuntime::System.wrap_command(local_binary, &root),
            NodeRuntime::Managed {
                node_dir: node_dir.clone(),
            }
            .wrap_command("npx -y pkg", &root),
            NodeRuntime::Managed { node_dir }.wrap_command(local_binary, &root),
            AdapterCommand(local_binary.to_string()),
            AdapterCommand(json_stdio("/usr/local/bin/claude-agent-acp", &["--acp"])),
        ]
    }

    #[test]
    fn the_strip_reaches_every_shape_a_branch_can_produce() {
        let strip = ["ANTHROPIC_API_KEY".to_string()];
        for produced in every_produced_shape() {
            let stripped = finalize_command(produced, &strip);
            let config = config_of(&stripped);
            assert_eq!(
                config.command(),
                env_program(),
                "unstripped launch: {}",
                stripped.0
            );
            let expected = crate::launch_config::env_argv(["-u", "ANTHROPIC_API_KEY"]);
            assert_eq!(&config.arguments()[..expected.len()], expected);
        }
    }

    /// Run `config` for real with `ANTHROPIC_API_KEY` set on the child, and
    /// report whether it exited 0 — i.e. whether the var was removed.
    #[cfg(unix)]
    fn probe_sees_no_key(config: &AcpAgentConfig) -> bool {
        std::process::Command::new(config.command())
            .args(config.arguments())
            .env("ANTHROPIC_API_KEY", "leaked")
            .status()
            .expect("probe spawns")
            .success()
    }

    /// Unix only, because the wrapper is `/usr/bin/env` there and this test
    /// binary is what `env_program` names anywhere else — and a test harness
    /// does not route `--env`. The daruda wrapper's own behaviour is spawned
    /// for real in `crates/app/tests/env_strip.rs`.
    #[cfg(unix)]
    #[test]
    fn the_emitted_unsets_really_remove_the_var_from_a_spawned_child() {
        // `env(1)`'s argv grammar is only checked at spawn time: a `-u` placed
        // after a `KEY=value` operand is taken as the utility to run, which no
        // string assertion catches. Both emitted shapes are executed here.
        let root = test_install_root();
        let strip = ["ANTHROPIC_API_KEY"];

        // Bash-string shape, with a leading assignment the `-u` must precede.
        let probe_args = ["--absent-env", "ANTHROPIC_API_KEY"];
        let probe = test_process::command_line(&probe_args);
        let bash = spec(&format!("KEEP=1 {probe}"), &strip);
        assert!(probe_sees_no_key(&config_of(&prepared(&bash, &root))));

        // JSON config shape — the unsets are argv entries, not a shell prefix.
        let program = test_process::executable().to_str().unwrap();
        let json = spec(&json_stdio(program, &probe_args), &strip);
        assert!(probe_sees_no_key(&config_of(&prepared(&json, &root))));

        // Control: without a strip the probe really does see the var, so the
        // two assertions above are testing the unsets and not a dud probe.
        let unstripped = spec(&json_stdio(program, &probe_args), &[]);
        assert!(!probe_sees_no_key(&config_of(&prepared(
            &unstripped,
            &root
        ))));
    }

    #[test]
    fn an_empty_strip_is_byte_identical_on_every_bash_shape() {
        for produced in every_produced_shape() {
            if produced.0.trim_start().starts_with(JSON_LAUNCH_PREFIX) {
                continue;
            }
            let before = produced.0.clone();
            assert_eq!(finalize_command(produced, &[]).0, before);
        }
    }

    #[test]
    fn an_empty_strip_still_normalizes_a_json_shape() {
        // The one deliberate non-identity: SDK 2.0's `from_str` rejects the
        // registry shape outright, so it is rewritten even with nothing to
        // strip. What must survive the rewrite is the launch it describes.
        let registry = AdapterCommand(json_stdio("/usr/local/bin/claude-agent-acp", &["--acp"]));
        let config = config_of(&finalize_command(registry, &[]));
        assert_eq!(
            config.command(),
            Path::new("/usr/local/bin/claude-agent-acp")
        );
        assert_eq!(config.arguments(), ["--acp"]);
        assert_eq!(
            config.environment().get("EXISTING").map(String::as_str),
            Some("1")
        );
    }

    #[test]
    fn system_runtime_strips_alongside_the_commands_own_env_prefix() {
        let launch = spec(
            "AUGMENT_DISABLE_AUTO_UPDATE=1 npx -y @augmentcode/auggie@0.32.0 --acp",
            &["ANTHROPIC_API_KEY"],
        );
        let wrapped = wrap_and_strip(&NodeRuntime::System, &launch, &test_install_root());
        let config = config_of(&wrapped);
        assert_eq!(config.command(), env_program());

        let unset_at = config
            .arguments()
            .iter()
            .position(|a| a == "ANTHROPIC_API_KEY")
            .expect("strip var present");
        let own_at = config
            .arguments()
            .iter()
            .position(|a| a == "AUGMENT_DISABLE_AUTO_UPDATE=1")
            .expect("the command's own assignment survives");
        assert!(unset_at < own_at, "`-u` must precede every assignment");
        assert!(config.arguments().iter().any(|a| a == "npx"));
    }

    #[test]
    fn managed_runtime_strips_alongside_the_commands_own_env_prefix() {
        let node_dir = PathBuf::from("/data/daruda/node/node-v24.11.0-darwin-arm64");
        let launch = spec(
            "AUGMENT_DISABLE_AUTO_UPDATE=1 npx -y @augmentcode/auggie@0.32.0 --acp",
            &["ANTHROPIC_API_KEY"],
        );
        let command = wrap_and_strip(
            &NodeRuntime::Managed {
                node_dir: node_dir.clone(),
            },
            &launch,
            &test_install_root(),
        );

        let config = config_of(&command);
        assert_eq!(config.command(), env_program());
        assert_eq!(
            config.arguments(),
            crate::launch_config::env_argv(vec![
                "-u".to_string(),
                "ANTHROPIC_API_KEY".to_string(),
                crate::node::managed_launcher(&node_dir, "npx")
                    .to_string_lossy()
                    .into_owned(),
                "-y".to_string(),
                "@augmentcode/auggie@0.32.0".to_string(),
                "--acp".to_string(),
            ])
        );
        // The command's own assignment stays in the env list, not the argv.
        assert_eq!(
            config
                .environment()
                .get("AUGMENT_DISABLE_AUTO_UPDATE")
                .map(String::as_str),
            Some("1")
        );
        assert!(config.environment().contains_key("PATH"));
    }
}
