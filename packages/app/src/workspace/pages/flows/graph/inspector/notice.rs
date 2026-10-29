//! Typed reasons for the inspector's single notification surface.
use daruda_flow::{NodeId, error::ValidationIssue};

#[derive(Clone, Debug, PartialEq)]
pub(in crate::workspace) enum Notice {
    Conflict,
    /// A save the engine refused. `at` is the node selected when it was shown:
    /// the banner belongs there, and on any node the issues name — not on
    /// every node the person visits afterwards.
    Refused {
        message: String,
        issues: Vec<ValidationIssue>,
        at: Option<NodeId>,
    },
    InvalidDraft(NodeId),
    Discarded(Vec<NodeId>),
    NameEmpty,
    Io(String),
    PromptStale,
    PromptOutside,
}

#[cfg(test)]
mod tests {
    // A plain enum; which notice stands when is a model rule, tested in
    // `inspector::tests`.
}
