//! Augmented red-black interval tree, arena-backed.
//!
//! Each node carries a `LineRange` keyed on `range.start` for ordering and
//! a `max_end` augmentation (the maximum `range.end` across the subtree).
//! Rotations, insert-fixup, and delete-fixup follow CLRS 3e ch. 13.
//!
//! No `Rc` / `Box` / `RefCell`. Nodes live in `nodes: Vec<Option<Node<P>>>`
//! and reference each other through `NodeIdx = usize` indices. Freed slots
//! are recycled via a `free` stack so handles stay dense.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::coord::{LineCoord, LineRange};
use super::persistence::MarkRecordSink;

/// Opaque identity of a mark. Stable across rotations and re-balances.
///
/// `#[serde(transparent)]` renders this as a bare integer on the wire,
/// matching the schema documented in `persistence.rs`.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MarkId(pub(crate) u64);

pub(crate) type NodeIdx = usize;

#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub(crate) enum Color {
    Red,
    Black,
}

pub(crate) struct Node<P> {
    pub(crate) id: MarkId,
    pub(crate) range: LineRange,
    pub(crate) max_end: LineCoord,
    pub(crate) payload: P,
    pub(crate) color: Color,
    pub(crate) left: Option<NodeIdx>,
    pub(crate) right: Option<NodeIdx>,
    pub(crate) parent: Option<NodeIdx>,
}

/// Augmented red-black interval tree keyed on `LineRange`.
///
/// Stores an arbitrary payload `P` per node. Supports O(log n) insert,
/// remove, and range-overlap queries; O(1) payload lookup by `MarkId` via
/// the `by_id` hash index. Nodes are arena-backed (`Vec<Option<Node<P>>>`),
/// avoiding heap allocation per node and keeping `NodeIdx` handles stable
/// across rotations.
pub struct IntervalTree<P> {
    pub(crate) root: Option<NodeIdx>,
    pub(crate) nodes: Vec<Option<Node<P>>>,
    free: Vec<NodeIdx>,
    by_id: HashMap<MarkId, NodeIdx>,
    pub(crate) next_id: u64,
    /// Controls whether `iter_visible()` filters by payload visibility.
    ///
    /// This flag lives on the generic struct to avoid threading it through
    /// every query call site, but only `impl IntervalTree<MarkPayload>` exposes
    /// a setter. Future non-MarkPayload payload types will inherit the field
    /// dormant — acceptable as long as SP-1 stays the only consumer.
    pub(crate) alt_screen_active: bool,
    /// Optional NDJSON sink. Populated only via the `IntervalTree<MarkPayload>`
    /// specialized API; the generic struct holds it but never emits records on
    /// its own (preserves payload-agnostic layering — see `persistence.rs`).
    pub(crate) sink: Option<Box<dyn MarkRecordSink>>,
    /// Monotonic record sequence; incremented before each `MarkRecord` is built
    /// so the first emitted record carries `seq = 1`.
    pub(crate) seq: u64,
}

impl<P> Default for IntervalTree<P> {
    fn default() -> Self {
        Self::new()
    }
}

impl<P> IntervalTree<P> {
    /// Create an empty tree with no sink and no nodes allocated.
    pub fn new() -> Self {
        Self {
            root: None,
            nodes: Vec::new(),
            free: Vec::new(),
            by_id: HashMap::new(),
            next_id: 0,
            alt_screen_active: false,
            sink: None,
            seq: 0,
        }
    }

    /// Install an NDJSON sink. Replaces any previously configured sink. The
    /// sink is only ever exercised by the `IntervalTree<MarkPayload>` wrapper
    /// methods (`insert_payload`, `update_payload`, ...) — generic mutations
    /// stay silent.
    pub fn set_sink(&mut self, sink: Box<dyn MarkRecordSink>) {
        self.sink = Some(sink);
    }

    /// Detach the current sink, if any. Useful before bulk-loading via
    /// [`super::persistence::replay`] so reconstruction does not echo records
    /// back into a freshly-opened log.
    pub fn take_sink(&mut self) -> Option<Box<dyn MarkRecordSink>> {
        self.sink.take()
    }

    /// Replay-only insert that preserves a caller-supplied `MarkId`. Bypasses
    /// the sink entirely so reconstruction never duplicates records into the
    /// log. Panics if `id` is already present.
    pub(crate) fn insert_with_id(&mut self, id: MarkId, range: LineRange, payload: P) {
        assert!(
            !self.by_id.contains_key(&id),
            "insert_with_id called with already-present MarkId"
        );

        let new_idx = self.alloc(Node {
            id,
            range,
            max_end: range.end,
            payload,
            color: Color::Red,
            left: None,
            right: None,
            parent: None,
        });
        self.by_id.insert(id, new_idx);

        self.bst_link(new_idx, range, id);

        self.fix_max_end_upwards(Some(new_idx));
        self.insert_fixup(new_idx);
    }

    /// Ensure `next_id` is at least `n`. Used by replay to skip past every id
    /// seen in the log so subsequent native inserts never collide. Bypasses
    /// the sink (which is irrelevant — this only touches the counter).
    pub(crate) fn set_next_id_at_least(&mut self, n: u64) {
        if self.next_id < n {
            self.next_id = n;
        }
    }

    /// Number of live marks currently stored in the tree.
    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    /// True when the tree holds no marks.
    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    /// True when a mark with `id` is currently present (not removed).
    pub fn contains(&self, id: MarkId) -> bool {
        self.by_id.contains_key(&id)
    }

    pub(crate) fn node(&self, idx: NodeIdx) -> &Node<P> {
        self.nodes[idx]
            .as_ref()
            .expect("node index points to vacant slot")
    }

    fn node_mut(&mut self, idx: NodeIdx) -> &mut Node<P> {
        self.nodes[idx]
            .as_mut()
            .expect("node index points to vacant slot")
    }

    fn alloc(&mut self, node: Node<P>) -> NodeIdx {
        if let Some(idx) = self.free.pop() {
            self.nodes[idx] = Some(node);
            idx
        } else {
            self.nodes.push(Some(node));
            self.nodes.len() - 1
        }
    }

    fn dealloc(&mut self, idx: NodeIdx) -> Node<P> {
        let node = self.nodes[idx]
            .take()
            .expect("dealloc on already-vacant slot");
        self.free.push(idx);
        node
    }

    /// Insert a new mark with `range` and `payload`, returning its freshly
    /// allocated `MarkId`. The id is stable for the lifetime of the mark.
    pub fn insert(&mut self, range: LineRange, payload: P) -> MarkId {
        let id = MarkId(self.next_id);
        self.next_id += 1;

        let new_idx = self.alloc(Node {
            id,
            range,
            max_end: range.end,
            payload,
            color: Color::Red,
            left: None,
            right: None,
            parent: None,
        });
        self.by_id.insert(id, new_idx);

        self.bst_link(new_idx, range, id);

        self.fix_max_end_upwards(Some(new_idx));
        self.insert_fixup(new_idx);
        id
    }

    /// Link `new_idx` into the BST using `(range.start, id)` as the ordering
    /// key. Caller is responsible for max_end and RB fixups afterwards. The
    /// id tie-breaker keeps nodes with equal starts in stable insertion order.
    fn bst_link(&mut self, new_idx: NodeIdx, range: LineRange, id: MarkId) {
        let mut parent: Option<NodeIdx> = None;
        let mut cur = self.root;
        while let Some(c) = cur {
            parent = Some(c);
            let go_left = (range.start, id.0) < (self.node(c).range.start, self.node(c).id.0);
            cur = if go_left {
                self.node(c).left
            } else {
                self.node(c).right
            };
        }
        self.node_mut(new_idx).parent = parent;
        match parent {
            None => self.root = Some(new_idx),
            Some(p) => {
                let go_left = (range.start, id.0) < (self.node(p).range.start, self.node(p).id.0);
                if go_left {
                    self.node_mut(p).left = Some(new_idx);
                } else {
                    self.node_mut(p).right = Some(new_idx);
                }
            }
        }
    }

    /// Remove the mark with `id`, returning its payload. Returns `None` when
    /// no mark with that id exists.
    pub fn remove(&mut self, id: MarkId) -> Option<P> {
        let idx = self.by_id.remove(&id)?;
        self.delete_node(idx);
        let node = self.dealloc(idx);
        Some(node.payload)
    }

    /// Mutate the payload of the mark with `id` in place via `f`. Returns
    /// `true` on success; `false` when no mark with `id` exists.
    pub fn update<F: FnOnce(&mut P)>(&mut self, id: MarkId, f: F) -> bool {
        let Some(&idx) = self.by_id.get(&id) else {
            return false;
        };
        f(&mut self.node_mut(idx).payload);
        true
    }

    /// Move the mark with `id` to `new_range`, re-inserting it at the
    /// correct BST position. Returns `true` on success; `false` when the id
    /// does not exist.
    pub fn update_range(&mut self, id: MarkId, new_range: LineRange) -> bool {
        let Some(&idx) = self.by_id.get(&id) else {
            return false;
        };
        // Range changes affect BST ordering and the max_end augmentation;
        // a delete + re-insert with the same id is the simplest correct
        // path, and matches the contract documented in the public API.
        let old_node_payload = {
            self.delete_node(idx);
            self.by_id.remove(&id);
            let node = self.dealloc(idx);
            node.payload
        };

        let new_idx = self.alloc(Node {
            id,
            range: new_range,
            max_end: new_range.end,
            payload: old_node_payload,
            color: Color::Red,
            left: None,
            right: None,
            parent: None,
        });
        self.by_id.insert(id, new_idx);

        self.bst_link(new_idx, new_range, id);

        self.fix_max_end_upwards(Some(new_idx));
        self.insert_fixup(new_idx);
        true
    }

    /// Look up the payload for `id` in O(1) via the `by_id` index.
    /// Returns `None` if the id has been removed or never existed.
    pub fn get_payload(&self, id: MarkId) -> Option<&P> {
        let idx = *self.by_id.get(&id)?;
        self.nodes.get(idx)?.as_ref().map(|n| &n.payload)
    }

    /// In-order iterator over all marks, yielding `MarkRef`s in ascending
    /// `(range.start, id)` order.
    pub fn iter(&self) -> InOrderIter<'_, P> {
        InOrderIter::new(self)
    }

    // ----- Helpers -----

    pub(crate) fn color(&self, idx: Option<NodeIdx>) -> Color {
        match idx {
            None => Color::Black,
            Some(i) => self.node(i).color,
        }
    }

    fn set_color(&mut self, idx: Option<NodeIdx>, c: Color) {
        if let Some(i) = idx {
            self.node_mut(i).color = c;
        }
    }

    fn parent_of(&self, idx: Option<NodeIdx>) -> Option<NodeIdx> {
        idx.and_then(|i| self.node(i).parent)
    }

    fn left_of(&self, idx: Option<NodeIdx>) -> Option<NodeIdx> {
        idx.and_then(|i| self.node(i).left)
    }

    fn right_of(&self, idx: Option<NodeIdx>) -> Option<NodeIdx> {
        idx.and_then(|i| self.node(i).right)
    }

    fn max_end_of(&self, idx: Option<NodeIdx>) -> Option<LineCoord> {
        idx.map(|i| self.node(i).max_end)
    }

    fn recompute_max_end(&mut self, idx: NodeIdx) {
        let left = self.node(idx).left;
        let right = self.node(idx).right;
        let own = self.node(idx).range.end;
        let mut max = own;
        if let Some(m) = self.max_end_of(left)
            && m > max
        {
            max = m;
        }
        if let Some(m) = self.max_end_of(right)
            && m > max
        {
            max = m;
        }
        self.node_mut(idx).max_end = max;
    }

    fn fix_max_end_upwards(&mut self, mut cur: Option<NodeIdx>) {
        while let Some(i) = cur {
            self.recompute_max_end(i);
            cur = self.node(i).parent;
        }
    }

    fn left_rotate(&mut self, x: NodeIdx) {
        let y = self
            .node(x)
            .right
            .expect("left_rotate requires a right child");
        // x.right = y.left
        let y_left = self.node(y).left;
        self.node_mut(x).right = y_left;
        if let Some(yl) = y_left {
            self.node_mut(yl).parent = Some(x);
        }
        // y.parent = x.parent
        let xp = self.node(x).parent;
        self.node_mut(y).parent = xp;
        match xp {
            None => self.root = Some(y),
            Some(p) => {
                if self.node(p).left == Some(x) {
                    self.node_mut(p).left = Some(y);
                } else {
                    self.node_mut(p).right = Some(y);
                }
            }
        }
        // y.left = x
        self.node_mut(y).left = Some(x);
        self.node_mut(x).parent = Some(y);

        // Augmentation: x first (now lower), then y.
        self.recompute_max_end(x);
        self.recompute_max_end(y);
    }

    fn right_rotate(&mut self, x: NodeIdx) {
        let y = self
            .node(x)
            .left
            .expect("right_rotate requires a left child");
        let y_right = self.node(y).right;
        self.node_mut(x).left = y_right;
        if let Some(yr) = y_right {
            self.node_mut(yr).parent = Some(x);
        }
        let xp = self.node(x).parent;
        self.node_mut(y).parent = xp;
        match xp {
            None => self.root = Some(y),
            Some(p) => {
                if self.node(p).right == Some(x) {
                    self.node_mut(p).right = Some(y);
                } else {
                    self.node_mut(p).left = Some(y);
                }
            }
        }
        self.node_mut(y).right = Some(x);
        self.node_mut(x).parent = Some(y);

        self.recompute_max_end(x);
        self.recompute_max_end(y);
    }

    fn insert_fixup(&mut self, mut z: NodeIdx) {
        while let Some(p) = self.node(z).parent
            && self.node(p).color == Color::Red
        {
            let gp = match self.node(p).parent {
                Some(g) => g,
                None => break,
            };
            if Some(p) == self.node(gp).left {
                let uncle = self.node(gp).right;
                if self.color(uncle) == Color::Red {
                    self.set_color(Some(p), Color::Black);
                    self.set_color(uncle, Color::Black);
                    self.set_color(Some(gp), Color::Red);
                    z = gp;
                } else {
                    if Some(z) == self.node(p).right {
                        z = p;
                        self.left_rotate(z);
                    }
                    let p2 = self.node(z).parent.expect("z must have parent here");
                    let gp2 = self.node(p2).parent.expect("p must have parent here");
                    self.set_color(Some(p2), Color::Black);
                    self.set_color(Some(gp2), Color::Red);
                    self.right_rotate(gp2);
                }
            } else {
                let uncle = self.node(gp).left;
                if self.color(uncle) == Color::Red {
                    self.set_color(Some(p), Color::Black);
                    self.set_color(uncle, Color::Black);
                    self.set_color(Some(gp), Color::Red);
                    z = gp;
                } else {
                    if Some(z) == self.node(p).left {
                        z = p;
                        self.right_rotate(z);
                    }
                    let p2 = self.node(z).parent.expect("z must have parent here");
                    let gp2 = self.node(p2).parent.expect("p must have parent here");
                    self.set_color(Some(p2), Color::Black);
                    self.set_color(Some(gp2), Color::Red);
                    self.left_rotate(gp2);
                }
            }
        }
        if let Some(r) = self.root {
            self.node_mut(r).color = Color::Black;
        }
    }

    /// Replace the subtree rooted at `u` with the subtree rooted at `v`
    /// (CLRS RB-TRANSPLANT). Does NOT touch `v`'s own children.
    fn transplant(&mut self, u: NodeIdx, v: Option<NodeIdx>) {
        let up = self.node(u).parent;
        match up {
            None => self.root = v,
            Some(p) => {
                if self.node(p).left == Some(u) {
                    self.node_mut(p).left = v;
                } else {
                    self.node_mut(p).right = v;
                }
            }
        }
        if let Some(vi) = v {
            self.node_mut(vi).parent = up;
        }
    }

    fn minimum(&self, mut idx: NodeIdx) -> NodeIdx {
        while let Some(l) = self.node(idx).left {
            idx = l;
        }
        idx
    }

    /// CLRS RB-DELETE with deferred max_end fixup. Leaves the arena slot
    /// for `z` in place — the caller deallocates after extracting the
    /// payload.
    fn delete_node(&mut self, z: NodeIdx) {
        let z_left = self.node(z).left;
        let z_right = self.node(z).right;

        // `y` is the node actually leaving the tree (z itself when it has
        // <2 children, otherwise z's successor). `y_original_color` decides
        // whether a fixup pass is needed.
        let mut y = z;
        let mut y_original_color = self.node(y).color;
        let x: Option<NodeIdx>;
        let x_parent: Option<NodeIdx>;

        if z_left.is_none() {
            x = z_right;
            let z_parent = self.node(z).parent;
            self.transplant(z, z_right);
            x_parent = z_parent;
            // max_end fixup starts at z's old parent (z is gone).
            self.fix_max_end_upwards(x_parent);
        } else if z_right.is_none() {
            x = z_left;
            let z_parent = self.node(z).parent;
            self.transplant(z, z_left);
            x_parent = z_parent;
            self.fix_max_end_upwards(x_parent);
        } else {
            // Two-children case: y is z's successor (minimum of right subtree).
            let zr = z_right.expect("right child present in two-children branch");
            y = self.minimum(zr);
            y_original_color = self.node(y).color;
            x = self.node(y).right;
            if self.node(y).parent == Some(z) {
                // y is z's direct right child — x's parent will be y once
                // we splice y into z's position.
                if let Some(xi) = x {
                    self.node_mut(xi).parent = Some(y);
                }
                x_parent = Some(y);
            } else {
                let y_parent = self
                    .node(y)
                    .parent
                    .expect("non-direct successor has parent");
                self.transplant(y, self.node(y).right);
                // y.right := z.right
                self.node_mut(y).right = z_right;
                if let Some(zr2) = z_right {
                    self.node_mut(zr2).parent = Some(y);
                }
                x_parent = Some(y_parent);
            }
            self.transplant(z, Some(y));
            // y.left := z.left
            self.node_mut(y).left = z_left;
            if let Some(zl2) = z_left {
                self.node_mut(zl2).parent = Some(y);
            }
            let z_color = self.node(z).color;
            self.node_mut(y).color = z_color;

            // Recompute max_end along the path that changed structure.
            // Starting from x_parent walks through y (the new occupant of
            // z's slot) and on up to the root.
            self.fix_max_end_upwards(x_parent);
        }

        if y_original_color == Color::Black {
            self.delete_fixup(x, x_parent);
        }
    }

    /// CLRS RB-DELETE-FIXUP working with explicit `(x, x_parent)` so a
    /// null `x` (which has no parent of its own) still has the context
    /// the fixup needs.
    fn delete_fixup(&mut self, mut x: Option<NodeIdx>, mut x_parent: Option<NodeIdx>) {
        while x != self.root && self.color(x) == Color::Black {
            let parent = match x_parent {
                Some(p) => p,
                None => break,
            };
            let is_left = self.node(parent).left == x;
            if is_left {
                let mut w = self
                    .node(parent)
                    .right
                    .expect("sibling exists when fixing extra-black left child");
                if self.color(Some(w)) == Color::Red {
                    self.set_color(Some(w), Color::Black);
                    self.set_color(Some(parent), Color::Red);
                    self.left_rotate(parent);
                    w = self
                        .node(parent)
                        .right
                        .expect("rotation guarantees a new black sibling");
                }
                let wl = self.left_of(Some(w));
                let wr = self.right_of(Some(w));
                if self.color(wl) == Color::Black && self.color(wr) == Color::Black {
                    self.set_color(Some(w), Color::Red);
                    x = Some(parent);
                    x_parent = self.parent_of(x);
                } else {
                    if self.color(wr) == Color::Black {
                        if let Some(wli) = wl {
                            self.set_color(Some(wli), Color::Black);
                        }
                        self.set_color(Some(w), Color::Red);
                        self.right_rotate(w);
                        w = self
                            .node(parent)
                            .right
                            .expect("rotation preserves sibling slot");
                    }
                    let parent_color = self.node(parent).color;
                    self.set_color(Some(w), parent_color);
                    self.set_color(Some(parent), Color::Black);
                    let new_wr = self.right_of(Some(w));
                    if let Some(wri) = new_wr {
                        self.set_color(Some(wri), Color::Black);
                    }
                    self.left_rotate(parent);
                    x = self.root;
                    x_parent = None;
                }
            } else {
                let mut w = self
                    .node(parent)
                    .left
                    .expect("sibling exists when fixing extra-black right child");
                if self.color(Some(w)) == Color::Red {
                    self.set_color(Some(w), Color::Black);
                    self.set_color(Some(parent), Color::Red);
                    self.right_rotate(parent);
                    w = self
                        .node(parent)
                        .left
                        .expect("rotation guarantees a new black sibling");
                }
                let wl = self.left_of(Some(w));
                let wr = self.right_of(Some(w));
                if self.color(wl) == Color::Black && self.color(wr) == Color::Black {
                    self.set_color(Some(w), Color::Red);
                    x = Some(parent);
                    x_parent = self.parent_of(x);
                } else {
                    if self.color(wl) == Color::Black {
                        if let Some(wri) = wr {
                            self.set_color(Some(wri), Color::Black);
                        }
                        self.set_color(Some(w), Color::Red);
                        self.left_rotate(w);
                        w = self
                            .node(parent)
                            .left
                            .expect("rotation preserves sibling slot");
                    }
                    let parent_color = self.node(parent).color;
                    self.set_color(Some(w), parent_color);
                    self.set_color(Some(parent), Color::Black);
                    let new_wl = self.left_of(Some(w));
                    if let Some(wli) = new_wl {
                        self.set_color(Some(wli), Color::Black);
                    }
                    self.right_rotate(parent);
                    x = self.root;
                    x_parent = None;
                }
            }
        }
        if let Some(xi) = x {
            self.node_mut(xi).color = Color::Black;
        }
    }
}

/// In-order traversal iterator. Yields nodes in ascending order of
/// `(range.start, id)`.
pub struct InOrderIter<'a, P> {
    tree: &'a IntervalTree<P>,
    stack: Vec<NodeIdx>,
    cur: Option<NodeIdx>,
}

impl<'a, P> InOrderIter<'a, P> {
    fn new(tree: &'a IntervalTree<P>) -> Self {
        Self {
            tree,
            stack: Vec::new(),
            cur: tree.root,
        }
    }
}

impl<'a, P> Iterator for InOrderIter<'a, P> {
    type Item = super::query::MarkRef<'a, P>;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(c) = self.cur {
            self.stack.push(c);
            self.cur = self.tree.node(c).left;
        }
        let idx = self.stack.pop()?;
        let node = self.tree.node(idx);
        self.cur = node.right;
        Some(super::query::MarkRef {
            id: node.id,
            range: node.range,
            payload: &node.payload,
        })
    }
}
