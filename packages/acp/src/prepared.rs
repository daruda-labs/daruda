//! Prepared launch data stays pinned until every session using it has ended.

use crate::connection::{AcpClientError, AdapterCommand};
use crate::npm_adapter::cache::InstallationLease;
use agent_client_protocol::{AcpAgent, AcpAgentConfig};
use std::collections::BTreeMap;
use std::path::Path;
use std::str::FromStr;

#[derive(Clone)]
pub struct PreparedAdapter {
    launch: Launch,
}

#[derive(Clone)]
enum Launch {
    External(AdapterCommand),
    Installed {
        config: AcpAgentConfig,
        lease: InstallationLease,
    },
}

impl PreparedAdapter {
    pub(crate) fn installed(config: AcpAgentConfig, lease: InstallationLease) -> Self {
        Self {
            launch: Launch::Installed { config, lease },
        }
    }

    pub(crate) fn finalize(self, strip: &[String]) -> Self {
        let launch = match self.launch {
            Launch::External(command) => {
                Launch::External(crate::launch_config::finalize_command(command, strip))
            }
            Launch::Installed { config, lease } => Launch::Installed {
                config: crate::launch_config::finalize_config(config, strip),
                lease,
            },
        };
        Self { launch }
    }

    /// Compatibility serialization; keep this owner alive while using the command.
    pub fn command(&self) -> AdapterCommand {
        match &self.launch {
            Launch::External(command) => command.clone(),
            Launch::Installed { config, .. } => {
                AdapterCommand(serde_json::to_string(config).expect("AcpAgentConfig serializes"))
            }
        }
    }

    pub(crate) fn agent(&self) -> Result<AcpAgent, AcpClientError> {
        Ok(AcpAgent::new(self.resolved_config()?))
    }

    /// The adapter's own CLI (`--cli …`) on this launch: the same program,
    /// environment and installation a session runs, with `words` appended.
    pub fn cli<I, S>(&self, words: I) -> Result<AdapterCli, AcpClientError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.cli_for(words, &[], &[])
    }

    /// [`Self::cli`] for one account: its `inject` over the launch's
    /// environment and its `strip` unset last, as a session for that account
    /// would run — so one preparation serves every account.
    pub fn cli_for<I, S>(
        &self,
        words: I,
        inject: &[(String, String)],
        strip: &[String],
    ) -> Result<AdapterCli, AcpClientError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let config = self.config()?.envs(inject.iter().cloned());
        let config = with_resolved_program(crate::launch_config::finalize_config(config, strip));
        let mut args = config.arguments().to_vec();
        args.extend(words.into_iter().map(Into::into));
        Ok(AdapterCli {
            program: config.command().to_path_buf(),
            args,
            env: config.environment().clone(),
            _lease: match &self.launch {
                Launch::External(_) => None,
                Launch::Installed { lease, .. } => Some(lease.clone()),
            },
        })
    }

    fn resolved_config(&self) -> Result<AcpAgentConfig, AcpClientError> {
        Ok(with_resolved_program(self.config()?))
    }

    fn config(&self) -> Result<AcpAgentConfig, AcpClientError> {
        Ok(match &self.launch {
            Launch::External(command) => AcpAgent::from_str(&command.0)
                .map_err(|error| AcpClientError::Command(format!("{error:?}")))?
                .into_config(),
            Launch::Installed { config, .. } => config.clone(),
        })
    }
}

/// One run of a prepared adapter's CLI. Holds the installation lease, so the
/// cache cannot evict the files it runs until this value is dropped.
pub struct AdapterCli {
    program: std::path::PathBuf,
    args: Vec<String>,
    env: BTreeMap<String, String>,
    _lease: Option<InstallationLease>,
}

impl AdapterCli {
    pub fn program(&self) -> &Path {
        &self.program
    }

    pub fn args(&self) -> &[String] {
        &self.args
    }

    pub fn env(&self) -> &BTreeMap<String, String> {
        &self.env
    }
}

/// `config` with its program spelled as the OS will run it, looked up in the
/// `PATH` the child is given. The SDK spawns it with a bare `Command::new`,
/// which on Windows finds only `<name>.exe` — `npx` and an npm-installed
/// adapter are `.cmd` shims. Every other platform gets `config` back as is.
fn with_resolved_program(config: AcpAgentConfig) -> AcpAgentConfig {
    let path = daruda_core::process::child_path(config.environment());
    let program = daruda_core::process::resolve_program(config.command().as_os_str(), path);
    if program == config.command() {
        return config;
    }
    AcpAgentConfig::new(program)
        .args(config.arguments().to_vec())
        .envs(config.environment().clone())
}

impl From<AdapterCommand> for PreparedAdapter {
    fn from(command: AdapterCommand) -> Self {
        Self {
            launch: Launch::External(command),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Nothing is rewritten where the name already resolves to itself — a
    /// path, or any name off Windows.
    #[test]
    fn a_program_given_as_a_path_is_kept_with_its_args_and_env() {
        let config = AcpAgentConfig::new("/opt/adapter/bin/run")
            .args(["--acp"])
            .env("PATH", "/opt/adapter/bin");
        let resolved = with_resolved_program(config.clone());
        assert_eq!(resolved.command(), config.command());
        assert_eq!(resolved.arguments(), config.arguments());
        assert_eq!(resolved.environment(), config.environment());
    }

    /// The child's own `PATH` is where a `.cmd` shim is looked for — a managed
    /// Node runtime is only on that one.
    #[cfg(windows)]
    #[test]
    fn a_cmd_shim_on_the_childs_path_is_resolved() {
        let dir = tempfile::tempdir().unwrap();
        let shim = dir.path().join("daruda-probe-adapter.cmd");
        std::fs::write(&shim, "@echo off\r\n").unwrap();
        let config = AcpAgentConfig::new("daruda-probe-adapter")
            .args(["--acp"])
            .env("Path", dir.path().to_string_lossy().into_owned());
        let resolved = with_resolved_program(config);
        assert_eq!(resolved.command(), shim);
        assert_eq!(resolved.arguments(), ["--acp".to_string()]);
    }

    /// A CLI run is the adapter's own launch plus the CLI words, so it runs the
    /// program and environment a session would rather than re-resolving them.
    #[test]
    fn cli_appends_its_words_to_the_prepared_launch() {
        let prepared =
            PreparedAdapter::from(AdapterCommand("FOO=1 /opt/adapter/bin/run --acp".into()));
        let cli = prepared.cli(["--cli", "auth", "login"]).unwrap();
        assert_eq!(cli.program(), Path::new("/opt/adapter/bin/run"));
        assert_eq!(cli.args(), ["--acp", "--cli", "auth", "login"]);
        assert_eq!(cli.env()["FOO"], "1");
    }

    /// One prepared launch serves every account: each run layers its account's
    /// variables over the launch's and unsets its strip list last, so a
    /// stripped name is gone even when the launch itself sets it.
    #[test]
    fn cli_for_layers_an_account_over_the_launch() {
        let prepared = PreparedAdapter::from(AdapterCommand(
            "SHARED=launch API_KEY=1 /opt/adapter/bin/run --acp".into(),
        ));
        let cli = prepared
            .cli_for(
                ["--cli"],
                &[("SHARED".into(), "account".into())],
                &["API_KEY".into()],
            )
            .unwrap();
        let args = cli.args();
        let unset = args.iter().position(|a| a == "API_KEY").unwrap();
        let adapter = args
            .iter()
            .position(|a| a == "/opt/adapter/bin/run")
            .unwrap();
        assert!(unset < adapter, "{args:?}");
        assert_eq!(&args[adapter + 1..], ["--acp", "--cli"]);
        assert_eq!(cli.env()["SHARED"], "account");
    }

    #[test]
    fn external_launch_retains_its_command_and_does_not_need_a_cache() {
        let prepared = PreparedAdapter::from(AdapterCommand("ssh host adapter".into()));
        assert_eq!(prepared.command().0, "ssh host adapter");
        assert!(prepared.agent().is_ok());
    }
}
