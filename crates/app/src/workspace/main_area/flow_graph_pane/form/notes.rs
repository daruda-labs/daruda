//! Which field a refusal is about.
//!
//! The engine says what is wrong; this says where to put it. That split is the
//! whole design: `ValidationKind` stays the one place a rule lives, the wording
//! stays in `surface::strings`, and the only new knowledge here is which box on
//! the form a person would go to in order to fix it.
//!
//! A `ValidationIssue` names a *node*, never a field, so the mapping is ours to
//! make — and it is deliberately partial. A cycle, an unknown `version`, an
//! unreachable output reference: those are about the flow rather than about any
//! one box, and they stay in the banner where the whole sentence is readable.

use daruda_flow::error::{ValidationIssue, ValidationKind};

/// A box on the inspector that a rule can name.
///
/// Only those: there is no `Run` here because no `ValidationKind` is about a
/// command's line — the engine loads an empty one, which is why the *form*
/// refuses that itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::workspace) enum FormField {
    Id,
    Deps,
    Cwd,
    /// The agent-override block as a whole — its five boxes are one decision.
    Agent,
    Prompt,
    Output,
    Fix,
    Rerun,
}

/// One refusal, pinned to the box it is about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::workspace) struct FieldNote {
    pub field: FormField,
    pub message: String,
}

/// The notes for `node`, out of everything the engine refused.
///
/// Issues about *other* nodes are dropped: they are real, and the banner still
/// carries them, but pointing at this form's `output` box for another node's
/// duplicate output would send the person to the wrong place.
pub(in crate::workspace) fn notes_for(issues: &[ValidationIssue], node: &str) -> Vec<FieldNote> {
    issues
        .iter()
        .filter(|issue| issue.node.as_deref() == Some(node))
        .filter_map(|issue| {
            field_for(&issue.kind).map(|field| FieldNote {
                field,
                message: crate::surface::strings::flow_issue(&issue.kind),
            })
        })
        .collect()
}

/// Which box a rule is about, when it is about one.
fn field_for(kind: &ValidationKind) -> Option<FormField> {
    use ValidationKind as K;
    Some(match kind {
        K::InvalidNodeId | K::ReservedNodeId | K::DuplicateId => FormField::Id,
        K::UnknownDep { .. } => FormField::Deps,
        K::CwdEscapesRunCwd | K::CwdMissing { .. } => FormField::Cwd,
        K::MissingAgent
        | K::AgentIdWithoutMode
        | K::AskWithoutMode
        | K::UnknownAgent { .. }
        | K::NobodyToAsk => FormField::Agent,
        K::MissingPromptFile { .. } | K::PromptFileOutsideFlowDir { .. } => FormField::Prompt,
        K::DuplicateOutput | K::OutputEscapesRunDir | K::OutputInReservedDir { .. } => {
            FormField::Output
        }
        K::RepairWithoutFailureContext | K::RepairWithoutAgent => FormField::Fix,
        K::RerunNotAnAncestor { .. } => FormField::Rerun,
        // A field the schema does not have, or a pair where one silently wins:
        // both name the key, but the form has a box per *concept*, and the key
        // it names may be one the form does not show at all.
        K::UnknownField { .. } | K::ConflictingField { .. } => return None,
        // About the flow, not about a box.
        K::Cycle
        | K::UnknownVersion(_)
        | K::UnknownProfile { .. }
        | K::ReservedProfileName
        | K::UnreachableOutputRef { .. }
        | K::RelativeRequestPath { .. } => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issue(node: Option<&str>, kind: ValidationKind) -> ValidationIssue {
        ValidationIssue {
            node: node.map(str::to_string),
            kind,
            message: "engine detail".into(),
        }
    }

    #[test]
    fn a_rule_about_this_node_finds_its_box() {
        let notes = notes_for(
            &[issue(Some("design"), ValidationKind::AgentIdWithoutMode)],
            "design",
        );
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].field, FormField::Agent);
        assert_eq!(
            notes[0].message,
            crate::surface::strings::flow_issue(&ValidationKind::AgentIdWithoutMode),
            "the wording is the one wording site's, not a second copy"
        );
    }

    /// The trap: `DuplicateOutput` names *one* of the two nodes, and pointing at
    /// this form's box for the other one's problem sends the person to a box
    /// that is not wrong.
    #[test]
    fn a_rule_about_another_node_points_at_nothing_here() {
        let notes = notes_for(
            &[issue(Some("build"), ValidationKind::DuplicateOutput)],
            "design",
        );
        assert!(notes.is_empty(), "{notes:?}");
    }

    #[test]
    fn a_rule_about_the_flow_stays_in_the_banner() {
        for kind in [
            ValidationKind::Cycle,
            ValidationKind::UnknownVersion(9),
            ValidationKind::UnreachableOutputRef {
                referenced: "x".into(),
            },
        ] {
            assert!(
                notes_for(&[issue(Some("design"), kind.clone())], "design").is_empty(),
                "{kind:?} is about the flow, not a box"
            );
        }
    }
}
