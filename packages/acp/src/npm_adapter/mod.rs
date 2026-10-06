//! Install supported npm adapters before starting their JS entry with Node.
//! Unknown launchers and npx options retain their configured execution path.

pub(crate) mod cache;
mod package;
mod runtime;

use std::path::Path;
use std::str::FromStr;

use agent_client_protocol::{AcpAgent, AcpAgentConfig};

use crate::PreparedAdapter;
use crate::node::NodeProgress;
use crate::preparation::{PreparationContext, PreparationError};

/// Only adapters whose npm distribution exposes a Node-compatible entry.
const SUPPORTED_ADAPTERS: &[(&str, &str)] = &[
    ("@agentclientprotocol/claude-agent-acp", "claude-agent-acp"),
    ("@agentclientprotocol/codex-acp", "codex-acp"),
];
const DEFAULT_SELECTOR: &str = "latest";

pub(crate) struct NpmAdapter {
    name: &'static str,
    bin: &'static str,
    selector: String,
    config: AcpAgentConfig,
}

impl NpmAdapter {
    pub(crate) fn parse(command: &str) -> Option<Self> {
        let config = if command.trim_start().starts_with('{') {
            crate::launch_config::parse_json_launch(command)?
        } else {
            AcpAgent::from_str(command).ok()?.into_config()
        };
        // An absolute executable or alternate launcher is an explicit override.
        if config.command() != Path::new("npx") {
            return None;
        }
        let mut args = config.arguments().iter();
        let mut spec = args.next()?;
        if matches!(spec.as_str(), "-y" | "--yes") {
            spec = args.next()?;
        }
        if spec == "--" {
            spec = args.next()?;
        }
        let (name, bin, selector) = SUPPORTED_ADAPTERS.iter().find_map(|&(name, bin)| {
            if spec == name {
                Some((name, bin, DEFAULT_SELECTOR))
            } else {
                spec.strip_prefix(name)?
                    .strip_prefix('@')
                    .filter(|selector| valid_selector(selector))
                    .map(|selector| (name, bin, selector))
            }
        })?;
        let selector = selector.to_owned();
        let config = AcpAgentConfig::new("node")
            .args(args.cloned())
            .envs(config.environment().clone());
        Some(Self {
            name,
            bin,
            selector,
            config,
        })
    }

    pub(crate) fn prepare(
        &self,
        root: &Path,
        strip_env: &[String],
        progress: &mut dyn FnMut(NodeProgress),
        context: &PreparationContext<'_>,
    ) -> Result<PreparedAdapter, PreparationError> {
        let node = crate::node::resolved::resolve(
            root,
            self.config.environment(),
            strip_env,
            progress,
            context,
        )?;
        let runtime = runtime::NpmRuntime::resolve(node, self.config.environment(), strip_env)?;
        let installed = package::install(self, &runtime, root, context)?;
        let config = AcpAgentConfig::new(&runtime.node)
            .arg(installed.entry.to_string_lossy().into_owned())
            .args(self.config.arguments().iter().cloned())
            .envs(runtime.env.clone());
        Ok(PreparedAdapter::installed(config, installed.lease))
    }
}

fn valid_selector(selector: &str) -> bool {
    !selector.is_empty()
        && selector.chars().all(|c| {
            c.is_ascii_alphanumeric()
                || matches!(
                    c,
                    '.' | '-' | '_' | '^' | '~' | '*' | '>' | '<' | '=' | ' ' | '|'
                )
        })
}

#[cfg(test)]
mod tests;
