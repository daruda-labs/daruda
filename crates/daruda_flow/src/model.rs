//! What the engine actually executes: every axis resolved, no `Option`
//! that `defaults` could have filled. Built only by `crate::resolve`.

use crate::NodeId;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Flow {
    pub version: u32,
    /// The profile these settings were merged under, if one was chosen.
    /// Provenance rather than an axis — every axis below is already
    /// resolved. It travels on the flow so a host cannot report a name
    /// other than the one that was actually applied.
    pub profile: Option<String>,
    /// The agent a repair's `fix` session runs as. `None` means the file
    /// names no unambiguous repair agent — legal only when it never repairs.
    pub default_agent: Option<AgentSpec>,
    pub nodes: Vec<Node>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub id: NodeId,
    pub deps: Vec<NodeId>,
    pub kind: NodeKind,
    /// The only axis every node kind shares.
    pub timeout: Duration,
    /// Where this node runs, relative to the run's working directory.
    /// `None` is that directory itself.
    ///
    /// Still relative here, unlike every other axis in this model: the
    /// directory it is relative *to* belongs to the request, not to the
    /// flow, and resolving it at load time would bake one host's path into
    /// a file that is committed and shared.
    pub cwd: Option<PathBuf>,
}

/// Splitting the execution axes by node kind is deliberate: a command node
/// has no agent, model, mode or permission, and a flow of nothing but
/// command nodes must not have to name an agent it never launches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeKind {
    Agent {
        agent: AgentSpec,
        prompt: Prompt,
        output: PathBuf,
        on_fail: AgentFail,
    },
    Command {
        run: String,
        on_fail: GateFail,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSpec {
    /// Catalog id the host resolves into a launch command.
    pub id: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub mode: Option<String>,
    pub permission: PermissionPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionPolicy {
    /// Refuse every request. A request arriving at all means the mode
    /// assumption was wrong, so failing loud beats approving unattended.
    Deny,
    /// Approve requests offering an allow-once option. An always-allow
    /// answer is never selected — it outlives the session.
    AllowOnce,
    /// Hand the request to a person and wait for their answer. The node's
    /// and the run's clocks both stop while it waits, because a budget
    /// bounds work and waiting is the absence of it.
    ///
    /// Only meaningful with a host that can answer — `request` refuses a
    /// run that could reach this without one, so it can never hang.
    Ask,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Prompt {
    Inline(String),
    File(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentFail {
    Halt,
    /// Re-run this node's own prompt with `hint` appended. One session per
    /// attempt — `hint` is the only channel the failure reason has.
    Retry {
        hint: Prompt,
        max_attempts: u32,
        wait: Duration,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateFail {
    Halt,
    /// Run `fix` in a fresh session, then re-derive the verdict starting
    /// from `rerun`. An empty `rerun` re-runs the gate alone.
    Repair {
        fix: String,
        rerun: Vec<NodeId>,
        max_attempts: u32,
        wait: Duration,
    },
}
