//! One rule, one function.
//!
//! Split out because the alternative is what this file replaced: a single
//! pass with a block per rule, where adding a rule meant growing one function
//! and the rule's name lived only in a comment. Here the name is the
//! signature, and `super::validate` reads as the list of what a flow is held
//! to.
//!
//! Every rule appends and none returns — one pass reports every problem an
//! author has to fix, rather than the first.

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path};

use super::{SUPPORTED_VERSION, check_output_refs, issue, node_id_is_wellformed};
use crate::NodeId;
use crate::error::{ValidationIssue, ValidationKind};
use crate::model::{AgentSpec, DoneWhen, Flow, Node};
use crate::parse::{SchemaKind, SchemaSubset};

/// Whether a path reaches outside the directory it is resolved against.
fn climbs_out(path: &Path) -> bool {
    path.components()
        .any(|c| matches!(c, Component::ParentDir | Component::RootDir))
}

/// This build runs one `version`, and says so rather than guessing at a file
/// written for another.
pub(super) fn version_is_supported(flow: &Flow, issues: &mut Vec<ValidationIssue>) {
    if flow.version != SUPPORTED_VERSION {
        issues.push(ValidationIssue {
            node: None,
            kind: ValidationKind::UnknownVersion(flow.version),
            message: format!("this build executes version {SUPPORTED_VERSION} flows"),
        });
    }
}

/// The repair session's own id is not an author's to take — a node holding it
/// would collide with the session a gate's `fix` runs in.
pub(super) fn id_is_not_reserved(node: &Node, issues: &mut Vec<ValidationIssue>) {
    if node.id == crate::schedule::FIX_SESSION_ID {
        issues.push(issue(
            node.id.clone(),
            ValidationKind::ReservedNodeId,
            format!("`{}` is reserved for the repair session", node.id),
        ));
    }
}

/// An id has to survive being pasted into a filename and into
/// `{{node.<id>.output}}`; [`node_id_is_wellformed`] is that rule.
pub(super) fn id_is_wellformed(node: &Node, issues: &mut Vec<ValidationIssue>) {
    if !node_id_is_wellformed(node.id.as_str()) {
        issues.push(issue(
            node.id.clone(),
            ValidationKind::InvalidNodeId,
            format!(
                "`{}` must be non-empty and use only letters, digits, `-` and `_`",
                node.id
            ),
        ));
    }
}

/// Every node kind, not only agent nodes: a command node's `cwd` is the more
/// likely one to be pointed somewhere it should not be.
pub(super) fn cwd_stays_inside_the_run(node: &Node, issues: &mut Vec<ValidationIssue>) {
    if let Some(cwd) = &node.cwd
        && climbs_out(cwd)
    {
        issues.push(issue(
            node.id.clone(),
            ValidationKind::CwdEscapesRunCwd,
            format!(
                "`{}` must stay inside the run's working directory",
                cwd.display()
            ),
        ));
    }
}

/// An attempt that may send no prompt at all cannot run the node.
pub(super) fn turn_cap_allows_a_prompt(
    node: &NodeId,
    max_turns: u32,
    issues: &mut Vec<ValidationIssue>,
) {
    if max_turns == 0 {
        issues.push(issue(
            node.clone(),
            ValidationKind::MaxTurnsIsZero,
            "an attempt has to be allowed at least one prompt".to_string(),
        ));
    }
}

pub(super) fn output_stays_inside_the_run_dir(
    node: &NodeId,
    output: &Path,
    issues: &mut Vec<ValidationIssue>,
) {
    if climbs_out(output) {
        issues.push(issue(
            node.clone(),
            ValidationKind::OutputEscapesRunDir,
            format!("`{}` must stay inside the run directory", output.display()),
        ));
    }
}

/// `./logs/x.md` names the same directory, so leading `.` components are
/// skipped rather than trusted — and so does `LOGS/x.md` on the
/// case-insensitive filesystem macOS ships by default, which is this app's
/// primary target. Compared exactly, that flow validates and then writes into
/// the engine's own directory, which is the whole thing this rule exists to
/// stop.
pub(super) fn output_avoids_the_engines_dir(
    node: &NodeId,
    output: &Path,
    issues: &mut Vec<ValidationIssue>,
) {
    if output
        .components()
        .find(|c| !matches!(c, Component::CurDir))
        .and_then(|c| c.as_os_str().to_str())
        .is_some_and(|c| c.eq_ignore_ascii_case(crate::schedule::LOG_DIR_NAME))
    {
        issues.push(issue(
            node.clone(),
            ValidationKind::OutputInReservedDir {
                reserved: crate::schedule::LOG_DIR_NAME,
            },
            format!(
                "`{}` is reserved for the engine's own artifacts",
                crate::schedule::LOG_DIR_NAME
            ),
        ));
    }
}

/// Folded, for the same reason as [`output_avoids_the_engines_dir`]: on
/// macOS's default filesystem `Out.md` and `out.md` are one file — and so are
/// two spellings of the same character that differ only in Unicode
/// normalisation, which is why case alone was not enough. Two nodes that pass
/// an exact comparison silently overwrite each other, and whatever reads
/// `{{node.first.output}}` downstream gets the second node's work without
/// anything saying so.
///
/// This does reject a pair that a case-sensitive filesystem would keep apart.
/// That is the trade taken deliberately: a flow file is committed and shared,
/// and one that works on Linux and quietly corrupts on macOS is worse than
/// one refused on both.
///
/// `claimed` is carried across nodes because a collision is a fact about a
/// pair, not about either node alone.
pub(super) fn output_is_not_already_claimed<'a>(
    node: &'a NodeId,
    output: &Path,
    claimed: &mut HashMap<String, &'a NodeId>,
    issues: &mut Vec<ValidationIssue>,
) {
    if let Some(previous) = claimed.insert(canonical_output(output), node) {
        issues.push(issue(
            node.clone(),
            ValidationKind::DuplicateOutput,
            format!("`{}` is already written by `{previous}`", output.display()),
        ));
    }
}

/// An output path folded to what the filesystem would treat it as.
///
/// Three foldings, each for something macOS's default filesystem does: `.`
/// components dropped (`./a.md` is `a.md`), case lowered, and Unicode
/// composed. The last is the one std cannot do and the one that reaches
/// beyond Latin — `각` written as one character and as three are the same
/// file, and so are the two spellings of `が`.
///
/// A heuristic, deliberately: the exact table a given macOS version folds by
/// is not ours to reproduce. Composing catches the spellings a person or an
/// editor actually produces, and erring toward refusing a pair a
/// case-sensitive filesystem would keep apart is the trade already taken
/// above.
fn canonical_output(output: &Path) -> String {
    use unicode_normalization::UnicodeNormalization;
    output
        .components()
        .filter(|c| !matches!(c, Component::CurDir))
        .collect::<std::path::PathBuf>()
        .to_string_lossy()
        .to_lowercase()
        .nfc()
        .collect()
}

/// Whether a `continue_until` can be read at all.
///
/// Four ways it cannot, and each fails the same way if unchecked: the node
/// runs its turns, nothing ever matches, and it fails on the turn cap having
/// spent every session. The rules exist so that happens at load instead.
pub(super) fn continue_until_is_readable(
    node: &NodeId,
    done_when: Option<&DoneWhen>,
    schema: Option<&SchemaSubset>,
    issues: &mut Vec<ValidationIssue>,
) {
    let Some(done_when) = done_when else {
        return;
    };
    // Read out of the output's JSON, so the output has to be a JSON object.
    let Some(schema) = schema.filter(|s| s.kind == SchemaKind::Object) else {
        issues.push(issue(
            node.clone(),
            ValidationKind::ContinueUntilWithoutObjectSchema,
            "`continue_until` reads a field out of the output, which needs an \
             `output_schema` of `type: object`"
                .to_string(),
        ));
        return;
    };
    let Some(field) = schema.properties.get(&done_when.field) else {
        issues.push(issue(
            node.clone(),
            ValidationKind::ContinueUntilFieldNotDeclared {
                field: done_when.field.clone(),
            },
            format!(
                "`{}` is not in this node's `output_schema`, so nothing tells the agent to write it",
                done_when.field
            ),
        ));
        return;
    };
    // A value the field cannot hold is the same defect as a field that is not
    // there: the node spends every turn and fails on the cap. This one is the
    // easiest to write by accident, because the enum and the verdict are two
    // lines apart and a typo in either reads fine.
    if let Some(allowed) = field.allowed.as_ref()
        && !allowed.contains(&done_when.equals)
    {
        issues.push(issue(
            node.clone(),
            ValidationKind::ContinueUntilValueNotAllowed {
                field: done_when.field.clone(),
            },
            format!(
                "`{}` is not one of the values `{}` allows, so the output could never say it",
                done_when.equals, done_when.field
            ),
        ));
        return;
    }
    // Optional would mean an agent that writes nothing leaves the node
    // finished, which is the opposite of what `continue_until` asks for.
    if !schema.required.contains(&done_when.field) {
        issues.push(issue(
            node.clone(),
            ValidationKind::ContinueUntilFieldNotRequired {
                field: done_when.field.clone(),
            },
            format!(
                "`{}` has to be in `required`, or an output that omits it counts as finished",
                done_when.field
            ),
        ));
    }
}

/// A fix prompt that names neither is a repair that cannot know what it is
/// repairing.
pub(super) fn repair_names_the_failure(
    node: &NodeId,
    fix: &str,
    issues: &mut Vec<ValidationIssue>,
) {
    if !fix.contains("{{failure}}") && !fix.contains("{{attempts}}") {
        issues.push(issue(
            node.clone(),
            ValidationKind::RepairWithoutFailureContext,
            "a fix prompt must name `{{failure}}` or `{{attempts}}`".to_string(),
        ));
    }
}

/// A repair runs its fix in an agent session, so the flow has to name one.
pub(super) fn repair_has_an_agent(
    node: &NodeId,
    default_agent: Option<&AgentSpec>,
    issues: &mut Vec<ValidationIssue>,
) {
    if default_agent.is_none() {
        issues.push(issue(
            node.clone(),
            ValidationKind::RepairWithoutAgent,
            "a repair runs its fix in an agent session, but this flow \
             names no unambiguous repair agent"
                .to_string(),
        ));
    }
}

/// A gate can only send the run back through nodes it is downstream of.
pub(super) fn rerun_roots_are_ancestors(
    node: &NodeId,
    rerun: &[NodeId],
    ancestors: &HashSet<NodeId>,
    issues: &mut Vec<ValidationIssue>,
) {
    for root in rerun {
        if !ancestors.contains(root) {
            issues.push(issue(
                node.clone(),
                ValidationKind::RerunNotAnAncestor { root: root.clone() },
                format!("`{root}` is not an ancestor of `{node}`"),
            ));
        }
    }
}

/// Every `{{node.<id>.output}}` in `texts` has to name an ancestor.
///
/// Takes a slice because a node reads its templates from more than one place —
/// an agent's prompt and its retry hint, a gate's command and its fix — and
/// the rule is the same for each.
pub(super) fn output_refs_name_ancestors(
    node: &NodeId,
    texts: &[&str],
    ancestors: &HashSet<NodeId>,
    issues: &mut Vec<ValidationIssue>,
) {
    for text in texts {
        check_output_refs(text, node, ancestors, issues);
    }
}
