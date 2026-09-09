//! Which nodes may start now: whose dependencies have all finished, and
//! which of those can run beside each other without sharing a working
//! directory. Separated from the drive loop because the loop only asks for
//! the next wave — deciding what may be in one is a question of its own.

use crate::NodeId;
use crate::graph::FlowGraph;
use crate::model::{Flow, GateFail, Node, NodeKind};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// The next set of nodes to run together: ready, in declaration order, at
/// most `parallel` of them, and **no two able to reach one working
/// directory**.
///
/// That last rule is the whole safety argument. Two agents editing one
/// directory at once corrupt each other, and no amount of care inside a
/// node prevents it — so nodes that could write in the same place are
/// simply not put in the same wave. A flow asking for eight at once still
/// gets one at a time if all eight work in the same place.
///
/// "Could reach", not "works in": a node's own directory is not the whole
/// answer, because its `on_fail` repair re-derives other nodes and
/// `repair` does not come back through here — see [`reachable_trees`].
///
/// **The unit is a directory, not a repository.** Two nodes in different
/// subdirectories of one checkout do overlap, and that is what `parallel`
/// is for. It also means they share one `.git`, so if both reach for git
/// at the same time git's own locking decides it — one of them fails
/// loudly on `index.lock` rather than quietly corrupting anything. That
/// trade is deliberate: excluding at repository granularity would serialise
/// every flow that fans out inside one checkout, which is the case this
/// whole feature exists for.
pub(super) fn take_ready_batch(
    flow: &Flow,
    graph: &FlowGraph,
    cwd: &Path,
    waiting: &mut Vec<NodeId>,
    done: &HashSet<NodeId>,
    parallel: usize,
) -> Vec<NodeId> {
    let mut batch: Vec<NodeId> = Vec::new();
    let mut taken_dirs: Vec<PathBuf> = Vec::new();
    waiting.retain(|id| {
        if batch.len() >= parallel || !deps_are_done(flow, id, done) {
            return true;
        }
        let dirs = reachable_trees(flow, graph, cwd, id);
        if dirs.iter().any(|dir| taken_dirs.contains(dir)) {
            return true;
        }
        taken_dirs.extend(dirs);
        batch.push(id.clone());
        false
    });
    batch
}

/// Every working directory this node could write in: its own, plus every
/// directory any repair reachable from it could re-derive into.
///
/// **Why the reservation is not just the node's own directory.** A gate's
/// `repair` re-derives its `rerun` closure by calling `drive` directly
/// (`super::repair`), which does not come back through this function — so a
/// member of that closure can start writing in a directory a wave sibling
/// is already using. Checking inside `repair` cannot fix it: `repair` runs
/// inside the wave's `join_all`, so waiting for a sibling there deadlocks.
/// Reserving the whole set up front is what makes the exclusion hold.
///
/// **A fixpoint, not one hop.** A member of this node's closure may itself
/// be a gate with a `rerun` of its own, and `validate`'s
/// `rerun_roots_are_ancestors` only asks that a root be an ancestor of
/// *its* gate — so a nested gate's closure can leave the outer one.
/// `super::repair`'s own note ("each member starts a fresh generation of
/// its own — that is the rule that gives a nested gate its cap back") is
/// that structure.
///
/// **Static.** `rerun_closure` is a question about the graph, so the answer
/// does not depend on what is running, which is what lets a batch reserve
/// the set before starting anything. Bounded by the node set rather than by
/// the graph being well-formed: two gates naming each other is a flow
/// `crate::validate` refuses, and the visited set means the scheduler does
/// not hang on one anyway.
fn reachable_trees(flow: &Flow, graph: &FlowGraph, cwd: &Path, id: &NodeId) -> Vec<PathBuf> {
    let mut trees: Vec<PathBuf> = Vec::new();
    let mut seen: HashSet<NodeId> = HashSet::new();
    let mut queue: Vec<NodeId> = vec![id.clone()];
    while let Some(next) = queue.pop() {
        if !seen.insert(next.clone()) {
            continue;
        }
        let Some(node) = flow.nodes.iter().find(|n| n.id == next) else {
            continue;
        };
        let dir = working_tree_of(cwd, node);
        if !trees.contains(&dir) {
            trees.push(dir);
        }
        queue.extend(
            graph
                .rerun_closure(rerun_of(node))
                .into_iter()
                .filter(|member| !seen.contains(member)),
        );
    }
    trees
}

/// The nodes this node's failure would re-derive, as the file declares
/// them. Empty for anything but a gate with a `repair` policy: `rerun`
/// lives on [`GateFail::Repair`], and an agent node's `retry` re-runs only
/// itself.
///
/// The declared roots, not the closure — the caller expands them, because
/// expanding needs the graph and this needs only the node.
fn rerun_of(node: &Node) -> &[NodeId] {
    match &node.kind {
        NodeKind::Command {
            on_fail: GateFail::Repair { rerun, .. },
            ..
        } => rerun,
        _ => &[],
    }
}

/// Which directory a node actually works in, as something two nodes can be
/// compared on.
///
/// **Resolved, not compared as written.** `a` and `./a` are one directory
/// spelled two ways, and a string comparison puts both in the same wave —
/// bypassing the one rule this whole feature rests on with a `./`. The
/// same goes for `A` and `a` on the case-insensitive filesystem macOS
/// ships by default, and for a symlink pointing at a directory already
/// taken.
///
/// `canonicalize` answers all three, because it asks the filesystem rather
/// than the spelling. It needs the directory to exist, which
/// `validate_request` has already established; if it fails anyway — the
/// directory went away mid-run — the lexical form is the fallback, and
/// erring toward *different* there only costs some overlap, never safety,
/// because a directory that is gone is not one two nodes can corrupt.
fn working_tree_of(cwd: &Path, node: &Node) -> PathBuf {
    let joined = match &node.cwd {
        Some(relative) => cwd.join(relative),
        None => cwd.to_path_buf(),
    };
    std::fs::canonicalize(&joined).unwrap_or(joined)
}

/// Whether everything this node waits on has finished.
///
/// A question about the flow, not about the run: `deps` is what the file
/// says, and asking the graph would be asking the same thing one
/// indirection away. Free-standing for the same reason — it needs no run
/// state, and a method would have implied it did.
pub(crate) fn deps_are_done(flow: &Flow, id: &NodeId, done: &HashSet<NodeId>) -> bool {
    flow.nodes
        .iter()
        .find(|n| &n.id == id)
        .is_none_or(|node| node.deps.iter().all(|dep| done.contains(dep)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::FlowGraph;
    use crate::parse::parse_flow_file;
    use crate::resolve::resolve;

    /// A flow of command nodes, each with its own `cwd` and optional
    /// `rerun`. Command nodes only: `rerun` lives on `GateFail::Repair`,
    /// which is the gate policy, so an agent node could not carry one.
    fn flow_of(spec: &[(&str, &[&str], &str, &[&str])]) -> crate::model::Flow {
        let mut text = String::from("version: 1\ndefaults:\n  parallel: 4\nnodes:\n");
        for (id, deps, cwd, rerun) in spec {
            text.push_str(&format!(
                "  - id: {id}\n    kind: command\n    run: \"true\"\n    cwd: {cwd}\n"
            ));
            if !deps.is_empty() {
                text.push_str(&format!("    deps: [{}]\n", deps.join(", ")));
            }
            if !rerun.is_empty() {
                text.push_str(&format!(
                    "    on_fail:\n      repair:\n        fix: fix {{{{attempts}}}}\n        \
                     rerun: [{}]\n        max_attempts: 2\n        wait: 0s\n",
                    rerun.join(", ")
                ));
            }
        }
        resolve(parse_flow_file(&text).expect("parses"), None).expect("resolves")
    }

    fn batch_of(flow: &crate::model::Flow, waiting: &[&str], done: &[&str]) -> Vec<NodeId> {
        let graph = FlowGraph::build(flow).expect("acyclic");
        let mut waiting: Vec<NodeId> = waiting.iter().map(|s| NodeId::from(*s)).collect();
        let done: HashSet<NodeId> = done.iter().map(|s| NodeId::from(*s)).collect();
        take_ready_batch(
            flow,
            &graph,
            std::path::Path::new("/tmp/daruda-ready-test"),
            &mut waiting,
            &done,
            flow.parallel,
        )
    }

    /// The reservation is the whole point: a gate that could re-derive a
    /// node in `b` must not share a wave with a node working in `b`, even
    /// though the gate itself works in `a`.
    #[test]
    fn a_gate_reserves_the_directories_its_repair_could_re_derive() {
        //  helper(b) --> gate(a, rerun: [helper])
        //  other(b)                                  (independent)
        let flow = flow_of(&[
            ("helper", &[], "b", &[]),
            ("gate", &["helper"], "a", &["helper"]),
            ("other", &[], "b", &[]),
        ]);
        let batch = batch_of(&flow, &["gate", "other"], &["helper"]);
        assert_eq!(
            batch,
            vec![NodeId::from("gate")],
            "the gate's repair could write in `b`, so `other` must wait"
        );
    }

    /// One hop is not enough. A member of the gate's own closure may be a
    /// gate itself, and `rerun_roots_are_ancestors` only asks that a root
    /// be an ancestor of *its* gate — so the nested closure can leave the
    /// outer one.
    #[test]
    fn a_nested_gates_rerun_is_reserved_too() {
        //  deep(c) --> inner(a, rerun: [deep]) --> outer(a, rerun: [inner])
        //  other(c)                                       (independent)
        let flow = flow_of(&[
            ("deep", &[], "c", &[]),
            ("inner", &["deep"], "a", &["deep"]),
            ("outer", &["inner"], "a", &["inner"]),
            ("other", &[], "c", &[]),
        ]);
        let batch = batch_of(&flow, &["outer", "other"], &["deep", "inner"]);
        assert_eq!(
            batch,
            vec![NodeId::from("outer")],
            "outer's repair re-derives inner, whose own repair reaches `c`"
        );
    }

    /// A node with no repair policy reserves its own directory and nothing
    /// else — the reservation must not become "the whole graph".
    #[test]
    fn a_node_with_no_repair_reserves_only_its_own_directory() {
        let flow = flow_of(&[("left", &[], "a", &[]), ("right", &[], "b", &[])]);
        let batch = batch_of(&flow, &["left", "right"], &[]);
        assert_eq!(
            batch,
            vec![NodeId::from("left"), NodeId::from("right")],
            "two plain nodes in different directories still overlap"
        );
    }

    /// Two gates naming each other is a flow `crate::validate` refuses,
    /// but the scheduler must not hang on one — the fixpoint is bounded by
    /// the node set, not by the graph being well-formed.
    #[test]
    fn gates_that_rerun_each_other_terminate() {
        let flow = flow_of(&[
            ("first", &[], "a", &["second"]),
            ("second", &[], "b", &["first"]),
        ]);
        let batch = batch_of(&flow, &["first", "second"], &[]);
        assert_eq!(
            batch,
            vec![NodeId::from("first")],
            "each names the other, so one reservation covers both directories"
        );
    }
}
