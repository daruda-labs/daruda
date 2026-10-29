//! Durable identity of an ACP conversation created by a flow attempt.

use crate::NodeId;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// An explicit system account or an opaque managed-account identifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionAccount {
    pub managed: Option<String>,
}

/// Optional host attribution; never contains credentials or launch environments.
#[derive(Debug, Clone, Default)]
pub struct HistoryContext {
    /// Worktree-relative source path. Absent for unattributed executions.
    pub source: Option<PathBuf>,
    pub accounts: std::collections::HashMap<String, SessionAccount>,
}

/// One actual session. Evidence numbers are unique across nodes and repairs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionBinding {
    pub node: NodeId,
    pub evidence: u32,
    #[serde(default)]
    pub attempt: u32,
    pub agent: String,
    /// Unknown for hosts that do not supply account attribution.
    #[serde(default)]
    pub account: Option<SessionAccount>,
    pub cwd: PathBuf,
    pub session_id: String,
}

/// The runner's awaitable commit boundary, installed only by the scheduler.
pub struct SessionPort<'a> {
    pub(crate) store: &'a crate::run_log::store::FlowRunStore,
    pub(crate) node: &'a NodeId,
    pub(crate) evidence: u32,
    pub(crate) attempt: u32,
}

impl SessionPort<'_> {
    pub async fn record(
        &self,
        agent: &str,
        account: Option<SessionAccount>,
        cwd: &std::path::Path,
        session_id: String,
    ) -> Result<(), crate::runner::NodeFailure> {
        self.store
            .commit(crate::run_log::machine::FlowRunEvent::SessionBound {
                binding: SessionBinding {
                    node: self.node.clone(),
                    evidence: self.evidence,
                    attempt: self.attempt,
                    agent: agent.to_owned(),
                    account,
                    cwd: cwd.to_path_buf(),
                    session_id,
                },
            })
            .await
            .map(|_| ())
            .map_err(|error| {
                crate::runner::NodeFailure::SessionError(format!(
                    "could not record the session before prompting: {error:?}"
                ))
            })
    }
}

#[cfg(test)]
mod tests;
