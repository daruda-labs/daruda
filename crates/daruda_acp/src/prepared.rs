//! Prepared launch data stays pinned until every session using it has ended.

use crate::connection::{AcpClientError, AdapterCommand};
use crate::npm_adapter::cache::InstallationLease;
use agent_client_protocol::{AcpAgent, AcpAgentConfig};
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
        match &self.launch {
            Launch::External(command) => AcpAgent::from_str(&command.0)
                .map_err(|error| AcpClientError::Command(format!("{error:?}"))),
            Launch::Installed { config, .. } => Ok(AcpAgent::new(config.clone())),
        }
    }
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

    #[test]
    fn external_launch_retains_its_command_and_does_not_need_a_cache() {
        let prepared = PreparedAdapter::from(AdapterCommand("ssh host adapter".into()));
        assert_eq!(prepared.command().0, "ssh host adapter");
        assert!(prepared.agent().is_ok());
    }
}
