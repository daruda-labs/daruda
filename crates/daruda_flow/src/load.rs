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

/// Parse, merge, build the graph, and run the graph-dependent rules.
pub fn load(text: &str) -> Result<LoadedFlow, FlowError> {
    let file = parse::parse_flow_file(text)?;
    let flow = resolve::resolve(file).map_err(FlowError::Validate)?;
    let graph = FlowGraph::build(&flow).map_err(|e| FlowError::Validate(vec![graph_issue(e)]))?;
    let issues = validate::validate(&flow, &graph);
    if issues.is_empty() {
        Ok(LoadedFlow { flow, graph })
    } else {
        Err(FlowError::Validate(issues))
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
        )
        .expect("valid flow loads");
        assert_eq!(loaded.flow().nodes.len(), 2);
        assert_eq!(loaded.graph().topological_order(), vec!["a", "b"]);
    }

    #[test]
    fn a_parse_error_stays_a_parse_error() {
        assert!(matches!(load("nodes: []"), Err(FlowError::Parse(_))));
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
        let err = load("version: 1\nnodes:\n  - id: a\n    kind: command\n    deps: [ghost]\n    run: \"true\"\n")
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
}
