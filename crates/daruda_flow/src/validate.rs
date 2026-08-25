//! The static rules that need the graph to answer: which outputs a node may
//! read, which nodes a gate may re-run, and whether two nodes collide.
//!
//! [`validate`] is the list of them; each rule's own statement is a function
//! in [`rules`]. That split is the point — a rule added here is a name in the
//! list, not another block in one pass.

use crate::NodeId;
use crate::error::{ValidationIssue, ValidationKind};
use crate::graph::FlowGraph;
use crate::model::{AgentFail, Flow, GateFail, NodeKind, Prompt};
use std::collections::{HashMap, HashSet};

mod rules;

/// The only `version` this build executes.
pub const SUPPORTED_VERSION: u32 = 1;

/// Every rule that needs a built graph. Collected, not short-circuited, so
/// one pass reports every problem a flow author has to fix.
pub fn validate(flow: &Flow, graph: &FlowGraph) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();
    rules::version_is_supported(flow, &mut issues);

    let ancestors: HashMap<&NodeId, HashSet<NodeId>> = flow
        .nodes
        .iter()
        .map(|n| (&n.id, graph.ancestors(&n.id).into_iter().collect()))
        .collect();
    // Carried across the loop: a colliding output is a fact about a pair.
    let mut claimed_outputs: HashMap<String, &NodeId> = HashMap::new();

    for node in &flow.nodes {
        let id = &node.id;
        let seen = &ancestors[id];

        rules::id_is_not_reserved(node, &mut issues);
        rules::id_is_wellformed(node, &mut issues);
        rules::cwd_stays_inside_the_run(node, &mut issues);

        match &node.kind {
            NodeKind::Agent(body) => {
                // Delegated: what the check enforces and what is refused here
                // are one statement about the subset, so they live together.
                if let Some(schema) = &body.output_schema {
                    issues.extend(crate::contract::schema::issues(schema, id));
                }
                rules::continue_until_is_readable(
                    id,
                    body.continue_until.as_ref(),
                    body.output_schema.as_ref(),
                    &mut issues,
                );
                rules::turn_cap_allows_a_prompt(id, body.max_turns, &mut issues);
                rules::output_stays_inside_the_run_dir(id, &body.output, &mut issues);
                rules::output_avoids_the_engines_dir(id, &body.output, &mut issues);
                rules::output_is_not_already_claimed(
                    id,
                    &body.output,
                    &mut claimed_outputs,
                    &mut issues,
                );

                let mut texts = Vec::new();
                if let Prompt::Inline(text) = &body.prompt {
                    texts.push(text.as_str());
                }
                if let AgentFail::Retry {
                    hint: Prompt::Inline(text),
                    ..
                } = &body.on_fail
                {
                    texts.push(text.as_str());
                }
                rules::output_refs_name_ancestors(id, &texts, seen, &mut issues);
            }
            NodeKind::Command { run, on_fail } => {
                let mut texts = vec![run.as_str()];
                if let GateFail::Repair { fix, rerun, .. } = on_fail {
                    rules::repair_names_the_failure(id, fix, &mut issues);
                    rules::repair_has_an_agent(id, flow, &mut issues);
                    rules::rerun_roots_are_ancestors(id, rerun, seen, &mut issues);
                    texts.push(fix.as_str());
                }
                rules::output_refs_name_ancestors(id, &texts, seen, &mut issues);
            }
        }
    }

    issues
}

fn issue(node: NodeId, kind: ValidationKind, message: String) -> ValidationIssue {
    ValidationIssue {
        node: Some(node),
        kind,
        message,
    }
}

/// An id has to survive being pasted into a filename, since that is how a
/// failed attempt's evidence is named. `.` is out along with the separators
/// it composes into: it also delimits `{{node.<id>.output}}`.
///
/// Public because an editor asks the same question a beat earlier: a form that
/// let the name through and then reported the engine's refusal after a save
/// round-trip is two answers to one rule, and the second one arrives too late
/// to be useful.
pub fn node_id_is_wellformed(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Find `{{node.<id>.output}}` references and require each target to be an
/// ancestor — `deps` declares order, so any ancestor's output exists.
pub(crate) fn check_output_refs(
    text: &str,
    node: &NodeId,
    ancestors: &HashSet<NodeId>,
    issues: &mut Vec<ValidationIssue>,
) {
    const PREFIX: &str = "{{node.";
    const SUFFIX: &str = ".output";
    const CLOSE: &str = "}}";
    let mut rest = text;
    while let Some(start) = rest.find(PREFIX) {
        let after = &rest[start + PREFIX.len()..];
        // Bound the scan to this `{{ }}` — a template that is not an output
        // reference (or is unterminated) is skipped, not treated as fatal.
        let Some(close) = after.find(CLOSE) else {
            break;
        };
        let token = &after[..close];
        if let Some(referenced) = token.strip_suffix(SUFFIX)
            && !ancestors.contains(referenced)
        {
            issues.push(issue(
                node.clone(),
                ValidationKind::UnreachableOutputRef {
                    referenced: NodeId::from(referenced),
                },
                format!("`{referenced}` is not an ancestor of `{node}`"),
            ));
        }
        rest = &after[close + CLOSE.len()..];
    }
}

#[cfg(test)]
mod tests;
