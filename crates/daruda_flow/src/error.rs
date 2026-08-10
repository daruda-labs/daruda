//! Failures this crate reports. `Display` is the log- and prompt-facing
//! wording; user-visible text is the host's i18n layer, which matches on
//! the variant instead of forwarding these strings.

use crate::NodeId;

#[derive(Debug, thiserror::Error)]
pub enum FlowError {
    /// The file is not valid YAML, or does not match the flow schema.
    #[error("could not parse the flow file: {0}")]
    Parse(String),
    /// Static checks found problems; every one this stage saw is reported.
    #[error("the flow has {} validation problem(s)", .0.len())]
    Validate(Vec<ValidationIssue>),
}

/// One reason a flow cannot be executed. Collected within a stage rather
/// than short-circuited, so one pass reports every problem that stage can
/// see.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationIssue {
    /// `None` when the problem is the graph as a whole, not one node.
    pub node: Option<NodeId>,
    pub kind: ValidationKind,
    /// Developer-facing detail. User-visible wording is the host's, keyed
    /// off `kind`.
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationKind {
    /// An agent node with neither `defaults.agent` nor its own override.
    MissingAgent,
    /// A node names its own `agent.id` but not `agent.mode`.
    AgentIdWithoutMode,
    /// A `version` this build does not know how to execute.
    UnknownVersion(u32),
    /// `{{node.X.output}}` names a node that is not an ancestor, so its
    /// output is not guaranteed to exist when this node runs.
    UnreachableOutputRef { referenced: NodeId },
    /// Two nodes write the same `output` path.
    DuplicateOutput,
    /// `output` resolves outside the run directory.
    OutputEscapesRunDir,
    /// A `rerun` root that is not an ancestor of its gate — re-running it
    /// cannot change the gate's verdict.
    RerunNotAnAncestor { root: NodeId },
    /// A `fix` prompt naming neither `{{failure}}` nor `{{attempts}}`.
    RepairWithoutFailureContext,
    /// A `repair` policy with no unambiguous agent to run its `fix`.
    RepairWithoutAgent,
    /// A node takes the id the repair session is recorded under.
    ReservedNodeId,
    /// An id that cannot be used as a filename. Archive names are built
    /// from the id, so a `/` or a `..` in one would move a failed
    /// attempt's evidence outside the run directory.
    InvalidNodeId,
    /// A key the schema does not have. Reported rather than ignored
    /// because the fields most worth mistyping — `deps`, `timeout`,
    /// `on_fail` — decide execution order and failure handling, and a
    /// silently-dropped one runs a different flow than the file describes.
    UnknownField { field: String },
    /// Both halves of an either-or pair, where one silently wins:
    /// `prompt` with `prompt_file`, or `hint` with `hint_file`. The
    /// ignored one is usually the one the author was editing.
    ConflictingField { field: String, wins: &'static str },
    /// An `output` under the directory the engine keeps its own artifacts
    /// in. Both would then write the same names, and archiving an output
    /// onto itself leaves the failed attempt's file live.
    OutputInReservedDir { reserved: &'static str },
    /// Two nodes share an id.
    DuplicateId,
    /// A node depends on an id no node declares.
    UnknownDep { dep: NodeId },
    /// The declared graph is cyclic.
    Cycle,
    /// An agent node names a catalog id the host did not supply.
    UnknownAgent { id: String },
    /// The run could reach `permission: ask` and this host wired nowhere
    /// to answer. Refused at submission rather than at the request, which
    /// would park a run nobody can release.
    NobodyToAsk,
    /// A file-backed prompt that does not exist next to the flow. `field`
    /// names which key it was, because the host renders its wording from
    /// this variant and "prompt file missing" would be wrong for a hint.
    MissingPromptFile {
        field: &'static str,
        path: std::path::PathBuf,
    },
    /// A path the host supplied is relative. Every path in a `RunRequest`
    /// is resolved by the host, so a relative one is resolved again against
    /// whatever directory the process happens to be in — which is neither
    /// the lane nor the run.
    RelativeRequestPath { field: &'static str },
    /// A file-backed prompt is not a flow-local relative path, or resolves
    /// outside the directory containing the flow file.
    PromptFileOutsideFlowDir {
        field: &'static str,
        path: std::path::PathBuf,
    },
}

/// Where an I/O failure happened. A run-level failure has no node and no
/// attempt; a node-level one always has both, so they travel together
/// rather than as two `Option`s that are always set or unset in step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IoSite {
    /// The lock or the completion marker — before or after any node.
    Run,
    Node {
        node: NodeId,
        attempt: u32,
    },
}

/// An I/O failure with enough context to act on. `std::io::Error` alone
/// rarely carries the path, and the engine touches several per attempt.
#[derive(Debug)]
pub struct FlowIoError {
    pub site: IoSite,
    /// What the engine was doing, as a fixed label. `&'static str` rather
    /// than an enum, matching `ValidationKind::MissingPromptFile`'s `field`.
    pub doing: &'static str,
    pub path: std::path::PathBuf,
    pub source: std::io::Error,
}

impl std::fmt::Display for FlowIoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.site {
            IoSite::Run => write!(f, "{} ", self.doing)?,
            IoSite::Node { node, attempt } => {
                write!(f, "{} for `{node}` attempt {attempt} ", self.doing)?
            }
        }
        write!(f, "failed at {}: {}", self.path.display(), self.source)
    }
}

impl std::error::Error for FlowIoError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_names_the_problem_count_without_leaking_wording() {
        let err = FlowError::Validate(vec![ValidationIssue {
            node: Some("a".to_string()),
            kind: ValidationKind::MissingAgent,
            message: "detail".to_string(),
        }]);
        let text = err.to_string();
        assert!(
            text.contains('1'),
            "the count belongs in the message: {text}"
        );
        // The per-issue `message` is developer detail; the host renders the
        // user-facing wording from `kind`, so it must not be spliced here.
        assert!(!text.contains("detail"));
    }
}
