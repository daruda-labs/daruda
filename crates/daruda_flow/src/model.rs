//! What the engine actually executes: every axis resolved, no `Option`
//! that `defaults` could have filled. Built only by `crate::resolve`.

use crate::NodeId;
use crate::parse::SchemaSubset;
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
    /// How many nodes may run at once, at least one.
    ///
    /// Nodes sharing a working directory are still run one at a time
    /// whatever this says — see `crate::schedule`. This is a ceiling, not
    /// a promise.
    pub parallel: usize,
    /// The agent a repair's `fix` session runs as. `None` means the file
    /// names no unambiguous repair agent — legal only when it never repairs.
    pub default_agent: Option<AgentSpec>,
    pub nodes: Vec<Node>,
}

/// One field of the output, and the value that means finished.
///
/// The wire type re-exported rather than mirrored: there is nothing to resolve
/// — no default fills a field name in — and two spellings of the same pair
/// would be two places for it to drift.
pub use crate::parse::DoneWhenFile as DoneWhen;

/// Turns one attempt may spend when the flow does not say.
///
/// Two, because that is what the correction turn already allowed: the first
/// prompt, and one more if the output was not usable. A flow written before
/// `max_turns` existed behaves exactly as it did.
pub const DEFAULT_MAX_TURNS: u32 = 2;

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
        /// The shape `output`'s contents must have, when the node declares
        /// one. The wire type unchanged, box included: there is nothing for
        /// `defaults` to fill in and nothing to merge, so a resolved twin
        /// would be a second thing to keep in step for no gain.
        output_schema: Option<Box<SchemaSubset>>,
        /// What the output has to say before this node is finished. `None` is
        /// the rule that held before this existed: a well-formed output is a
        /// finished node.
        ///
        /// The wire type unchanged, for the same reason `output_schema` is:
        /// `defaults` has nothing to fill in here, so a resolved twin would be
        /// a second thing to keep in step for no gain.
        continue_until: Option<Box<DoneWhen>>,
        /// How many prompts one attempt may send, the first included.
        ///
        /// Resolved rather than optional, because the absent case has a real
        /// answer: 2 is what the correction turn already allowed, so a flow
        /// written before this field behaves as it did.
        max_turns: u32,
        on_fail: AgentFail,
    },
    Command {
        run: String,
        on_fail: GateFail,
    },
}

impl NodeKind {
    /// The shape this node's output must have, when it owes one at all.
    pub fn output_schema(&self) -> Option<&SchemaSubset> {
        match self {
            NodeKind::Agent { output_schema, .. } => output_schema.as_deref(),
            NodeKind::Command { .. } => None,
        }
    }

    /// What this node's output has to say before it is finished, when the node
    /// asked for more than a well-formed file.
    pub fn continue_until(&self) -> Option<&DoneWhen> {
        match self {
            NodeKind::Agent { continue_until, .. } => continue_until.as_deref(),
            NodeKind::Command { .. } => None,
        }
    }

    /// How many prompts one attempt of this node may send, the first included.
    /// A command node runs once — there is no turn to spend.
    pub fn max_turns(&self) -> u32 {
        match self {
            NodeKind::Agent { max_turns, .. } => *max_turns,
            NodeKind::Command { .. } => 1,
        }
    }
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
