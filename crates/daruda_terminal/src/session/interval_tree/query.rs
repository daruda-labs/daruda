//! Interval-tree queries: overlap and point-in-range lookups.
//!
//! Both queries walk the augmented tree and prune subtrees whose `max_end`
//! lies entirely before the query range, achieving `O(log n + k)` where
//! `k` is the number of results.

use super::coord::{LineCoord, LineRange};
use super::tree::{IntervalTree, MarkId, NodeIdx};

/// Borrowed view of a mark for iteration / query results.
pub struct MarkRef<'a, P> {
    /// Stable identity of the mark.
    pub id: MarkId,
    /// The line range the mark covers.
    pub range: LineRange,
    /// Reference to the mark's payload stored in the tree arena.
    pub payload: &'a P,
}

impl<P> IntervalTree<P> {
    /// Return an iterator over all marks whose range overlaps `range`.
    /// Runs in O(log n + k) where k is the number of results.
    pub fn overlap(&self, range: LineRange) -> OverlapIter<'_, P> {
        OverlapIter {
            tree: self,
            stack: self.root.map(|r| vec![r]).unwrap_or_default(),
            query: range,
        }
    }

    /// Return an iterator over all marks whose range contains `line`.
    /// Convenience wrapper around [`overlap`] with a point range.
    pub fn at_line(&self, line: LineCoord) -> OverlapIter<'_, P> {
        self.overlap(LineRange {
            start: line,
            end: line,
        })
    }
}

/// DFS iterator over overlapping marks, pruning subtrees whose `max_end`
/// lies entirely before the query range.
///
/// DFS over the interval tree, descending only into subtrees whose
/// `max_end >= query.start`. Within those subtrees a node is yielded iff
/// its own `range` overlaps `query`; the right child is pruned when the
/// current node's `range.start > query.end` (BST property: every right
/// descendant has a start at least as large).
pub struct OverlapIter<'a, P> {
    tree: &'a IntervalTree<P>,
    stack: Vec<NodeIdx>,
    query: LineRange,
}

impl<'a, P> Iterator for OverlapIter<'a, P> {
    type Item = MarkRef<'a, P>;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(idx) = self.stack.pop() {
            let node = self.tree.node(idx);

            if let Some(l) = node.left {
                let l_node = self.tree.node(l);
                if l_node.max_end >= self.query.start {
                    self.stack.push(l);
                }
            }
            if node.range.start <= self.query.end
                && let Some(r) = node.right
            {
                let r_node = self.tree.node(r);
                if r_node.max_end >= self.query.start {
                    self.stack.push(r);
                }
            }

            if node.range.overlaps(&self.query) {
                return Some(MarkRef {
                    id: node.id,
                    range: node.range,
                    payload: &node.payload,
                });
            }
        }
        None
    }
}
