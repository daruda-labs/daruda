//! What a line the person just drew means to the file.
//!
//! GPUI-free, and deliberately blind to the canvas: every name here is a
//! [`NodeId`] — the id the flow file uses — so a canvas node id cannot reach
//! this module at all. Translating one into the other is the caller's job, and
//! that boundary is what keeps the direction below honest.
//!
//! **The direction is the whole risk.** A canvas edge runs output → input;
//! `deps` says "this node runs *after* those". Get them the wrong way round
//! and every line the person draws means the opposite of what it looks like,
//! in a file that then runs backwards. Nothing about the two shapes says which
//! way is right, so it is asserted here rather than reasoned about at the call
//! site.

use daruda_flow::NodeId;

use super::model::GraphEdge;

/// The dependency a canvas edge from `out_of` to `into` declares.
///
/// The canvas draws `out_of`'s **output** port into `into`'s **input** port,
/// which is the picture of "`into` waits for `out_of`" — so the dep is
/// recorded on `into`, naming `out_of`. The parameter names say the ports
/// rather than "from"/"to", because those two words are what get swapped.
pub(super) fn dep_from_edge(out_of: &NodeId, into: &NodeId) -> GraphEdge {
    GraphEdge {
        from: out_of.clone(),
        to: into.clone(),
    }
}

/// The first drawn edge the file does not already declare, or `None` when the
/// picture and the file agree.
///
/// One at a time on purpose: each becomes a separate write, and the write's
/// reload rebuilds the canvas, so a second difference would be found again on
/// the notify that follows. Answering with all of them would invite a caller
/// to batch writes the reload is about to invalidate.
///
/// Each drawn edge arrives paired with whatever the caller needs to act on it
/// — the canvas's own edge id, in practice. What that is stays the caller's
/// business: this module decides *which* edge, not what to do about it.
pub(super) fn unrecorded<T: Clone>(
    drawn: &[(T, GraphEdge)],
    deps: &[GraphEdge],
) -> Option<(T, GraphEdge)> {
    drawn.iter().find(|(_, edge)| !deps.contains(edge)).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edge(from: &str, to: &str) -> GraphEdge {
        GraphEdge {
            from: from.into(),
            to: to.into(),
        }
    }

    /// The one assertion this module exists for. `design`'s output into
    /// `build`'s input means `build` has `deps: [design]` — **not** the other
    /// way round. `form::apply::connect` writes the `GraphEdge` this returns,
    /// and `apply`'s own test carries the same claim through to the file.
    #[test]
    fn a_line_out_of_one_card_and_into_another_is_the_second_ones_dep() {
        let dep = dep_from_edge(&"design".into(), &"build".into());
        assert_eq!(
            dep.to, "build",
            "the dep is recorded on the card drawn into"
        );
        assert_eq!(dep.from, "design", "and it names the card drawn out of");
    }

    #[test]
    fn drawing_it_the_other_way_says_the_other_thing() {
        assert_ne!(
            dep_from_edge(&"design".into(), &"build".into()),
            dep_from_edge(&"build".into(), &"design".into()),
            "a reversed drag is a different dependency, not the same one"
        );
    }

    /// The companion is opaque here — a number stands in for the canvas edge
    /// id the real caller passes, which is the point: this module does not
    /// know what it is carrying.
    fn drawn(edges: &[(u8, GraphEdge)]) -> Vec<(u8, GraphEdge)> {
        edges.to_vec()
    }

    #[test]
    fn an_edge_the_file_does_not_have_is_the_one_to_write() {
        let deps = vec![edge("design", "build")];
        let canvas = drawn(&[(1, edge("design", "build")), (2, edge("build", "ship"))]);
        assert_eq!(
            unrecorded(&canvas, &deps),
            Some((2, edge("build", "ship"))),
            "and it comes back with the id that drew it"
        );
    }

    #[test]
    fn a_picture_that_agrees_with_the_file_is_nothing_to_do() {
        let deps = vec![edge("design", "build")];
        assert_eq!(
            unrecorded(&drawn(&[(1, edge("design", "build"))]), &deps),
            None
        );
        assert_eq!(
            unrecorded::<u8>(&[], &deps),
            None,
            "and neither is an empty one"
        );
    }

    /// The reversal trap from the other side: a canvas holding the opposite
    /// edge is a *difference*, not a match, so it would be written — which is
    /// exactly why the direction is pinned above.
    #[test]
    fn a_reversed_edge_is_not_the_one_the_file_has() {
        let deps = vec![edge("design", "build")];
        assert_eq!(
            unrecorded(&drawn(&[(9, edge("build", "design"))]), &deps),
            Some((9, edge("build", "design")))
        );
    }
}
