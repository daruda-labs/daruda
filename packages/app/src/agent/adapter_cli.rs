//! An agent CLI run (login, auth status) on the adapter a session would run.
//!
//! GPUI-free. [`prepare`] goes through [`super::adapter_prep`], so for a
//! supported npm adapter a CLI run never reaches npx's shared, unlocked
//! cache; any other launch runs as its sessions do. One preparation serves
//! every account ([`PreparedAgentCli::for_account`]).

use daruda_acp::PreparedAdapter;
use daruda_acp::preparation::PreparationCancellation;
use daruda_agent::accounts::CliCommand;
use daruda_config::AccountEnv;

use super::launch_resolve::{resolve_local_cli_launch, strip_names};

/// A CLI run before preparation: the agent that serves the auth domain and
/// the CLI words to run on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AdapterCliRequest {
    /// The agent's local adapter command.
    pub(crate) adapter: String,
    /// The agent definition's own environment.
    pub(crate) agent_env: Vec<(String, String)>,
    pub(crate) words: Vec<String>,
}

/// A prepared adapter and the CLI words to run on it, for any account.
pub(crate) struct PreparedAgentCli {
    adapter: PreparedAdapter,
    words: Vec<String>,
}

/// A CLI run ready to spawn. Holds the installation lease until dropped, so
/// the cache cannot evict the files the run uses.
pub(crate) struct PreparedCli {
    pub(crate) command: CliCommand,
    _installation: daruda_acp::AdapterCli,
}

/// Prepare `request`'s adapter. **Blocking**; see [`super::adapter_prep::prepare`].
pub(crate) fn prepare(
    request: &AdapterCliRequest,
    cancellation: &PreparationCancellation,
) -> Result<PreparedAgentCli, String> {
    let spec = resolve_local_cli_launch(&request.adapter, &request.agent_env);
    let adapter = super::adapter_prep::prepare(&spec, cancellation, &mut |_| {})
        .map_err(|e| e.to_string())?;
    Ok(PreparedAgentCli {
        adapter,
        words: request.words.clone(),
    })
}

impl PreparedAgentCli {
    /// The run for `account`: its environment over the agent's, its strip
    /// list enforced last — the precedence a session for it gets.
    pub(crate) fn for_account(&self, account: &AccountEnv) -> Result<PreparedCli, String> {
        let cli = self
            .adapter
            .cli_for(
                self.words.iter().cloned(),
                &account.inject,
                &strip_names(account),
            )
            .map_err(|e| e.to_string())?;
        Ok(PreparedCli {
            command: command_of(&cli),
            _installation: cli,
        })
    }
}

fn command_of(cli: &daruda_acp::AdapterCli) -> CliCommand {
    CliCommand {
        program: cli.program().to_path_buf(),
        args: cli.args().to_vec(),
        env: cli
            .env()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A managed account's run gets its config dir and loses the variables
    /// it strips, even ones the agent itself declares.
    #[test]
    fn a_managed_account_layers_over_the_agent() {
        let prepared = PreparedAgentCli {
            adapter: PreparedAdapter::from(daruda_acp::AdapterCommand(
                "ANTHROPIC_API_KEY=agent /opt/adapter --acp".into(),
            )),
            words: vec!["--cli".into(), "auth".into()],
        };
        let account = AccountEnv {
            inject: vec![("CLAUDE_CONFIG_DIR".into(), "/acct".into())],
            strip: vec!["ANTHROPIC_API_KEY"],
        };
        let run = prepared.for_account(&account).unwrap();
        let args = &run.command.args;
        let unset = args.iter().position(|a| a == "ANTHROPIC_API_KEY").unwrap();
        let adapter = args.iter().position(|a| a == "/opt/adapter").unwrap();
        assert!(unset < adapter, "{args:?}");
        assert_eq!(&args[adapter + 1..], ["--acp", "--cli", "auth"]);
        assert!(
            run.command
                .env
                .contains(&("CLAUDE_CONFIG_DIR".into(), "/acct".into()))
        );
    }

    /// The System account adds nothing: no wrapper, the agent's run as is.
    #[test]
    fn the_system_account_runs_the_agent_unchanged() {
        let prepared = PreparedAgentCli {
            adapter: PreparedAdapter::from(daruda_acp::AdapterCommand("/opt/adapter --acp".into())),
            words: vec!["--cli".into()],
        };
        let run = prepared.for_account(&AccountEnv::ambient()).unwrap();
        assert_eq!(run.command.program, std::path::Path::new("/opt/adapter"));
        assert_eq!(run.command.args, ["--acp", "--cli"]);
    }
}
