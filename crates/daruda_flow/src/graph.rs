//! Pure graph questions about a resolved flow: what order to run in, which
//! nodes precede a node, and which follow it. Isolated from scheduling so
//! the algorithms can be tested on their own.

use crate::NodeId;
use crate::model::Flow;
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::{Dfs, Reversed};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphError {
    /// A node names a dep that no node declares.
    UnknownDep { node: NodeId, dep: NodeId },
    /// The declared graph is not acyclic. Carries every node that could not
    /// be ordered — the cycle plus anything downstream of it.
    Cycle(Vec<NodeId>),
    /// Two nodes share an id, so a dep naming it is ambiguous.
    DuplicateId(NodeId),
}

impl std::fmt::Display for GraphError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GraphError::UnknownDep { node, dep } => {
                write!(
                    f,
                    "node `{node}` depends on `{dep}`, which no node declares"
                )
            }
            GraphError::Cycle(nodes) => write!(f, "the flow has a cycle among {nodes:?}"),
            GraphError::DuplicateId(id) => write!(f, "two nodes share the id `{id}`"),
        }
    }
}

impl std::error::Error for GraphError {}

/// The nodes a run will actually visit.
///
/// One question asked in three places — which nodes the scheduler walks, which
/// agents get provisioned, and which nodes are worth validating — so it has one
/// answer. Three near-agreements is how a `until` run downloads a runtime for a
/// node it will not reach, or is refused over a node it was told to skip.
///
/// `Everything` rather than a materialised set of every id: the common case
/// selects nothing, and it should not cost a set to say so.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    Everything,
    /// A node and its ancestors. Closed under deps by construction, so
    /// nothing in here can wait on something outside it.
    UpTo(std::collections::HashSet<NodeId>),
}

impl Selection {
    /// What `until` selects out of `flow`.
    ///
    /// A flow whose graph will not build selects everything. The run is going
    /// to fail on that graph anyway, and until then the safe direction is to
    /// validate more rather than less.
    pub fn of(flow: &Flow, until: Option<&NodeId>) -> Self {
        let Some(target) = until else {
            return Selection::Everything;
        };
        let Ok(graph) = FlowGraph::build(flow) else {
            return Selection::Everything;
        };
        let mut set: std::collections::HashSet<NodeId> =
            graph.ancestors(target).into_iter().collect();
        set.insert(target.clone());
        Selection::UpTo(set)
    }

    /// Whether a repair's `fix` session can happen in this run.
    ///
    /// Only a gate declares one, so a selection that stops before every gate
    /// reaches no repair — and the repair agent it would have run as is not
    /// this run's business to validate, ask for, or download.
    pub fn reaches_a_repair(&self, flow: &Flow) -> bool {
        flow.nodes.iter().any(|node| {
            self.includes(&node.id)
                && matches!(
                    &node.kind,
                    crate::model::NodeKind::Command {
                        on_fail: crate::model::GateFail::Repair { .. },
                        ..
                    }
                )
        })
    }

    pub fn includes(&self, id: &NodeId) -> bool {
        match self {
            Selection::Everything => true,
            Selection::UpTo(set) => set.contains(id),
        }
    }
}

/// Every agent a run of `flow` under `until` could open a session as.
///
/// One answer for the four questions that ask it — which agents the catalog
/// must hold, which of them can put a question to a person, which runtimes to
/// download, and which the host may refuse for this lane. Each used to walk
/// the whole flow, so a run selecting a head node was validated, refused, and
/// provisioned for a tail it never reaches.
///
/// The repair agent is in the list only when a selected gate can call for one.
/// It is not any node's agent — it is the one a `fix` session runs as — so
/// nothing else would ever name it.
pub fn agents_in_play<'f>(flow: &'f Flow, until: Option<&NodeId>) -> Vec<&'f str> {
    let selected = Selection::of(flow, until);
    let mut ids: Vec<&str> = flow
        .nodes
        .iter()
        .filter(|node| selected.includes(&node.id))
        .filter_map(|node| match &node.kind {
            crate::model::NodeKind::Agent(body) => Some(body.agent.id.as_str()),
            crate::model::NodeKind::Command { .. } => None,
        })
        .collect();
    if selected.reaches_a_repair(flow)
        && let Some(repair) = flow.default_agent.as_ref()
    {
        ids.push(repair.id.as_str());
    }
    ids
}

/// A resolved flow's dependency graph, built once and queried many times.
#[derive(Debug)]
pub struct FlowGraph {
    graph: DiGraph<NodeId, ()>,
    index: HashMap<NodeId, NodeIndex>,
    /// Declaration order, used to break topological ties so the same flow
    /// always runs in the same sequence.
    order: HashMap<NodeId, usize>,
}

impl FlowGraph {
    pub fn build(flow: &Flow) -> Result<Self, GraphError> {
        let mut graph = DiGraph::new();
        let mut index = HashMap::new();
        let mut order = HashMap::new();

        for (position, node) in flow.nodes.iter().enumerate() {
            if index.contains_key(&node.id) {
                return Err(GraphError::DuplicateId(node.id.clone()));
            }
            index.insert(node.id.clone(), graph.add_node(node.id.clone()));
            order.insert(node.id.clone(), position);
        }

        for node in &flow.nodes {
            let to = index[&node.id];
            for dep in &node.deps {
                let from = *index.get(dep).ok_or_else(|| GraphError::UnknownDep {
                    node: node.id.clone(),
                    dep: dep.clone(),
                })?;
                graph.add_edge(from, to, ());
            }
        }

        let this = Self {
            graph,
            index,
            order,
        };
        // Reject a cycle at build time so every later query can assume a DAG.
        this.try_topological_order()?;
        Ok(this)
    }

    /// Execution order: dependencies first, ties broken by declaration
    /// order.
    pub fn topological_order(&self) -> Vec<NodeId> {
        match self.try_topological_order() {
            Ok(order) => order,
            // `build` rejects cyclic graphs, so no `FlowGraph` can reach
            // this arm. Panicking beats silently returning a partial order.
            Err(_) => unreachable!("FlowGraph::build rejects cyclic graphs"),
        }
    }

    /// Kahn's algorithm with a declaration-ordered ready set, so the result
    /// is deterministic rather than dependent on petgraph's internal order.
    fn try_topological_order(&self) -> Result<Vec<NodeId>, GraphError> {
        let mut indegree: HashMap<NodeIndex, usize> = self
            .graph
            .node_indices()
            .map(|i| {
                (
                    i,
                    self.graph
                        .neighbors_directed(i, petgraph::Direction::Incoming)
                        .count(),
                )
            })
            .collect();

        let mut ready: Vec<NodeIndex> = indegree
            .iter()
            .filter(|(_, d)| **d == 0)
            .map(|(i, _)| *i)
            .collect();
        self.sort_by_declaration(&mut ready);

        let mut out = Vec::with_capacity(self.graph.node_count());
        while let Some(current) = ready.pop() {
            out.push(self.graph[current].clone());
            let mut freed = Vec::new();
            for next in self
                .graph
                .neighbors_directed(current, petgraph::Direction::Outgoing)
            {
                if let Some(d) = indegree.get_mut(&next) {
                    *d -= 1;
                    if *d == 0 {
                        freed.push(next);
                    }
                }
            }
            ready.extend(freed);
            self.sort_by_declaration(&mut ready);
        }

        if out.len() == self.graph.node_count() {
            Ok(out)
        } else {
            let ordered: std::collections::HashSet<_> = out.into_iter().collect();
            Err(GraphError::Cycle(
                self.graph
                    .node_weights()
                    .filter(|id| !ordered.contains(*id))
                    .cloned()
                    .collect(),
            ))
        }
    }

    /// Descending, because the ready set is popped from the back — so the
    /// smallest declaration index comes out first.
    fn sort_by_declaration(&self, ready: &mut [NodeIndex]) {
        ready.sort_by_key(|i| std::cmp::Reverse(self.order[&self.graph[*i]]));
    }

    /// Every node that must run before `id`, transitively.
    pub fn ancestors(&self, id: &NodeId) -> Vec<NodeId> {
        self.reachable(id, true)
    }

    /// Every node that `id` must run before, transitively.
    pub fn descendants(&self, id: &NodeId) -> Vec<NodeId> {
        self.reachable(id, false)
    }

    /// The roots plus everything downstream of them, in execution order.
    /// A node's descendants are included even when they are not on the path
    /// to the gate — their outputs were derived from code a fix is about to
    /// change, so leaving them would hand the next stage stale evidence.
    pub fn rerun_closure(&self, roots: &[NodeId]) -> Vec<NodeId> {
        if roots.is_empty() {
            return Vec::new();
        }
        let mut wanted: std::collections::HashSet<NodeId> = roots.iter().cloned().collect();
        for root in roots {
            wanted.extend(self.descendants(root));
        }
        self.topological_order()
            .into_iter()
            .filter(|id| wanted.contains(id))
            .collect()
    }

    fn reachable(&self, id: &NodeId, upstream: bool) -> Vec<NodeId> {
        let Some(&start) = self.index.get(id) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        if upstream {
            let mut dfs = Dfs::new(Reversed(&self.graph), start);
            while let Some(n) = dfs.next(Reversed(&self.graph)) {
                if n != start {
                    out.push(self.graph[n].clone());
                }
            }
        } else {
            let mut dfs = Dfs::new(&self.graph, start);
            while let Some(n) = dfs.next(&self.graph) {
                if n != start {
                    out.push(self.graph[n].clone());
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    // Diagrams use:
    //   A --> B   an edge (B declares A in `deps`)
    // If you add a test, add its diagram.
    use super::*;
    use crate::parse::parse_flow_file;
    use crate::resolve::resolve;

    /// Build a command-only flow from `id: [deps]` pairs, in declaration
    /// order — enough for every graph question, and no agent needed.
    fn flow_of(spec: &[(&str, &[&str])]) -> crate::model::Flow {
        let mut text = String::from("version: 1\nnodes:\n");
        for (id, deps) in spec {
            text.push_str(&format!(
                "  - id: {id}\n    kind: command\n    run: \"true\"\n"
            ));
            if !deps.is_empty() {
                text.push_str(&format!("    deps: [{}]\n", deps.join(", ")));
            }
        }
        resolve(parse_flow_file(&text).expect("parses"), None).expect("resolves")
    }

    //  design --> implement --> test
    #[test]
    fn topological_order_follows_the_chain() {
        let flow = flow_of(&[
            ("design", &[]),
            ("implement", &["design"]),
            ("test", &["implement"]),
        ]);
        let g = FlowGraph::build(&flow).expect("acyclic");
        assert_eq!(g.topological_order(), vec!["design", "implement", "test"]);
    }

    //  design --> implement --> docs      (declared first)
    //                       --> test      (declared second)
    #[test]
    fn ties_break_on_declaration_order_not_insertion_luck() {
        let flow = flow_of(&[
            ("design", &[]),
            ("implement", &["design"]),
            ("docs", &["implement"]),
            ("test", &["implement"]),
        ]);
        let g = FlowGraph::build(&flow).expect("acyclic");
        assert_eq!(
            g.topological_order(),
            vec!["design", "implement", "docs", "test"]
        );
    }

    #[test]
    fn ancestors_and_descendants_are_transitive_closures() {
        let flow = flow_of(&[
            ("design", &[]),
            ("implement", &["design"]),
            ("docs", &["implement"]),
            ("test", &["implement"]),
        ]);
        let g = FlowGraph::build(&flow).expect("acyclic");

        let mut anc = g.ancestors(&"test".into());
        anc.sort();
        assert_eq!(anc, vec![NodeId::from("design"), NodeId::from("implement")]);

        // `docs` is a descendant of `implement` but NOT an ancestor of
        // `test` — the distinction the rerun set depends on.
        let mut desc = g.descendants(&"implement".into());
        desc.sort();
        assert_eq!(desc, vec![NodeId::from("docs"), NodeId::from("test")]);
    }

    //  design --> implement --> docs      (a side branch)
    //                       --> test --> gate
    #[test]
    fn rerun_closure_takes_the_roots_and_everything_downstream() {
        let flow = flow_of(&[
            ("design", &[]),
            ("implement", &["design"]),
            ("docs", &["implement"]),
            ("test", &["implement"]),
            ("gate", &["test"]),
        ]);
        let g = FlowGraph::build(&flow).expect("acyclic");
        // The side branch `docs` is in the closure even though it is not on
        // the path to `gate` — its output was derived from code the fix is
        // about to change.
        assert_eq!(
            g.rerun_closure(&["implement".into()]),
            vec![
                NodeId::from("implement"),
                NodeId::from("docs"),
                NodeId::from("test"),
                NodeId::from("gate")
            ]
        );
    }

    //  a --> b        (no roots given, so nothing is downstream of nothing)
    #[test]
    fn an_empty_rerun_closure_is_empty_not_the_whole_graph() {
        let flow = flow_of(&[("a", &[]), ("b", &["a"])]);
        let g = FlowGraph::build(&flow).expect("acyclic");
        assert!(g.rerun_closure(&[]).is_empty());
    }

    //  a --> b --> a      (a cycle)
    #[test]
    fn a_cycle_is_rejected_with_the_nodes_named() {
        let flow = flow_of(&[("a", &["b"]), ("b", &["a"])]);
        match FlowGraph::build(&flow).expect_err("cyclic") {
            GraphError::Cycle(nodes) => {
                assert!(nodes.contains(&"a".into()) && nodes.contains(&"b".into()));
            }
            other => panic!("expected a cycle error, got {other:?}"),
        }
    }

    #[test]
    fn an_unknown_dep_is_rejected_by_name() {
        let flow = flow_of(&[("a", &["ghost"])]);
        let err = FlowGraph::build(&flow).expect_err("unknown dep");
        assert!(matches!(err, GraphError::UnknownDep { ref node, ref dep }
            if node == "a" && dep == "ghost"));
    }

    #[test]
    fn a_duplicate_id_is_rejected() {
        let flow = flow_of(&[("a", &[]), ("a", &[])]);
        let err = FlowGraph::build(&flow).expect_err("duplicate id");
        assert!(matches!(err, GraphError::DuplicateId(ref id) if id == "a"));
    }
}
