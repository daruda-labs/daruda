//! Which nodes this pane will reuse instead of running, and when that stops
//! being true.
//!
//! A pin is this machine at this moment: it names an output a previous run
//! already produced, so it belongs to the pane rather than to the flow file —
//! writing one into a committed file would tell every other reader to skip a
//! node they have never run. Nothing here reaches disk.
//!
//! GPUI-free, for the reason [`super::policy`] is: the rule worth getting right
//! is when a pin has to go, and that is a question about two texts.

use std::collections::BTreeSet;

use daruda_flow::NodeId;
use daruda_flow::parse::parse_flow_file;

/// The nodes whose output is pinned, in the file's own vocabulary.
///
/// A set rather than a list: pinning twice is pinning, and the order a person
/// clicked in says nothing about the run.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::workspace) struct PinSet {
    nodes: BTreeSet<NodeId>,
}

impl PinSet {
    pub(in crate::workspace) fn contains(&self, node: &NodeId) -> bool {
        self.nodes.contains(node)
    }

    pub(in crate::workspace) fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Every pinned node, sorted. What a run is handed.
    pub(in crate::workspace) fn to_vec(&self) -> Vec<NodeId> {
        self.nodes.iter().cloned().collect()
    }

    /// Pin `nodes`, or unpin them if every one is already pinned.
    ///
    /// One gesture for both directions, and "all of them" rather than "any of
    /// them" is what makes a marquee over a half-pinned group finish the job
    /// instead of undoing the half that was done.
    pub(in crate::workspace) fn toggle(&mut self, nodes: &[NodeId]) {
        if nodes.iter().all(|node| self.nodes.contains(node)) {
            for node in nodes {
                self.nodes.remove(node);
            }
        } else {
            self.nodes.extend(nodes.iter().cloned());
        }
    }

    pub(in crate::workspace) fn remove(&mut self, node: &NodeId) {
        self.nodes.remove(node);
    }

    pub(in crate::workspace) fn clear(&mut self) {
        self.nodes.clear();
    }

    /// What one press of the pin button would do to `reusable` — the selected
    /// nodes that have an output at all.
    pub(in crate::workspace) fn action_for(&self, reusable: Vec<NodeId>) -> PinAction {
        if reusable.is_empty() {
            PinAction::Unavailable
        } else if reusable.iter().all(|node| self.nodes.contains(node)) {
            PinAction::Unpin(reusable)
        } else {
            PinAction::Pin(reusable)
        }
    }
}

/// What the pin button offers, given what is selected.
///
/// The nodes ride inside the variants because the tooltip names them and the
/// press acts on them, and a list beside a verb is a pair that can disagree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::workspace) enum PinAction {
    /// Nothing selected whose output could be reused.
    Unavailable,
    Pin(Vec<NodeId>),
    Unpin(Vec<NodeId>),
}

/// Why a pin stopped holding.
///
/// Three things can change what a node would produce — its own definition,
/// something upstream of it, and the axes every node inherits — and a person
/// who has just edited one of them is owed which. Dropping the pin without
/// saying so is how an iteration quietly costs a node again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::workspace) enum PinDropped {
    /// This node's own lines changed.
    NodeChanged,
    /// It is not in the file any more.
    NodeGone,
    /// A node it depends on changed, so what this one reads has.
    UpstreamChanged { node: NodeId },
    /// `defaults` or `profiles` — what an unstated axis resolves to.
    InheritedAxesChanged,
    /// One of the two texts could not be compared against.
    Unreadable,
    /// The run that produced the output is not there any more — swept, or
    /// deleted by hand. Not an edit's doing, so it arrives from the run's own
    /// pin resolution rather than from `surviving`.
    SourceGone,
}

/// What became of a pane's pins across an edit.
pub(in crate::workspace) struct Surviving {
    pub kept: PinSet,
    /// In the file's order, so the same edit reads the same way twice.
    pub dropped: Vec<(NodeId, PinDropped)>,
}

/// The pins that still hold after the file went from `was` to `now`.
///
/// A pin says "what this node produced last time is still what it would
/// produce". What can make that false is the node's own definition **and
/// every definition upstream of it**: a node reads its ancestors' outputs, so
/// rewriting `design`'s prompt changes what `build` would produce without
/// touching a character of `build`. Comparing each node against itself alone
/// kept `build` pinned across exactly that edit and reused an output computed
/// from a design the run had just replaced — with nothing on screen saying so.
///
/// Clearing everything on any edit would mean re-pinning on every iteration,
/// which is the whole value of the feature; so the rule is the closure: a pin
/// survives when its node and all of its ancestors read the same, and a
/// sibling's edit still costs nothing.
///
/// Text that does not parse — either side — clears the lot. There is no
/// node-by-node comparison to make then, and the safe answer to "I cannot tell"
/// is to pay for the node. `defaults` and `profiles` count as every node's own
/// definition, because they are what an unstated axis resolves to.
pub(in crate::workspace) fn surviving(pins: &PinSet, was: Option<&str>, now: &str) -> Surviving {
    // Every reload runs this, and most panes have pinned nothing — no reason to
    // parse two files to answer a question about an empty set.
    if pins.is_empty() {
        return Surviving {
            kept: PinSet::default(),
            dropped: Vec::new(),
        };
    }
    let all_dropped = |why: PinDropped| Surviving {
        kept: PinSet::default(),
        dropped: pins
            .nodes
            .iter()
            .map(|id| (id.clone(), why.clone()))
            .collect(),
    };
    let (Some(was), Ok(now)) = (
        was.and_then(|t| parse_flow_file(t).ok()),
        parse_flow_file(now),
    ) else {
        return all_dropped(PinDropped::Unreadable);
    };
    if was.defaults != now.defaults || was.profiles != now.profiles {
        return all_dropped(PinDropped::InheritedAxesChanged);
    }
    let verdict = |id: &NodeId| -> Option<PinDropped> {
        let before = was.nodes.iter().find(|n| &n.id == id);
        let after = now.nodes.iter().find(|n| &n.id == id);
        match (before, after) {
            (_, None) => return Some(PinDropped::NodeGone),
            (Some(a), Some(b)) if a != b => return Some(PinDropped::NodeChanged),
            // Arrived since the last text, so there is no "what it produced
            // last time" for the pin to be about.
            (None, Some(_)) => return Some(PinDropped::NodeChanged),
            _ => {}
        }
        // Sorted, so the reason names the same ancestor every time.
        with_ancestors(&now, id).into_iter().find_map(|up| {
            let before = was.nodes.iter().find(|n| n.id == up);
            let after = now.nodes.iter().find(|n| n.id == up);
            (before != after).then_some(PinDropped::UpstreamChanged { node: up })
        })
    };
    let mut kept = BTreeSet::new();
    let mut dropped = Vec::new();
    for id in &pins.nodes {
        match verdict(id) {
            Some(why) => dropped.push((id.clone(), why)),
            None => {
                kept.insert(id.clone());
            }
        }
    }
    Surviving {
        kept: PinSet { nodes: kept },
        dropped,
    }
}

/// `id` and everything it transitively depends on, read off `deps` in the
/// parsed file.
///
/// The parsed file rather than a built graph: this runs on text that has not
/// been resolved and may not even be runnable, and the question — which
/// definitions feed this node — is answerable without either. A dep naming a
/// node the file lacks is simply not walked; `daruda_flow`'s own validation
/// is what reports that, and a pin is not the place to repeat it.
fn with_ancestors(file: &daruda_flow::parse::FlowFile, id: &NodeId) -> BTreeSet<NodeId> {
    let mut seen = BTreeSet::new();
    // A `deps` cycle is refused at load, but this runs on unvalidated text —
    // so termination comes from the visited set, not from the file.
    let mut pending = vec![id.clone()];
    while let Some(next) = pending.pop() {
        if !seen.insert(next.clone()) {
            continue;
        }
        if let Some(node) = file.nodes.iter().find(|n| n.id == next) {
            pending.extend(node.deps.iter().cloned());
        }
    }
    seen.remove(id);
    seen
}

#[cfg(test)]
mod tests {
    use super::*;

    const TWO: &str = "\
version: 1
defaults:
  agent:
    id: claude
    mode: bypassPermissions
nodes:
  - id: design
    kind: agent
    output: design.md
    prompt: write a line
  - id: build
    kind: agent
    deps: [design]
    output: build.md
    prompt: build it
";

    fn pinned(nodes: &[&str]) -> PinSet {
        let mut set = PinSet::default();
        set.toggle(&nodes.iter().map(|n| NodeId::from(*n)).collect::<Vec<_>>());
        set
    }

    #[test]
    fn toggling_pins_then_unpins_the_same_nodes() {
        let mut set = PinSet::default();
        set.toggle(&["design".into()]);
        assert!(set.contains(&"design".into()));
        set.toggle(&["design".into()]);
        assert!(set.is_empty());
    }

    /// One button, and what it offers follows the selection rather than a mode
    /// the person has to remember they are in.
    #[test]
    fn the_button_offers_the_direction_the_selection_is_not_already_in() {
        let none = PinSet::default();
        assert_eq!(none.action_for(Vec::new()), PinAction::Unavailable);
        assert_eq!(
            none.action_for(vec!["design".into()]),
            PinAction::Pin(vec!["design".into()])
        );
        assert_eq!(
            pinned(&["design"]).action_for(vec!["design".into()]),
            PinAction::Unpin(vec!["design".into()])
        );
        assert_eq!(
            pinned(&["design"]).action_for(vec!["design".into(), "build".into()]),
            PinAction::Pin(vec!["design".into(), "build".into()]),
            "one of the two is not pinned, so the press finishes the job"
        );
    }

    /// A marquee over a group where one is already pinned finishes the job.
    /// Toggling each one separately would undo that one.
    #[test]
    fn a_half_pinned_group_becomes_wholly_pinned() {
        let mut set = pinned(&["design"]);
        set.toggle(&["design".into(), "build".into()]);
        assert_eq!(
            set.to_vec(),
            vec![NodeId::from("build"), NodeId::from("design")]
        );
    }

    /// The rule the feature lives on: editing one node must not cost the pin
    /// on another, or every iteration starts by re-pinning everything.
    #[test]
    fn only_the_node_that_changed_loses_its_pin() {
        let edited = TWO.replace("build it", "build it twice");
        let kept = surviving(&pinned(&["design", "build"]), Some(TWO), &edited).kept;
        assert_eq!(kept.to_vec(), vec![NodeId::from("design")]);
    }

    /// The direction the old rule missed: `build` reads `design`'s output, so
    /// rewriting `design` changes what `build` would produce without touching
    /// a character of `build`. Keeping that pin reuses an output computed from
    /// a design the run is about to replace.
    #[test]
    fn a_pin_does_not_outlive_an_edit_to_what_it_depends_on() {
        let edited = TWO.replace("write a line", "write three lines");
        let kept = surviving(&pinned(&["design", "build"]), Some(TWO), &edited).kept;
        assert!(
            kept.is_empty(),
            "design changed, so neither it nor build still holds: {:?}",
            kept.to_vec()
        );
    }

    /// And the direction that must keep working, or every iteration starts by
    /// re-pinning: a sibling with no path to the pinned node costs it nothing.
    #[test]
    fn a_pin_outlives_an_edit_to_a_node_it_does_not_depend_on() {
        const FORK: &str = "\
version: 1
defaults:
  agent:
    id: claude
    mode: bypassPermissions
nodes:
  - id: design
    kind: agent
    output: design.md
    prompt: write a line
  - id: notes
    kind: agent
    output: notes.md
    prompt: take notes
";
        let edited = FORK.replace("take notes", "take better notes");
        let kept = surviving(&pinned(&["design"]), Some(FORK), &edited).kept;
        assert_eq!(kept.to_vec(), vec![NodeId::from("design")]);
    }

    #[test]
    fn a_node_that_left_the_file_loses_its_pin() {
        let gone = TWO.replace(
            "  - id: build\n    kind: agent\n    deps: [design]\n    output: build.md\n    prompt: build it\n",
            "",
        );
        let kept = surviving(&pinned(&["design", "build"]), Some(TWO), &gone).kept;
        assert_eq!(kept.to_vec(), vec![NodeId::from("design")]);
    }

    /// Reformatting the file around a node is not a change to that node, and
    /// the comparison is of the parsed node rather than of its bytes.
    #[test]
    fn a_node_that_reads_the_same_keeps_its_pin() {
        let moved = TWO.replace("version: 1\n", "version: 1\n# a comment\n");
        let kept = surviving(&pinned(&["design", "build"]), Some(TWO), &moved).kept;
        assert_eq!(
            kept.to_vec(),
            vec![NodeId::from("build"), NodeId::from("design")]
        );
    }

    /// A node states one axis and inherits the rest, so an edit to `defaults`
    /// changes what every node would produce without changing any node's own
    /// lines. Comparing nodes alone would keep pins that are no longer true.
    #[test]
    fn changing_what_every_node_inherits_clears_every_pin() {
        let other_agent = TWO.replace("id: claude", "id: codex");
        assert!(
            surviving(&pinned(&["design"]), Some(TWO), &other_agent)
                .kept
                .is_empty()
        );
    }

    /// Dropping a pin without saying why is how an iteration quietly pays for
    /// a node again. Each of the three things that can change what a node
    /// produces has to be told apart — and the upstream one has to *name* the
    /// ancestor, which is what the closure walk already knows.
    #[test]
    fn a_dropped_pin_says_which_of_the_three_things_changed() {
        let own = TWO.replace("build it", "build it twice");
        assert_eq!(
            surviving(&pinned(&["build"]), Some(TWO), &own).dropped,
            vec![(NodeId::from("build"), PinDropped::NodeChanged)]
        );

        let upstream = TWO.replace("write a line", "write three lines");
        assert_eq!(
            surviving(&pinned(&["build"]), Some(TWO), &upstream).dropped,
            vec![(
                NodeId::from("build"),
                PinDropped::UpstreamChanged {
                    node: NodeId::from("design"),
                }
            )],
            "the ancestor is named, not just blamed"
        );

        let inherited = TWO.replace("id: claude", "id: codex");
        assert_eq!(
            surviving(&pinned(&["build"]), Some(TWO), &inherited).dropped,
            vec![(NodeId::from("build"), PinDropped::InheritedAxesChanged)]
        );

        let gone = TWO.replace(
            "  - id: build\n    kind: agent\n    deps: [design]\n    output: build.md\n    prompt: build it\n",
            "",
        );
        assert_eq!(
            surviving(&pinned(&["build"]), Some(TWO), &gone).dropped,
            vec![(NodeId::from("build"), PinDropped::NodeGone)]
        );
    }

    /// A pin that still holds is not in the list — the cards read it to decide
    /// what to say, and a kept pin saying "unpinned" would be a lie.
    #[test]
    fn a_pin_that_holds_is_not_reported_as_dropped() {
        let elsewhere = TWO.replace("build it", "build it twice");
        let surviving = surviving(&pinned(&["design", "build"]), Some(TWO), &elsewhere);
        assert_eq!(surviving.kept.to_vec(), vec![NodeId::from("design")]);
        assert_eq!(
            surviving
                .dropped
                .iter()
                .map(|(id, _)| id)
                .collect::<Vec<_>>(),
            vec![&NodeId::from("build")]
        );
    }

    #[test]
    fn text_that_does_not_parse_clears_every_pin() {
        assert!(
            surviving(&pinned(&["design"]), Some("nodes: ["), TWO)
                .kept
                .is_empty(),
            "the old text could not be compared against"
        );
        assert!(
            surviving(&pinned(&["design"]), Some(TWO), "nodes: [")
                .kept
                .is_empty(),
            "the new text could not be compared against"
        );
        assert!(
            surviving(&pinned(&["design"]), None, TWO).kept.is_empty(),
            "there was no old text at all"
        );
    }
}
