//! What a node is allowed to ask for: its declared policy, and the
//! capability that becomes once the run's own channels are known.
//! Separated from the drive loop because the loop only hands a capability
//! to the runner — deriving it is a question of its own, and one that has
//! nothing to do with the `on_fail` sense of "policy" next door.

use super::Run;
use crate::model::{AgentSpec, Node, NodeKind};

fn permission_of(node: &Node) -> crate::model::PermissionPolicy {
    match &node.kind {
        NodeKind::Agent(body) => body.agent.permission,
        // A command node launches no agent, so nothing can ask for
        // permission; the value is inert and never read.
        NodeKind::Command { .. } => crate::model::PermissionPolicy::Deny,
    }
}

impl Run<'_> {
    /// Turn a node's declared policy into the capability the runner gets.
    ///
    /// `Ask` without a port cannot be built, so it degrades to `Deny` —
    /// unreachable in practice because `validate_request` refuses such a
    /// run before the lock, and safe rather than silent if it ever were.
    pub(super) fn permission_for(&self, node: &Node) -> crate::runner::Permission<'_> {
        self.permission_for_policy(permission_of(node))
    }

    /// The repair's `fix` runs as `flow.default_agent` and inherits its
    /// policy, so a flow whose defaults say `ask` asks during repair too.
    pub(super) fn permission_for_fix(&self, agent: &AgentSpec) -> crate::runner::Permission<'_> {
        self.permission_for_policy(agent.permission)
    }

    /// A policy becomes a capability: `ask` is only one if this run has
    /// somewhere to ask. Validation refuses that combination up front, so
    /// reaching `Deny` here means a host built a request by hand.
    pub(super) fn permission_for_policy(
        &self,
        policy: crate::model::PermissionPolicy,
    ) -> crate::runner::Permission<'_> {
        match policy {
            crate::model::PermissionPolicy::Deny => crate::runner::Permission::Deny,
            crate::model::PermissionPolicy::AllowOnce => crate::runner::Permission::AllowOnce,
            crate::model::PermissionPolicy::Ask => match self.ask.as_ref() {
                Some(channel) => crate::runner::Permission::Ask(channel),
                None => crate::runner::Permission::Deny,
            },
        }
    }
}
