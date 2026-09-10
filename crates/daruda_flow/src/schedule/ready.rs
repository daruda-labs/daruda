//! Which nodes may start now: whose dependencies have all finished, and
//! which of those can run beside each other without sharing a working
//! directory. Separated from the drive loop because the loop only asks for
//! the next wave — deciding what may be in one is a question of its own.

use crate::NodeId;
use crate::graph::FlowGraph;
use crate::lock::CanonicalTree;
use crate::model::{Flow, GateFail, Node, NodeKind};
use std::collections::HashSet;
use std::path::Path;

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
) -> Batch {
    let mut batch: Vec<NodeId> = Vec::new();
    let mut held: Vec<NodeId> = Vec::new();
    let mut taken_dirs: Vec<CanonicalTree> = Vec::new();
    waiting.retain(|id| {
        if batch.len() >= parallel || !deps_are_done(flow, id, done) {
            return true;
        }
        let Some(dirs) = reachable_trees(flow, graph, cwd, id) else {
            held.push(id.clone());
            return true;
        };
        if dirs.iter().any(|dir| overlaps(dir, &taken_dirs)) {
            return true;
        }
        taken_dirs.extend(dirs);
        batch.push(id.clone());
        false
    });
    // Order matters: a batch to run is the answer even when something was
    // held, because the held node stays in `waiting` and the next wave asks
    // again. That is the retry, and it needs no loop of its own — which is
    // what keeps a hold from spinning or from sitting on the run's lock.
    if !batch.is_empty() {
        return Batch::Ready(batch);
    }
    if held.is_empty() {
        Batch::Exhausted
    } else {
        Batch::Held(held)
    }
}

/// Why a wave is what it is.
///
/// The empty case is not one answer but two, and only the type keeps a
/// caller from reading "could not start anything" as "there is nothing left
/// to start". The drive loop treats [`Batch::Exhausted`] as the end of the
/// graph — so a held node reported the same way would end the run as a
/// success without having run.
#[derive(Debug)]
pub(super) enum Batch {
    /// Nodes to run now, in declaration order.
    Ready(Vec<NodeId>),
    /// Nothing is ready and nothing is held: every node left is waiting on
    /// a dependency that will never finish. Only a cycle produces that, and
    /// `FlowGraph::build` refuses those — so this is a graph nobody could
    /// have handed us.
    Exhausted,
    /// Nothing could start, and these are why: the scheduler could not
    /// establish which directory they work in, so it does not know whether
    /// running them would collide with anything.
    Held(Vec<NodeId>),
}

/// Every working directory this node could write in: its own, the run's
/// root if it repairs at all, and every directory any repair reachable
/// from it could re-derive into.
///
/// **Why the reservation is not just the node's own directory.** A gate's
/// `repair` runs its `fix` session and then re-derives its `rerun` closure
/// by calling `drive` directly (`super::repair`), neither of which comes
/// back through this function — so a fix session, or a member of that
/// closure, can start writing in a directory a wave sibling is already
/// using. Checking inside `repair` cannot fix it: `repair` runs inside the
/// wave's `join_all`, so waiting for a sibling there deadlocks. Reserving
/// the whole set up front is what makes the exclusion hold.
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
///
/// `None` when any directory in the set cannot be resolved — the whole
/// answer is then unknown, not partly known, because a set missing one
/// member would let the node into a wave beside whatever that member
/// would have excluded.
fn reachable_trees(
    flow: &Flow,
    graph: &FlowGraph,
    cwd: &Path,
    id: &NodeId,
) -> Option<Vec<CanonicalTree>> {
    let mut trees: Vec<CanonicalTree> = Vec::new();
    let mut seen: HashSet<NodeId> = HashSet::new();
    let mut queue: Vec<NodeId> = vec![id.clone()];
    while let Some(next) = queue.pop() {
        if !seen.insert(next.clone()) {
            continue;
        }
        let Some(node) = flow.nodes.iter().find(|n| n.id == next) else {
            continue;
        };
        push(&mut trees, working_tree_of(cwd, node)?);
        let Some(rerun) = rerun_of(node) else {
            continue;
        };
        // The `fix` session's own directory. `super::repair::run_fix` opens
        // it at the run's root rather than at the gate's, so a gate working
        // in a subdirectory still reaches the root when it repairs — and
        // the flow author cannot narrow that the way they can a node's own
        // `cwd`.
        push(&mut trees, CanonicalTree::resolve(cwd).ok()?);
        queue.extend(
            graph
                .rerun_closure(rerun)
                .into_iter()
                .filter(|member| !seen.contains(member)),
        );
    }
    Some(trees)
}

/// The nodes this node's failure would re-derive, or `None` when its
/// failure re-derives nothing. `rerun` lives on [`GateFail::Repair`], and
/// an agent node's `retry` re-runs only itself.
///
/// `Some(&[])` and `None` are different answers, which is why this is not
/// just a slice: a repair with an empty `rerun` still opens a `fix`
/// session, and that session works somewhere.
///
/// The declared roots, not the closure — the caller expands them, because
/// expanding needs the graph and this needs only the node.
fn rerun_of(node: &Node) -> Option<&[NodeId]> {
    // Spelled out rather than defaulted: a node kind added with a policy
    // that re-derives anything must stop compiling here. A catch-all would
    // instead answer "nothing", quietly narrowing the reservation this
    // whole function exists to widen.
    match &node.kind {
        NodeKind::Command {
            on_fail: GateFail::Repair { rerun, .. },
            ..
        } => Some(rerun),
        NodeKind::Command {
            on_fail: GateFail::Halt,
            ..
        } => None,
        NodeKind::Agent(_) => None,
    }
}

/// Which directory a node actually works in, as something two nodes can be
/// compared on.
///
/// **Resolved, not compared as written** — which the return type now says
/// rather than this comment. `a` and `./a` are one directory spelled two
/// ways, and a string comparison puts both in the same wave, bypassing the
/// one rule this whole feature rests on with a `./`. The same goes for `A`
/// and `a` on the case-insensitive filesystem macOS ships by default, and
/// for a symlink pointing at a directory already taken.
///
/// [`CanonicalTree`] answers all three, because it asks the filesystem
/// rather than the spelling — and is the same type the lock is keyed off,
/// so the wave and the lock cannot disagree about what one tree is. `None`
/// when the filesystem cannot answer.
///
/// **No lexical fallback.** Falling back to the written form and calling
/// two paths *different* would be safe only if a failure meant the
/// directory was gone — nothing there for two nodes to corrupt. It does
/// not mean that: a live directory can fail to resolve on a network
/// filesystem timeout, a permission change on a component above it, an
/// `ELOOP`, or a stale handle. Two nodes pointing through a symlink at one
/// directory, one of which fails this moment, would then be read as
/// different and put in the same wave — the exact collision the batch
/// exists to prevent. The whole exclusion rests on this comparison, so
/// when the comparison cannot be made the answer is "unknown", not
/// "different".
fn working_tree_of(cwd: &Path, node: &Node) -> Option<CanonicalTree> {
    match &node.cwd {
        Some(relative) => CanonicalTree::resolve(&cwd.join(relative)).ok(),
        None => CanonicalTree::resolve(cwd).ok(),
    }
}

/// Add a directory to the reservation, once.
fn push(trees: &mut Vec<CanonicalTree>, dir: CanonicalTree) {
    if !trees.contains(&dir) {
        trees.push(dir);
    }
}

/// Whether working in `dir` could touch anything already reserved.
///
/// Containment, not equality: a node at the run's root writes in every
/// subdirectory beneath it, so it is in "the same place" as a node working
/// in one of them even though the two paths are different strings. The
/// `working_dirs` module says the same thing about locks — a holder of the
/// root and a holder of `sub/` do not exclude each other, and both write to
/// `sub/`.
///
/// Both directions, because the reservation may be made in either order.
/// Sound on resolved paths only, and [`CanonicalTree`] is what makes that a
/// precondition the caller cannot skip rather than a line here: a `&Path`
/// does not go where one is wanted. `starts_with` then compares components,
/// so `/a/bc` does not contain `/a/b`.
fn overlaps(dir: &CanonicalTree, taken: &[CanonicalTree]) -> bool {
    taken.iter().any(|other| {
        dir.as_path().starts_with(other.as_path()) || other.as_path().starts_with(dir.as_path())
    })
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

    /// One node in a fixture. Named rather than a tuple because four
    /// positional fields, two of them slices, read as noise at the call
    /// site — and clippy says the same.
    struct N {
        id: &'static str,
        deps: &'static [&'static str],
        cwd: &'static str,
        /// `None` repairs nothing; `Some(&[])` repairs but re-derives
        /// nothing — and still opens a `fix` session.
        rerun: Option<&'static [&'static str]>,
    }

    /// The common shape: no deps, no repair.
    fn plain(id: &'static str, cwd: &'static str) -> N {
        N {
            id,
            deps: &[],
            cwd,
            rerun: None,
        }
    }

    /// A flow of command nodes, each with its own `cwd` and an optional
    /// repair policy. Command nodes only: `rerun` lives on
    /// `GateFail::Repair`, which is the gate policy, so an agent node could
    /// not carry one.
    fn flow_of(spec: &[N]) -> crate::model::Flow {
        let mut text = String::from("version: 1\ndefaults:\n  parallel: 4\nnodes:\n");
        for N {
            id,
            deps,
            cwd,
            rerun,
        } in spec
        {
            text.push_str(&format!(
                "  - id: {id}\n    kind: command\n    run: \"true\"\n    cwd: {cwd}\n"
            ));
            if !deps.is_empty() {
                text.push_str(&format!("    deps: [{}]\n", deps.join(", ")));
            }
            if let Some(rerun) = rerun {
                text.push_str("    on_fail:\n      repair:\n        fix: fix {{attempts}}\n");
                if !rerun.is_empty() {
                    text.push_str(&format!("        rerun: [{}]\n", rerun.join(", ")));
                }
                text.push_str("        max_attempts: 2\n        wait: 0s\n");
            }
        }
        resolve(parse_flow_file(&text).expect("parses"), None).expect("resolves")
    }

    /// The directories a node may name here. Made for real, because
    /// `working_tree_of` asks the filesystem and a missing one is a
    /// different answer now — `missing` is the one deliberately absent.
    const DIRS: [&str; 3] = ["a", "b", "c"];

    fn batch_of(flow: &crate::model::Flow, waiting: &[&str], done: &[&str]) -> Batch {
        let dir = tempfile::tempdir().expect("tempdir");
        for sub in DIRS {
            std::fs::create_dir_all(dir.path().join(sub)).expect("mkdir");
        }
        let graph = FlowGraph::build(flow).expect("acyclic");
        let mut waiting: Vec<NodeId> = waiting.iter().map(|s| NodeId::from(*s)).collect();
        let done: HashSet<NodeId> = done.iter().map(|s| NodeId::from(*s)).collect();
        take_ready_batch(flow, &graph, dir.path(), &mut waiting, &done, flow.parallel)
    }

    /// The ids a batch would run, for a test that only cares about those.
    fn ready(batch: Batch) -> Vec<NodeId> {
        match batch {
            Batch::Ready(ids) => ids,
            other => panic!("expected a batch to run, got {other:?}"),
        }
    }

    /// The reservation is the whole point: a gate that could re-derive a
    /// node in `b` must not share a wave with a node working in `b`, even
    /// though the gate itself works in `a`.
    #[test]
    fn a_gate_reserves_the_directories_its_repair_could_re_derive() {
        //  helper(b) --> gate(a, rerun: [helper])
        //  other(b)                                  (independent)
        let flow = flow_of(&[
            plain("helper", "b"),
            N {
                id: "gate",
                deps: &["helper"],
                cwd: "a",
                rerun: Some(&["helper"]),
            },
            plain("other", "b"),
        ]);
        let batch = ready(batch_of(&flow, &["gate", "other"], &["helper"]));
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
            plain("deep", "c"),
            N {
                id: "inner",
                deps: &["deep"],
                cwd: "a",
                rerun: Some(&["deep"]),
            },
            N {
                id: "outer",
                deps: &["inner"],
                cwd: "a",
                rerun: Some(&["inner"]),
            },
            plain("other", "c"),
        ]);
        let batch = ready(batch_of(&flow, &["outer", "other"], &["deep", "inner"]));
        assert_eq!(
            batch,
            vec![NodeId::from("outer")],
            "outer's repair re-derives inner, whose own repair reaches `c`"
        );
    }

    /// A node at the root can write in every subdirectory under it, so
    /// "the same place" is containment and not string equality. The
    /// neighbouring `working_dirs` module states the same thing about
    /// locks: a holder of the root and a holder of `sub/` do not exclude
    /// each other, and both write to `sub/`.
    #[test]
    fn a_node_at_the_root_does_not_share_a_wave_with_one_below_it() {
        let flow = flow_of(&[plain("root", "."), plain("under", "a")]);
        assert_eq!(
            ready(batch_of(&flow, &["root", "under"], &[])),
            vec![NodeId::from("root")],
            "the root node can write in `a`, so `under` must wait"
        );
    }

    /// A gate's `fix` session runs at the run's own root, whatever
    /// directory the gate itself works in — so the root is part of what a
    /// repair could write in, and a node working there cannot share the
    /// wave.
    #[test]
    fn a_gate_reserves_the_root_its_fix_session_runs_in() {
        //  gate(a, repair) — its fix runs at the root
        //  other()        — works at the root itself
        let flow = flow_of(&[
            N {
                id: "gate",
                deps: &[],
                cwd: "a",
                rerun: Some(&[]),
            },
            plain("other", "."),
        ]);
        assert_eq!(
            ready(batch_of(&flow, &["gate", "other"], &[])),
            vec![NodeId::from("gate")],
            "the fix session works at the root, so `other` must wait"
        );
    }

    /// A node with no repair policy reserves its own directory and nothing
    /// else — the reservation must not become "the whole graph".
    #[test]
    fn a_node_with_no_repair_reserves_only_its_own_directory() {
        let flow = flow_of(&[plain("left", "a"), plain("right", "b")]);
        let batch = ready(batch_of(&flow, &["left", "right"], &[]));
        assert_eq!(
            batch,
            vec![NodeId::from("left"), NodeId::from("right")],
            "two plain nodes in different directories still overlap"
        );
    }

    /// A directory the filesystem cannot resolve is not a directory the
    /// batch may guess about: the node is held, not treated as working
    /// somewhere of its own.
    #[test]
    fn a_directory_that_cannot_be_resolved_holds_the_node() {
        let flow = flow_of(&[plain("lonely", "missing")]);
        match batch_of(&flow, &["lonely"], &[]) {
            Batch::Held(ids) => assert_eq!(ids, vec![NodeId::from("lonely")]),
            other => panic!("an unresolvable directory must hold, got {other:?}"),
        }
    }

    /// A hold does not stop a wave that has other work: the held node stays
    /// in `waiting`, so the next wave asks again. That is the retry, and it
    /// is why no hold loop is needed.
    #[test]
    fn a_held_node_leaves_the_rest_of_the_wave_alone() {
        let flow = flow_of(&[plain("lonely", "missing"), plain("fine", "a")]);
        assert_eq!(
            ready(batch_of(&flow, &["lonely", "fine"], &[])),
            vec![NodeId::from("fine")],
            "the resolvable node still runs"
        );
    }

    /// Nothing ready and nothing held is the end of the graph, which is a
    /// different answer from "could not start anything".
    #[test]
    fn a_graph_with_nothing_left_is_exhausted_not_held() {
        let flow = flow_of(&[
            plain("first", "a"),
            N {
                id: "second",
                deps: &["first"],
                cwd: "b",
                rerun: None,
            },
        ]);
        match batch_of(&flow, &["second"], &[]) {
            Batch::Exhausted => {}
            other => panic!("a dependency nobody will finish is exhaustion, got {other:?}"),
        }
    }

    /// Two gates naming each other is a flow `crate::validate` refuses,
    /// but the scheduler must not hang on one — the fixpoint is bounded by
    /// the node set, not by the graph being well-formed.
    #[test]
    fn gates_that_rerun_each_other_terminate() {
        let flow = flow_of(&[
            N {
                id: "first",
                deps: &[],
                cwd: "a",
                rerun: Some(&["second"]),
            },
            N {
                id: "second",
                deps: &[],
                cwd: "b",
                rerun: Some(&["first"]),
            },
        ]);
        let batch = ready(batch_of(&flow, &["first", "second"], &[]));
        assert_eq!(
            batch,
            vec![NodeId::from("first")],
            "each names the other, so one reservation covers both directories"
        );
    }
}
