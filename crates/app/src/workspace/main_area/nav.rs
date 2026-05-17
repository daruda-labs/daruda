//! Geometric pane navigation modeled after iTerm2's `PTYTab`
//! `sessionAdjacentTo:verticalDir:after:` (PTYTab.m:1307-1422).
//!
//! Three-stage filter:
//! 1. **Projection** — keep panes whose perpendicular extent overlaps the
//!    source pane and whose axis position lies in the requested direction.
//! 2. **Adjacency** — drop panes that have another pane sitting between them
//!    and the source along the navigation axis.
//! 3. **Tie-break** — pick the candidate with the largest activity counter
//!    (most recently focused).

use std::collections::HashMap;

use crate::workspace::main_area::pane_tree::{PaneId, PaneRect};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::workspace) enum NavDirection {
    Left,
    Right,
    Up,
    Down,
}

const EPS: f32 = 0.5;

fn vertical_overlap(a: &PaneRect, b: &PaneRect) -> bool {
    a.y < b.y + b.h - EPS && b.y < a.y + a.h - EPS
}

fn horizontal_overlap(a: &PaneRect, b: &PaneRect) -> bool {
    a.x < b.x + b.w - EPS && b.x < a.x + a.w - EPS
}

fn in_direction(a: &PaneRect, b: &PaneRect, dir: NavDirection) -> bool {
    match dir {
        NavDirection::Left => b.x + b.w <= a.x + EPS && vertical_overlap(a, b),
        NavDirection::Right => b.x >= a.x + a.w - EPS && vertical_overlap(a, b),
        NavDirection::Up => b.y + b.h <= a.y + EPS && horizontal_overlap(a, b),
        NavDirection::Down => b.y >= a.y + a.h - EPS && horizontal_overlap(a, b),
    }
}

/// Distance from `a`'s leading edge to `b`'s leading edge along the axis.
fn axis_distance(a: &PaneRect, b: &PaneRect, dir: NavDirection) -> f32 {
    match dir {
        NavDirection::Left => a.x - (b.x + b.w),
        NavDirection::Right => b.x - (a.x + a.w),
        NavDirection::Up => a.y - (b.y + b.h),
        NavDirection::Down => b.y - (a.y + a.h),
    }
}

/// Does `c` sit strictly between `a` and `b` along `dir`?
fn sits_between(a: &PaneRect, b: &PaneRect, c: &PaneRect, dir: NavDirection) -> bool {
    if c.id == a.id || c.id == b.id {
        return false;
    }
    if !in_direction(a, c, dir) {
        return false;
    }
    let d_ab = axis_distance(a, b, dir);
    let d_ac = axis_distance(a, c, dir);
    d_ac < d_ab - EPS
}

/// Find the best neighbor of `from` in the given direction.
pub(in crate::workspace) fn pane_in_direction(
    rects: &[PaneRect],
    from: PaneId,
    dir: NavDirection,
    activity: &HashMap<PaneId, u64>,
) -> Option<PaneId> {
    let a = rects.iter().find(|r| r.id == from)?;

    // Stage 1: projection.
    let projected: Vec<&PaneRect> = rects
        .iter()
        .filter(|b| b.id != a.id && in_direction(a, b, dir))
        .collect();
    if projected.is_empty() {
        return None;
    }

    // Stage 2: adjacency — drop those with someone between them and `a`.
    let adjacent: Vec<&PaneRect> = projected
        .iter()
        .copied()
        .filter(|b| !rects.iter().any(|c| sits_between(a, b, c, dir)))
        .collect();
    if adjacent.is_empty() {
        return None;
    }

    // Stage 3: pick max activity counter; ties broken by smallest axis-distance
    // then by smallest perpendicular offset to the source.
    adjacent
        .into_iter()
        .max_by(|x, y| {
            let ax = activity.get(&x.id).copied().unwrap_or(0);
            let ay = activity.get(&y.id).copied().unwrap_or(0);
            ax.cmp(&ay)
                .then_with(|| {
                    axis_distance(a, y, dir)
                        .partial_cmp(&axis_distance(a, x, dir))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| {
                    perp_offset(a, x, dir)
                        .partial_cmp(&perp_offset(a, y, dir))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        })
        .map(|r| r.id)
}

fn perp_offset(a: &PaneRect, b: &PaneRect, dir: NavDirection) -> f32 {
    match dir {
        NavDirection::Left | NavDirection::Right => (a.y - b.y).abs(),
        NavDirection::Up | NavDirection::Down => (a.x - b.x).abs(),
    }
}

#[cfg(test)]
mod tests {
    use crate::workspace::main_area::pane_tree::{PaneLayout, SplitDirection, collect_pane_rects};
    use super::*;

    fn split(dir: SplitDirection, children: Vec<PaneLayout>) -> PaneLayout {
        PaneLayout::new_split(dir, children)
    }
    fn leaf(id: PaneId) -> PaneLayout {
        PaneLayout::Pane(id)
    }

    fn rects_of(layout: &PaneLayout) -> Vec<PaneRect> {
        let mut out = Vec::new();
        collect_pane_rects(layout, 0.0, 0.0, 800.0, 600.0, &mut out);
        out
    }

    #[test]
    fn left_right_in_h_split() {
        // [1 | 2 | 3]
        let layout = split(SplitDirection::Horizontal, vec![leaf(1), leaf(2), leaf(3)]);
        let rects = rects_of(&layout);
        let act = HashMap::new();
        assert_eq!(
            pane_in_direction(&rects, 2, NavDirection::Left, &act),
            Some(1)
        );
        assert_eq!(
            pane_in_direction(&rects, 2, NavDirection::Right, &act),
            Some(3)
        );
        assert_eq!(pane_in_direction(&rects, 1, NavDirection::Left, &act), None);
        assert_eq!(
            pane_in_direction(&rects, 3, NavDirection::Right, &act),
            None
        );
    }

    #[test]
    fn up_down_in_v_split() {
        let layout = split(SplitDirection::Vertical, vec![leaf(1), leaf(2), leaf(3)]);
        let rects = rects_of(&layout);
        let act = HashMap::new();
        assert_eq!(
            pane_in_direction(&rects, 2, NavDirection::Up, &act),
            Some(1)
        );
        assert_eq!(
            pane_in_direction(&rects, 2, NavDirection::Down, &act),
            Some(3)
        );
    }

    #[test]
    fn adjacency_skips_far_pane() {
        // [1 | 2 | 3] — from 1, Right should pick 2 (not 3, which is farther).
        let layout = split(SplitDirection::Horizontal, vec![leaf(1), leaf(2), leaf(3)]);
        let rects = rects_of(&layout);
        let act = HashMap::new();
        assert_eq!(
            pane_in_direction(&rects, 1, NavDirection::Right, &act),
            Some(2)
        );
    }

    #[test]
    fn nested_layout_navigation() {
        // Horizontal: [1 | V[2/3]]
        // From 1 → Right → either 2 or 3 (both adjacent). Activity breaks tie.
        let layout = split(
            SplitDirection::Horizontal,
            vec![
                leaf(1),
                split(SplitDirection::Vertical, vec![leaf(2), leaf(3)]),
            ],
        );
        let rects = rects_of(&layout);
        let mut act = HashMap::new();
        act.insert(3, 5);
        act.insert(2, 1);
        assert_eq!(
            pane_in_direction(&rects, 1, NavDirection::Right, &act),
            Some(3)
        );
        // From 2 → Down → 3
        assert_eq!(
            pane_in_direction(&rects, 2, NavDirection::Down, &act),
            Some(3)
        );
        // From 3 → Left → 1
        assert_eq!(
            pane_in_direction(&rects, 3, NavDirection::Left, &act),
            Some(1)
        );
    }

    #[test]
    fn no_neighbor_returns_none() {
        let layout = leaf(1);
        let rects = rects_of(&layout);
        let act = HashMap::new();
        assert_eq!(
            pane_in_direction(&rects, 1, NavDirection::Right, &act),
            None
        );
    }
}
