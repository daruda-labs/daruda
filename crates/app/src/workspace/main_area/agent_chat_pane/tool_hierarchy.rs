//! Parent/child structure of a conversation's tool calls, built in one pass.
//!
//! Every rule about that structure lives here — a `parent_tool_id` naming a
//! call absent from `items` leaves the child top-level, an ancestor walk is
//! bounded so a malformed or cyclic link terminates, and a walk downward stops
//! at the depth the card stops rendering at. Callers ask questions; none of
//! them restate the rules.

use std::collections::HashMap;

use daruda_acp::{ChatItem, ToolCallItem};

/// Bounds malformed or cyclic subagent parent links.
pub(in crate::workspace) const SUBAGENT_NEST_DEPTH_CAP: usize = 8;

pub(in crate::workspace) struct ToolHierarchy<'a> {
    /// Tool-call id → its index in `items`. First occurrence wins.
    index_of: HashMap<&'a str, usize>,
    /// Tool-call id → its declared parent, present in `items` or not.
    parent_of: HashMap<&'a str, &'a str>,
    /// Declared parent id → the calls naming it.
    children_of: HashMap<&'a str, Vec<&'a str>>,
}

impl<'a> ToolHierarchy<'a> {
    pub(in crate::workspace) fn build(items: &'a [ChatItem]) -> Self {
        let mut index_of: HashMap<&'a str, usize> = HashMap::new();
        let mut parent_of: HashMap<&'a str, &'a str> = HashMap::new();
        let mut children_of: HashMap<&'a str, Vec<&'a str>> = HashMap::new();
        for (ix, item) in items.iter().enumerate() {
            let ChatItem::ToolCall(tc) = item else {
                continue;
            };
            let id = tc.id.as_str();
            index_of.entry(id).or_insert(ix);
            if let Some(parent) = tc.parent_tool_id.as_deref() {
                parent_of.insert(id, parent);
                children_of.entry(parent).or_default().push(id);
            }
        }
        Self {
            index_of,
            parent_of,
            children_of,
        }
    }

    /// Whether `items` holds a tool call with this id.
    pub(in crate::workspace) fn contains(&self, id: &str) -> bool {
        self.index_of.contains_key(id)
    }

    /// Whether this call renders inside a parent's card instead of earning a
    /// row of its own. A dangling parent id keeps it top-level, so a child of
    /// a call `items` never carried cannot vanish.
    pub(in crate::workspace) fn is_nested_child(&self, tc: &ToolCallItem) -> bool {
        tc.parent_tool_id
            .as_deref()
            .is_some_and(|pid| self.contains(pid))
    }

    /// `start`, then each declared ancestor above it, nearest first. Capped at
    /// [`SUBAGENT_NEST_DEPTH_CAP`] hops so a cycle terminates. Yields ancestors
    /// absent from `items` too: presence decides row ownership
    /// ([`Self::is_nested_child`]), not membership in a chain.
    pub(in crate::workspace) fn with_ancestors(
        &self,
        start: &'a str,
    ) -> impl Iterator<Item = &'a str> + '_ {
        std::iter::successors(Some(start), move |cur| self.parent_of.get(*cur).copied())
            .take(SUBAGENT_NEST_DEPTH_CAP + 1)
    }

    /// [`Self::with_ancestors`] without `start` itself.
    pub(in crate::workspace) fn ancestors(
        &self,
        start: &'a str,
    ) -> impl Iterator<Item = &'a str> + '_ {
        self.with_ancestors(start).skip(1)
    }

    /// The `items` index of the call that owns a row for `id`: itself when its
    /// parent is absent from `items`, else the nearest present ancestor.
    /// `None` when `id` is unknown or the walk exceeds the depth cap.
    pub(in crate::workspace) fn owning_row_index(&self, id: &str) -> Option<usize> {
        let mut current = id;
        for _ in 0..SUBAGENT_NEST_DEPTH_CAP {
            let ix = *self.index_of.get(current)?;
            match self.parent_of.get(current) {
                Some(parent) if self.contains(parent) => current = parent,
                _ => return Some(ix),
            }
        }
        None
    }

    /// Blocks a reveal puts back inside `root`'s card: each descendant `keeps`
    /// rejects whose own ancestors all survived it. A rejected node stands for
    /// its whole subtree, so the walk stops there rather than counting inside.
    ///
    /// Bounded by [`SUBAGENT_NEST_DEPTH_CAP`] the way the card's own recursion
    /// is, so the count names exactly the children that would have rendered.
    pub(in crate::workspace) fn cut_below(
        &self,
        root: &str,
        keeps: impl Fn(&str) -> bool,
    ) -> usize {
        let mut cut = 0;
        let mut frontier = vec![(root, 0usize)];
        while let Some((id, depth)) = frontier.pop() {
            if depth >= SUBAGENT_NEST_DEPTH_CAP {
                continue;
            }
            for child in self.children_of.get(id).into_iter().flatten() {
                if keeps(child) {
                    frontier.push((child, depth + 1));
                } else {
                    cut += 1;
                }
            }
        }
        cut
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use daruda_acp::{ToolKindView, ToolStatusView};

    fn tool(id: &str, parent: Option<&str>) -> ChatItem {
        ChatItem::ToolCall(ToolCallItem {
            id: id.to_owned(),
            title: format!("Tool {id}"),
            kind: ToolKindView::Edit,
            tool_name: None,
            status: ToolStatusView::Completed,
            diffs: Vec::new(),
            output: Vec::new(),
            raw_input: None,
            locations: Vec::new(),
            parent_tool_id: parent.map(str::to_owned),
            exit: None,
        })
    }

    fn call(items: &[ChatItem], ix: usize) -> &ToolCallItem {
        match &items[ix] {
            ChatItem::ToolCall(tc) => tc,
            _ => panic!("not a tool call"),
        }
    }

    #[test]
    fn a_present_parent_nests_the_child_and_a_dangling_one_does_not() {
        let items = [
            tool("a", None),
            tool("b", Some("a")),
            tool("c", Some("ghost")),
        ];
        let h = ToolHierarchy::build(&items);
        assert!(!h.is_nested_child(call(&items, 0)));
        assert!(h.is_nested_child(call(&items, 1)));
        assert!(
            !h.is_nested_child(call(&items, 2)),
            "a parent that is not in items leaves the child top-level"
        );
    }

    #[test]
    fn owning_row_index_climbs_to_the_call_that_earns_a_row() {
        let items = [
            tool("a", None),
            tool("b", Some("a")),
            tool("c", Some("b")),
            tool("d", Some("ghost")),
        ];
        let h = ToolHierarchy::build(&items);
        assert_eq!(h.owning_row_index("c"), Some(0));
        assert_eq!(h.owning_row_index("a"), Some(0));
        assert_eq!(h.owning_row_index("d"), Some(3), "dangling parent → itself");
        assert_eq!(h.owning_row_index("ghost"), None, "not in items");
    }

    #[test]
    fn owning_row_index_gives_up_on_a_cycle_instead_of_looping() {
        let items = [tool("x", Some("y")), tool("y", Some("x"))];
        let h = ToolHierarchy::build(&items);
        assert_eq!(h.owning_row_index("x"), None);
    }

    /// Everything `keeps` rejects is one block, and the walk does not look
    /// under it: the card stops rendering there, so its subtree is already
    /// gone with it and counting inside would report the same cut twice.
    #[test]
    fn a_rejected_node_counts_once_and_its_subtree_is_not_walked() {
        let items = [
            tool("root", None),
            tool("cut", Some("root")),
            tool("under-cut", Some("cut")),
            tool("deeper", Some("under-cut")),
        ];
        let h = ToolHierarchy::build(&items);
        assert_eq!(h.cut_below("root", |id| id != "cut"), 1);
    }

    /// The other half: a kept node is walked *through*, so a cut that only
    /// appears a level down is still reported.
    #[test]
    fn a_kept_node_is_walked_through_to_the_cut_below_it() {
        let items = [
            tool("root", None),
            tool("kept", Some("root")),
            tool("cut-a", Some("kept")),
            tool("cut-b", Some("kept")),
        ];
        let h = ToolHierarchy::build(&items);
        assert_eq!(h.cut_below("root", |id| !id.starts_with("cut")), 2);
        assert_eq!(
            h.cut_below("root", |_| true),
            0,
            "nothing rejected, nothing counted"
        );
    }

    /// The count must name exactly the children a card would have rendered, and
    /// the card stops recursing at the cap — so a cut past it is not a cut the
    /// reveal could put back.
    #[test]
    fn the_downward_walk_stops_at_the_same_cap_the_card_does() {
        let mut items = vec![tool("n0", None)];
        for d in 1..=(SUBAGENT_NEST_DEPTH_CAP + 2) {
            items.push(tool(&format!("n{d}"), Some(&format!("n{}", d - 1))));
        }
        let h = ToolHierarchy::build(&items);
        let deepest_rendered = format!("n{SUBAGENT_NEST_DEPTH_CAP}");
        assert_eq!(h.cut_below("n0", |id| id != deepest_rendered), 1);
        let past_the_cap = format!("n{}", SUBAGENT_NEST_DEPTH_CAP + 1);
        assert_eq!(
            h.cut_below("n0", |id| id != past_the_cap),
            0,
            "one level further out is not on screen to begin with"
        );
    }

    #[test]
    fn the_downward_walk_terminates_on_a_cyclic_link() {
        let items = [tool("x", Some("y")), tool("y", Some("x"))];
        let h = ToolHierarchy::build(&items);
        assert_eq!(h.cut_below("x", |_| true), 0);
        assert!(
            h.cut_below("x", |id| id != "y") > 0,
            "and still reports the cut"
        );
    }

    #[test]
    fn the_ancestor_walk_stops_at_the_depth_cap() {
        let mut items = vec![tool("n0", None)];
        for d in 1..=(SUBAGENT_NEST_DEPTH_CAP + 2) {
            items.push(tool(&format!("n{d}"), Some(&format!("n{}", d - 1))));
        }
        let h = ToolHierarchy::build(&items);
        let leaf = format!("n{}", SUBAGENT_NEST_DEPTH_CAP + 2);
        let with_self: Vec<&str> = h.with_ancestors(&leaf).collect();
        assert_eq!(with_self.len(), SUBAGENT_NEST_DEPTH_CAP + 1);
        assert_eq!(with_self[0], leaf);
        let strict: Vec<&str> = h.ancestors(&leaf).collect();
        assert_eq!(strict.len(), SUBAGENT_NEST_DEPTH_CAP);
        assert_eq!(strict[0], format!("n{}", SUBAGENT_NEST_DEPTH_CAP + 1));
    }

    #[test]
    fn the_ancestor_walk_terminates_on_a_self_parent() {
        let items = [tool("y", Some("y"))];
        let h = ToolHierarchy::build(&items);
        assert_eq!(
            h.ancestors("y").collect::<Vec<_>>().len(),
            SUBAGENT_NEST_DEPTH_CAP
        );
    }

    #[test]
    fn a_dangling_ancestor_is_still_part_of_the_chain() {
        let items = [tool("a", Some("ghost"))];
        let h = ToolHierarchy::build(&items);
        assert_eq!(h.ancestors("a").collect::<Vec<_>>(), vec!["ghost"]);
    }
}
