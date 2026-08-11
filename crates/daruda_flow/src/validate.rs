//! The static rules that need the graph to answer: which outputs a node may
//! read, which nodes a gate may re-run, and whether two nodes collide.

use crate::NodeId;
use crate::error::{ValidationIssue, ValidationKind};
use crate::graph::FlowGraph;
use crate::model::{AgentFail, Flow, GateFail, NodeKind, Prompt};
use std::collections::{HashMap, HashSet};

/// The only `version` this build executes.
pub const SUPPORTED_VERSION: u32 = 1;

/// Every rule that needs a built graph. Collected, not short-circuited, so
/// one pass reports every problem a flow author has to fix.
pub fn validate(flow: &Flow, graph: &FlowGraph) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();

    if flow.version != SUPPORTED_VERSION {
        issues.push(ValidationIssue {
            node: None,
            kind: ValidationKind::UnknownVersion(flow.version),
            message: format!("this build executes version {SUPPORTED_VERSION} flows"),
        });
    }

    let ancestors: HashMap<&NodeId, HashSet<NodeId>> = flow
        .nodes
        .iter()
        .map(|n| (&n.id, graph.ancestors(&n.id).into_iter().collect()))
        .collect();

    let mut seen_outputs: HashMap<String, &NodeId> = HashMap::new();

    for node in &flow.nodes {
        if node.id == crate::schedule::FIX_SESSION_ID {
            issues.push(issue(
                node.id.clone(),
                ValidationKind::ReservedNodeId,
                format!("`{}` is reserved for the repair session", node.id),
            ));
        }
        if !is_filename_safe(&node.id) {
            issues.push(issue(
                node.id.clone(),
                ValidationKind::InvalidNodeId,
                format!(
                    "`{}` must be non-empty and use only letters, digits, `-` and `_`",
                    node.id
                ),
            ));
        }
        let node_ancestors = &ancestors[&node.id];
        match &node.kind {
            NodeKind::Agent {
                prompt,
                output,
                on_fail,
                ..
            } => {
                if output.components().any(|c| {
                    matches!(
                        c,
                        std::path::Component::ParentDir | std::path::Component::RootDir
                    )
                }) {
                    issues.push(issue(
                        node.id.clone(),
                        ValidationKind::OutputEscapesRunDir,
                        format!("`{}` must stay inside the run directory", output.display()),
                    ));
                }
                // `./logs/x.md` names the same directory, so leading `.`
                // components are skipped rather than trusted — and so does
                // `LOGS/x.md` on the case-insensitive filesystem macOS ships
                // by default, which is this app's primary target. Compared
                // exactly, that flow validates and then writes into the
                // engine's own directory, which is the whole thing this rule
                // exists to stop.
                if output
                    .components()
                    .find(|c| !matches!(c, std::path::Component::CurDir))
                    .and_then(|c| c.as_os_str().to_str())
                    .is_some_and(|c| c.eq_ignore_ascii_case(crate::schedule::LOG_DIR_NAME))
                {
                    issues.push(issue(
                        node.id.clone(),
                        ValidationKind::OutputInReservedDir {
                            reserved: crate::schedule::LOG_DIR_NAME,
                        },
                        format!(
                            "`{}` is reserved for the engine's own artifacts",
                            crate::schedule::LOG_DIR_NAME
                        ),
                    ));
                }
                // Folded, for the same reason as the reserved-directory
                // check above: on macOS's default filesystem `Out.md` and
                // `out.md` are one file, so two nodes that pass an exact
                // comparison silently overwrite each other — and whatever
                // reads `{{node.first.output}}` downstream gets the second
                // node's work without anything saying so.
                //
                // This does reject a pair that a case-sensitive filesystem
                // would keep apart. That is the trade taken deliberately: a
                // flow file is committed and shared, and one that works on
                // Linux and quietly corrupts on macOS is worse than one
                // refused on both.
                let canonical: String = output
                    .components()
                    .filter(|c| !matches!(c, std::path::Component::CurDir))
                    .collect::<std::path::PathBuf>()
                    .to_string_lossy()
                    .to_lowercase();
                if let Some(previous) = seen_outputs.insert(canonical, &node.id) {
                    issues.push(issue(
                        node.id.clone(),
                        ValidationKind::DuplicateOutput,
                        format!("`{}` is already written by `{previous}`", output.display()),
                    ));
                }
                if let Prompt::Inline(text) = prompt {
                    check_output_refs(text, &node.id, node_ancestors, &mut issues);
                }
                if let AgentFail::Retry {
                    hint: Prompt::Inline(text),
                    ..
                } = on_fail
                {
                    check_output_refs(text, &node.id, node_ancestors, &mut issues);
                }
            }
            NodeKind::Command { run, on_fail } => {
                check_output_refs(run, &node.id, node_ancestors, &mut issues);
                if let GateFail::Repair { fix, rerun, .. } = on_fail {
                    if !fix.contains("{{failure}}") && !fix.contains("{{attempts}}") {
                        issues.push(issue(
                            node.id.clone(),
                            ValidationKind::RepairWithoutFailureContext,
                            "a fix prompt must name `{{failure}}` or `{{attempts}}`".to_string(),
                        ));
                    }
                    if flow.default_agent.is_none() {
                        issues.push(issue(
                            node.id.clone(),
                            ValidationKind::RepairWithoutAgent,
                            "a repair runs its fix in an agent session, but this flow \
                             names no unambiguous repair agent"
                                .to_string(),
                        ));
                    }
                    check_output_refs(fix, &node.id, node_ancestors, &mut issues);
                    for root in rerun {
                        if !node_ancestors.contains(root) {
                            issues.push(issue(
                                node.id.clone(),
                                ValidationKind::RerunNotAnAncestor { root: root.clone() },
                                format!("`{root}` is not an ancestor of `{}`", node.id),
                            ));
                        }
                    }
                }
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
fn is_filename_safe(id: &str) -> bool {
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
                    referenced: referenced.to_string(),
                },
                format!("`{referenced}` is not an ancestor of `{node}`"),
            ));
        }
        rest = &after[close + CLOSE.len()..];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{ValidationIssue, ValidationKind};
    use crate::graph::FlowGraph;
    use crate::parse::parse_flow_file;
    use crate::resolve::resolve;

    fn issues_for(text: &str) -> Vec<ValidationIssue> {
        let flow = resolve(parse_flow_file(text).expect("parses"), None).expect("resolves");
        let graph = FlowGraph::build(&flow).expect("acyclic");
        validate(&flow, &graph)
    }

    /// The design document's flagship example. It must pass its own
    /// validator — earlier revisions of the design shipped an example that
    /// did not, twice, and a third that was not even valid YAML.
    ///
    /// Note the quoted `run:` on the gate: a plain YAML scalar may not hold
    /// `": "`, which `VERDICT: PASS` does.
    const FLAGSHIP: &str = "\
version: 1
defaults:
  timeout: 10m
  agent:
    id: claude
    mode: bypassPermissions
    permission: deny
nodes:
  - id: design
    kind: agent
    agent: { effort: high }
    output: design.md
    prompt: Read DESIGN.md and write the design to {{output}}.
  - id: implement
    kind: agent
    deps: [design]
    prompt: Implement the design in {{node.design.output}}.
    output: implement.md
  - id: test
    kind: command
    deps: [implement]
    run: cargo test
    on_fail:
      repair:
        fix: \"{{failure}}. Read {{attempts}} and fix the cause.\"
        max_attempts: 2
  - id: review
    kind: agent
    deps: [test]
    output: review.md
    prompt: Review {{node.implement.output}} and write VERDICT first.
  - id: review-gate
    kind: command
    deps: [review]
    run: \"grep -q '^VERDICT: PASS' {{node.review.output}}\"
    on_fail:
      repair:
        fix: Apply the review notes in {{attempts}}.
        rerun: [review]
        max_attempts: 2
";

    #[test]
    fn the_flagship_example_passes_its_own_validator() {
        assert_eq!(issues_for(FLAGSHIP), Vec::new());
    }

    /// `review` reads `implement`'s output while depending only on `test`.
    /// Legal: `deps` declares order, and `implement` is still an ancestor,
    /// so its output is guaranteed to exist. Checking `deps` instead of
    /// ancestors would reject the flagship example above.
    #[test]
    fn an_output_reference_to_a_non_dep_ancestor_is_allowed() {
        assert!(
            !issues_for(FLAGSHIP)
                .iter()
                .any(|i| matches!(i.kind, ValidationKind::UnreachableOutputRef { .. }))
        );
    }

    #[test]
    fn an_output_reference_to_a_non_ancestor_is_rejected() {
        let issues = issues_for(
            "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: a
    kind: agent
    output: a.md
    prompt: write
  - id: b
    kind: agent
    output: b.md
    prompt: read {{node.a.output}}
",
        );
        assert!(
            issues
                .iter()
                .any(|i| matches!(i.kind, ValidationKind::UnreachableOutputRef { .. })),
            "b does not depend on a, so a's output may not exist yet: {issues:?}"
        );
    }

    /// The same rule applies to a retry hint — it is one of the four
    /// template-bearing fields, and a hint reading a non-ancestor's output
    /// is the same defect as a prompt doing it.
    #[test]
    fn an_output_reference_inside_a_retry_hint_is_checked() {
        let issues = issues_for(
            "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: a
    kind: agent
    output: a.md
    prompt: write
  - id: b
    kind: agent
    output: b.md
    prompt: write
    on_fail:
      retry:
        max_attempts: 2
        hint: look at {{node.a.output}}
",
        );
        assert!(
            issues
                .iter()
                .any(|i| matches!(i.kind, ValidationKind::UnreachableOutputRef { .. }))
        );
    }

    /// An unrelated `{{node.x}}` earlier in the prompt must not swallow the
    /// text up to a later `.output}}` — each `{{ }}` pair is scanned on its
    /// own, so a legal reference after a non-output template still passes.
    #[test]
    fn an_unrelated_template_before_a_legal_output_ref_does_not_break_the_scan() {
        let issues = issues_for(
            "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: a
    kind: agent
    output: a.md
    prompt: write
  - id: b
    kind: agent
    deps: [a]
    output: b.md
    prompt: \"{{node.x}} see {{node.a.output}}\"
",
        );
        assert!(
            !issues
                .iter()
                .any(|i| matches!(i.kind, ValidationKind::UnreachableOutputRef { .. })),
            "a is an ancestor of b, so its output reference is legal: {issues:?}"
        );
    }

    #[test]
    fn duplicate_output_paths_are_rejected() {
        let issues = issues_for(
            "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: a
    kind: agent
    output: shared.md
    prompt: write
  - id: b
    kind: agent
    deps: [a]
    output: shared.md
    prompt: write
",
        );
        assert!(
            issues
                .iter()
                .any(|i| matches!(i.kind, ValidationKind::DuplicateOutput))
        );
    }

    /// `./out.md` and `out.md` name the same file once `.` components are
    /// stripped — the collision must fire even though the strings differ.
    /// macOS ships a case-insensitive filesystem by default, so these are
    /// one file: the second node overwrites the first, and whatever reads
    /// `{{node.first.output}}` downstream gets work it did not ask for
    /// with nothing saying so. Verified against a real APFS volume, not
    /// assumed.
    #[test]
    fn output_paths_differing_only_in_case_are_rejected() {
        let issues = issues_for(
            "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: a
    kind: agent
    output: out.md
    prompt: write
  - id: b
    kind: agent
    output: Out.md
    prompt: write
",
        );
        assert!(
            issues
                .iter()
                .any(|i| matches!(i.kind, ValidationKind::DuplicateOutput)),
            "{issues:?}"
        );
    }

    /// Same filesystem, same reasoning: `LOGS/` reaches the directory the
    /// engine keeps its archives and runner logs in.
    #[test]
    fn an_output_in_the_reserved_directory_is_rejected_whatever_its_case() {
        let issues = issues_for(
            "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: a
    kind: agent
    output: LOGS/out.md
    prompt: write
",
        );
        assert!(
            issues
                .iter()
                .any(|i| matches!(i.kind, ValidationKind::OutputInReservedDir { .. })),
            "{issues:?}"
        );
    }

    #[test]
    fn duplicate_output_paths_with_a_leading_cur_dir_are_rejected() {
        let issues = issues_for(
            "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: a
    kind: agent
    output: out.md
    prompt: write
  - id: b
    kind: agent
    deps: [a]
    output: ./out.md
    prompt: write
",
        );
        assert!(
            issues
                .iter()
                .any(|i| matches!(i.kind, ValidationKind::DuplicateOutput)),
            "./out.md and out.md are the same path: {issues:?}"
        );
    }

    #[test]
    fn an_output_escaping_the_run_directory_is_rejected() {
        let issues = issues_for(
            "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: a
    kind: agent
    output: ../outside.md
    prompt: write
",
        );
        assert!(
            issues
                .iter()
                .any(|i| matches!(i.kind, ValidationKind::OutputEscapesRunDir))
        );
    }

    #[test]
    fn a_rerun_root_outside_the_gates_ancestors_is_rejected() {
        let issues = issues_for(
            "\
version: 1
nodes:
  - id: a
    kind: command
    run: \"true\"
  - id: b
    kind: command
    run: \"true\"
  - id: gate
    kind: command
    deps: [a]
    run: \"true\"
    on_fail:
      repair:
        fix: fix it from {{attempts}}
        rerun: [b]
        max_attempts: 2
",
        );
        assert!(
            issues
                .iter()
                .any(|i| matches!(i.kind, ValidationKind::RerunNotAnAncestor { .. })),
            "b is not an ancestor of gate, so re-running it cannot change the verdict"
        );
    }

    #[test]
    fn a_repair_that_names_no_failure_channel_is_rejected() {
        let issues = issues_for(
            "\
version: 1
nodes:
  - id: gate
    kind: command
    run: \"true\"
    on_fail:
      repair:
        fix: just try again
        max_attempts: 2
",
        );
        assert!(
            issues
                .iter()
                .any(|i| matches!(i.kind, ValidationKind::RepairWithoutFailureContext))
        );
    }

    /// A repair runs its `fix` in an agent session, so a flow that declares
    /// one without any agent to run it cannot be executed.
    #[test]
    fn a_repair_in_a_flow_with_no_agent_is_rejected() {
        let issues = issues_for(
            "\
version: 1
nodes:
  - id: gate
    kind: command
    run: \"true\"
    on_fail:
      repair:
        fix: fix it from {{attempts}}
        max_attempts: 2
",
        );
        assert!(
            issues
                .iter()
                .any(|i| matches!(i.kind, ValidationKind::RepairWithoutAgent)),
            "nothing here can run the fix prompt: {issues:?}"
        );
    }

    /// An id becomes a filename in `logs/`, so a separator in one steers
    /// `archive_attempt`'s rename out of the run directory entirely — it
    /// moves a real output to wherever the id points.
    #[test]
    fn an_id_that_is_not_a_safe_filename_is_rejected() {
        for id in ["../../pwned", "/tmp/abs", "a/b", "a b", "design.v2", ""] {
            let issues = issues_for(&format!(
                "\
version: 1
defaults: {{ agent: {{ id: claude }} }}
nodes:
  - id: \"{id}\"
    kind: agent
    output: out.md
    prompt: write
"
            ));
            assert!(
                issues
                    .iter()
                    .any(|i| matches!(i.kind, ValidationKind::InvalidNodeId)),
                "`{id}` must be rejected, got {issues:?}"
            );
        }
    }

    #[test]
    fn an_ordinary_id_is_accepted() {
        let issues = issues_for(
            "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: design_v2-final
    kind: agent
    output: out.md
    prompt: write
",
        );
        assert!(issues.is_empty(), "{issues:?}");
    }

    /// `logs/` holds the archived evidence, named from node id and attempt.
    /// An output living there can be renamed onto itself, which leaves the
    /// failed attempt's file live for the next attempt to inherit.
    #[test]
    fn an_output_inside_the_engines_own_directory_is_rejected() {
        for output in ["logs/review.md", "./logs/deep/review.md"] {
            let issues = issues_for(&format!(
                "\
version: 1
defaults: {{ agent: {{ id: claude }} }}
nodes:
  - id: review
    kind: agent
    output: {output}
    prompt: write
"
            ));
            assert!(
                issues
                    .iter()
                    .any(|i| matches!(i.kind, ValidationKind::OutputInReservedDir { .. })),
                "`{output}` must be rejected, got {issues:?}"
            );
        }
    }

    /// The archive and the runner log both key on `RunContext.node_id`, so a
    /// node that takes the repair session's id makes its own artifacts and
    /// the fix's indistinguishable.
    #[test]
    fn a_node_that_takes_the_repair_sessions_id_is_rejected() {
        let issues = issues_for(
            "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: __fix__
    kind: agent
    output: fix.md
    prompt: write
",
        );
        assert!(
            issues
                .iter()
                .any(|i| matches!(i.kind, ValidationKind::ReservedNodeId)),
            "{issues:?}"
        );
    }

    #[test]
    fn an_unknown_version_is_rejected() {
        let issues =
            issues_for("version: 99\nnodes:\n  - id: a\n    kind: command\n    run: \"true\"\n");
        assert!(
            issues
                .iter()
                .any(|i| matches!(i.kind, ValidationKind::UnknownVersion(99)))
        );
    }
}
