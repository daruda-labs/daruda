//! The one call a host makes: text in, an executable flow out, or every
//! problem that stopped it.
//!
//! Stages run in order and the first failing one reports — a flow that
//! cannot merge has no `Flow` to build a graph from, and one with a cycle
//! has no order to check outputs against. `resolve` and `validate` collect
//! every issue in their stage; `FlowGraph::build` stops at the first
//! `DuplicateId` or `UnknownDep`.

use crate::error::{FlowError, ValidationIssue, ValidationKind};
use crate::graph::{FlowGraph, GraphError};
use crate::model::Flow;
use crate::{parse, resolve, validate};

/// A flow and the graph built from it.
///
/// One value because the two are only meaningful together: every graph
/// question is asked about a node of *that* flow, and asking it of another
/// flow's graph returns an empty answer rather than an error — a check
/// that silently passes. Held privately so the pair can only come from
/// [`load`], which is the only thing that builds them in step.
#[derive(Debug)]
pub struct LoadedFlow {
    flow: Flow,
    graph: FlowGraph,
}

impl LoadedFlow {
    pub fn flow(&self) -> &Flow {
        &self.flow
    }

    pub fn graph(&self) -> &FlowGraph {
        &self.graph
    }
}

/// The profiles this flow declares, in the order a host should offer
/// them. Parsing only — the choice has to be made before anything can be
/// merged, so this cannot come out of [`load`].
pub fn profiles(text: &str) -> Result<Vec<String>, FlowError> {
    Ok(parse::parse_flow_file(text)?.profiles.into_keys().collect())
}

/// A flow that resolved and whose graph built, beside whatever the
/// graph-dependent rules refused about it.
///
/// Both halves are real, and that is the whole point: [`validate::validate`]
/// runs *on* a resolved flow and a built graph, so every issue it reports is
/// about a flow that can be drawn. An editor can show the picture and the
/// problem together, where [`load`] has to choose one.
///
/// The earlier stages have no such pair to offer — a file that does not parse,
/// resolve, or build a graph leaves nothing to draw — so they stay `Err`.
pub struct Inspected {
    pub loaded: LoadedFlow,
    /// Empty when the flow is runnable. Non-empty means [`load`] refuses it.
    pub issues: Vec<ValidationIssue>,
}

/// Parse, merge under `profile`, build the graph, and run the
/// graph-dependent rules — reporting the last stage's verdict rather than
/// acting on it.
pub fn inspect(text: &str, profile: Option<&str>) -> Result<Inspected, FlowError> {
    let file = parse::parse_flow_file(text)?;
    // After the shape parses and before anything is merged: these are the
    // keys serde could not police itself (see `parse::schema_issues`), and
    // a node whose `deps` was mistyped must not reach the graph builder.
    let issues = parse::schema_issues(text);
    if !issues.is_empty() {
        return Err(FlowError::Validate(issues));
    }
    let flow = resolve::resolve(file, profile).map_err(FlowError::Validate)?;
    let graph = FlowGraph::build(&flow).map_err(|e| FlowError::Validate(vec![graph_issue(e)]))?;
    let issues = validate::validate(&flow, &graph);
    Ok(Inspected {
        loaded: LoadedFlow { flow, graph },
        issues,
    })
}

/// The same, for everyone who needs a flow that will actually run.
///
/// Every path to an agent session goes through here, so the rules stay a
/// refusal no matter what the editor is willing to draw.
pub fn load(text: &str, profile: Option<&str>) -> Result<LoadedFlow, FlowError> {
    let inspected = inspect(text, profile)?;
    if inspected.issues.is_empty() {
        Ok(inspected.loaded)
    } else {
        Err(FlowError::Validate(inspected.issues))
    }
}

/// A graph failure is a validation result to the host, which knows only
/// `FlowError`. `GraphError` stays a distinct type inside the crate so the
/// graph module can be tested without the error taxonomy.
fn graph_issue(error: GraphError) -> ValidationIssue {
    let message = error.to_string();
    match error {
        GraphError::DuplicateId(id) => ValidationIssue {
            node: Some(id),
            kind: ValidationKind::DuplicateId,
            message,
        },
        GraphError::UnknownDep { node, dep } => ValidationIssue {
            node: Some(node),
            kind: ValidationKind::UnknownDep { dep },
            message,
        },
        GraphError::Cycle(_) => ValidationIssue {
            node: None,
            kind: ValidationKind::Cycle,
            message,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ValidationKind;

    #[test]
    fn a_valid_flow_yields_both_the_model_and_its_graph() {
        let loaded = load(
            "\
version: 1
nodes:
  - id: a
    kind: command
    run: \"true\"
  - id: b
    kind: command
    deps: [a]
    run: \"true\"
",
            None,
        )
        .expect("valid flow loads");
        assert_eq!(loaded.flow().nodes.len(), 2);
        assert_eq!(loaded.graph().topological_order(), vec!["a", "b"]);
    }

    #[test]
    fn a_parse_error_stays_a_parse_error() {
        assert!(matches!(load("nodes: []", None), Err(FlowError::Parse(_))));
    }

    /// A cycle is a graph fact, but the host sees one error type. Without
    /// this bridge `GraphError` would have no route out of the crate.
    #[test]
    fn a_cycle_reaches_the_caller_as_a_validation_issue() {
        let err = load(
            "\
version: 1
nodes:
  - id: a
    kind: command
    deps: [b]
    run: \"true\"
  - id: b
    kind: command
    deps: [a]
    run: \"true\"
",
            None,
        )
        .expect_err("cyclic");
        match err {
            FlowError::Validate(issues) => {
                assert!(
                    issues
                        .iter()
                        .any(|i| matches!(i.kind, ValidationKind::Cycle))
                );
            }
            other => panic!("expected a validation error, got {other:?}"),
        }
    }

    #[test]
    fn an_unknown_dep_reaches_the_caller_named() {
        let err = load("version: 1\nnodes:\n  - id: a\n    kind: command\n    deps: [ghost]\n    run: \"true\"\n", None)
            .expect_err("unknown dep");
        match err {
            FlowError::Validate(issues) => {
                assert!(issues.iter().any(
                    |i| matches!(&i.kind, ValidationKind::UnknownDep { dep } if dep == "ghost")
                ))
            }
            other => panic!("expected a validation error, got {other:?}"),
        }
    }

    #[test]
    fn merge_issues_reach_the_caller_too() {
        let err = load(
            "version: 1\nnodes:\n  - id: a\n    kind: agent\n    output: a.md\n    prompt: write\n",
            None,
        )
        .expect_err("no agent named anywhere");
        match err {
            FlowError::Validate(issues) => {
                assert!(
                    issues
                        .iter()
                        .any(|i| matches!(i.kind, ValidationKind::MissingAgent))
                );
            }
            other => panic!("expected a validation error, got {other:?}"),
        }
    }

    /// The split the editor needs: a rule the last stage refuses is about a
    /// flow that resolved and whose graph built, so both are there to draw —
    /// while `load`, which every path to a paid session goes through, still
    /// refuses it.
    #[test]
    fn a_graph_rule_refuses_a_run_and_still_yields_a_flow_to_draw() {
        // Two nodes writing one file: caught by `validate`, which needs the
        // resolved flow it is about.
        const CLASHING: &str = "\
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
";
        let inspected = inspect(CLASHING, None).expect("it resolves and its graph builds");
        assert_eq!(
            inspected.loaded.flow().nodes.len(),
            2,
            "both cards are there"
        );
        assert!(
            inspected
                .issues
                .iter()
                .any(|i| matches!(i.kind, ValidationKind::DuplicateOutput)),
            "{:?}",
            inspected.issues
        );
        assert!(
            matches!(load(CLASHING, None), Err(FlowError::Validate(_))),
            "and it is still not runnable"
        );
    }

    /// The earlier stages have nothing to offer an editor: a cycle leaves no
    /// graph, so `inspect` fails the same way `load` does rather than handing
    /// back half a picture.
    #[test]
    fn a_stage_before_the_last_one_leaves_nothing_to_draw() {
        const CYCLIC: &str = "\
version: 1
nodes:
  - id: a
    kind: command
    deps: [b]
    run: \"true\"
  - id: b
    kind: command
    deps: [a]
    run: \"true\"
";
        assert!(matches!(inspect(CYCLIC, None), Err(FlowError::Validate(_))));
        assert!(
            inspect("nodes: [", None).is_err(),
            "and neither does a parse error"
        );
    }
}
