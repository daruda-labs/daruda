//! Typed reasons for the inspector's single notification surface.
use daruda_flow::{NodeId, error::ValidationIssue};

#[derive(Clone)]
pub(in crate::workspace) enum Notice {
    Conflict,
    Refused {
        message: String,
        issues: Vec<ValidationIssue>,
    },
    InvalidDraft(NodeId),
    Discarded(Vec<NodeId>),
    NameEmpty,
    Io(String),
    PromptStale,
    PromptOutside,
}

#[cfg(test)]
mod tests {}
