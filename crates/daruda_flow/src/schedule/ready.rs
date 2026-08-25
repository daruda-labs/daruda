//! Which nodes may start now: whose dependencies have all finished, and
//! which of those can run beside each other without sharing a working
//! directory. Separated from the drive loop because the loop only asks for
//! the next wave — deciding what may be in one is a question of its own.

use crate::NodeId;
use crate::model::{Flow, Node};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// The next set of nodes to run together: ready, in declaration order, at
/// most `parallel` of them, and **no two sharing a working directory**.
///
/// That last rule is the whole safety argument. Two agents editing one tree
/// at once corrupt each other, and no amount of care inside a node prevents
/// it — so nodes that would share a directory are simply not put in the
/// same wave. A flow asking for eight at once still gets one at a time if
/// all eight work in the same place.
pub(super) fn take_ready_batch(
    flow: &Flow,
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
        let Some(node) = flow.nodes.iter().find(|n| &n.id == id) else {
            return true;
        };
        let dir = working_tree_of(cwd, node);
        if taken_dirs.contains(&dir) {
            return true;
        }
        taken_dirs.push(dir);
        batch.push(id.clone());
        false
    });
    batch
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
