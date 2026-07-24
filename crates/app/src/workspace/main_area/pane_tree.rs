//! Pure layout data structure for pane splitting — no GPUI dependency.
//!
//! N-ary tree modeled after iTerm2's `PTYTab` / `NSSplitView`:
//! - Each `Split` holds an ordered list of children and normalized ratios.
//! - Inserting a pane next to an existing one follows iTerm2's 3-case rule
//!   (same-direction / opposite-direction / single-child parent).
//! - Removing a pane re-normalizes the tree (`cleanup_after_remove`) so that
//!   Splits with a single child collapse and Splits whose direction matches
//!   their parent are flattened into the parent.

pub(in crate::workspace) type PaneId = u64;

pub(in crate::workspace) const DIVIDER_PX: f32 = 1.0;

/// Minimum fraction of an axis a single pane may occupy. Prevents dividers
/// from being dragged into an unusable state. 5% matches iTerm2's lower bound.
pub(in crate::workspace) const MIN_RATIO: f32 = 0.05;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::workspace) enum SplitDirection {
    // Note: Horizontal = children side-by-side along X axis (the divider bar is vertical).
    // Not renamed to Left/Right/Up/Down because Zed's 4-direction enum has no 1:1 mapping.
    Horizontal, // side-by-side (children laid out along X)
    Vertical,   // stacked (children laid out along Y)
}

/// Quadrant of a pane that a header-drag targets. The drop-split UI that
/// consumes this is wired separately; the pure geometry/transform lives here.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::workspace) enum DropHalf {
    West,
    East,
    North,
    South,
}

impl DropHalf {
    pub(in crate::workspace) fn direction(self) -> SplitDirection {
        match self {
            // West/East = panes side-by-side (children along X) = Horizontal
            DropHalf::West | DropHalf::East => SplitDirection::Horizontal,
            // North/South = panes stacked (children along Y) = Vertical
            DropHalf::North | DropHalf::South => SplitDirection::Vertical,
        }
    }

    /// New pane is inserted BEFORE the target when dropped on its left/top half.
    pub(in crate::workspace) fn before(self) -> bool {
        matches!(self, DropHalf::West | DropHalf::North)
    }
}

/// Score threshold = active strip width from each edge (40% of the axis).
/// Leaves a central (1 - 2×0.4) = 20%×20% rectangle as a dead-zone (None).
/// Matches iTerm2's SplitSelectionView.
pub(in crate::workspace) const DROP_DEAD_ZONE: f32 = 0.4;

/// `(x, y)` is the cursor position local to a pane of size `(w, h)`.
/// Precondition: 0.0 <= x <= w and 0.0 <= y <= h (caller bounds-checks).
pub(in crate::workspace) fn compute_drop_half(x: f32, y: f32, w: f32, h: f32) -> Option<DropHalf> {
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let (hx, hh) = if x < w / 2.0 {
        (x / w, DropHalf::West)
    } else {
        ((w - x) / w, DropHalf::East)
    };
    let (vy, vh) = if y < h / 2.0 {
        (y / h, DropHalf::North)
    } else {
        ((h - y) / h, DropHalf::South)
    };
    // On an exact diagonal tie (hx == vy) the vertical half (North/South) wins, matching iTerm2.
    let (score, half) = if hx < vy { (hx, hh) } else { (vy, vh) };
    (score < DROP_DEAD_ZONE).then_some(half)
}

pub(in crate::workspace) enum PaneLayout {
    Pane(PaneId),
    Split {
        direction: SplitDirection,
        children: Vec<PaneLayout>,
        ratios: Vec<f32>,
    },
}

impl PaneLayout {
    pub(in crate::workspace) fn new_split(
        direction: SplitDirection,
        children: Vec<PaneLayout>,
    ) -> PaneLayout {
        let n = children.len();
        debug_assert!(n >= 2, "Split must have at least two children");
        let ratios = vec![1.0 / n as f32; n];
        PaneLayout::Split {
            direction,
            children,
            ratios,
        }
    }

    pub(in crate::workspace) fn contains(&self, target: PaneId) -> bool {
        match self {
            PaneLayout::Pane(id) => *id == target,
            PaneLayout::Split { children, .. } => children.iter().any(|c| c.contains(target)),
        }
    }

    pub(in crate::workspace) fn first_leaf(&self) -> PaneId {
        match self {
            PaneLayout::Pane(id) => *id,
            PaneLayout::Split { children, .. } => children[0].first_leaf(),
        }
    }

    fn collect_pane_ids(&self, out: &mut Vec<PaneId>) {
        match self {
            PaneLayout::Pane(id) => out.push(*id),
            PaneLayout::Split { children, .. } => {
                for c in children {
                    c.collect_pane_ids(out);
                }
            }
        }
    }

    pub(in crate::workspace) fn pane_ids(&self) -> Vec<PaneId> {
        let mut ids = Vec::new();
        self.collect_pane_ids(&mut ids);
        ids
    }

    pub(in crate::workspace) fn leaf_count(&self) -> usize {
        match self {
            PaneLayout::Pane(_) => 1,
            PaneLayout::Split { children, .. } => children.iter().map(|c| c.leaf_count()).sum(),
        }
    }

    pub(in crate::workspace) fn next_pane(&self, current: PaneId) -> Option<PaneId> {
        let ids = self.pane_ids();
        let pos = ids.iter().position(|&id| id == current)?;
        Some(ids[(pos + 1) % ids.len()])
    }

    pub(in crate::workspace) fn prev_pane(&self, current: PaneId) -> Option<PaneId> {
        let ids = self.pane_ids();
        let pos = ids.iter().position(|&id| id == current)?;
        Some(ids[(pos + ids.len() - 1) % ids.len()])
    }
}

/// Insert `new_node` (a leaf or an arbitrary subtree) next to `target`,
/// following iTerm2's split rules.
///
/// Returns `Ok(())` when the target was found and `new_node` was consumed
/// into the tree. Returns `Err(new_node)` — handing the subtree back
/// unconsumed — when no match was found at this level or below, so the
/// caller can retry with the next sibling or give up.
///
/// Mirrors `PTYTab.m -splitVertically:newSession:before:targetSession:`:
///
/// - **Case C (root leaf):** replace root with a 2-child Split. `before`
///   orders the new subtree ahead of the original.
/// - **Case A (same direction parent):** splice `new_node` next to `target`
///   in the parent's children; redistribute ratios so the new subtree takes
///   half of `target`'s share. `before` inserts on `target`'s left/top side.
/// - **Case B (opposite direction parent):** replace the target leaf with a
///   new Split whose children are `[target, new_node]` (or `[new_node,
///   target]` when `before`) in the requested direction.
fn try_insert_node_at(
    layout: &mut PaneLayout,
    target: PaneId,
    direction: SplitDirection,
    new_node: PaneLayout,
    before: bool,
) -> Result<(), PaneLayout> {
    // Case C: root is the target leaf.
    if let PaneLayout::Pane(id) = layout
        && *id == target
    {
        let original = PaneLayout::Pane(*id);
        let children = if before {
            vec![new_node, original]
        } else {
            vec![original, new_node]
        };
        *layout = PaneLayout::new_split(direction, children);
        return Ok(());
    }

    let PaneLayout::Split {
        direction: parent_dir,
        children,
        ratios,
    } = layout
    else {
        return Err(new_node);
    };

    // Direct child matches target?
    for i in 0..children.len() {
        if let PaneLayout::Pane(id) = children[i]
            && id == target
        {
            if *parent_dir == direction {
                // Case A: splice next to target, halve its ratio.
                let share = ratios[i] / 2.0;
                ratios[i] = share;
                let slot = if before { i } else { i + 1 };
                ratios.insert(slot, share);
                children.insert(slot, new_node);
            } else {
                // Case B: wrap target in a new Split.
                let target_leaf = std::mem::replace(&mut children[i], PaneLayout::Pane(0));
                let inner = if before {
                    vec![new_node, target_leaf]
                } else {
                    vec![target_leaf, new_node]
                };
                children[i] = PaneLayout::new_split(direction, inner);
            }
            return Ok(());
        }
    }

    // Target lives deeper — recurse, handing new_node back on a miss so the
    // next child (or the caller) can retry inserting it.
    let mut new_node = new_node;
    for child in children.iter_mut() {
        match try_insert_node_at(child, target, direction, new_node, before) {
            Ok(()) => return Ok(()),
            Err(returned) => new_node = returned,
        }
    }
    Err(new_node)
}

/// Insert `new_node` (a leaf or an arbitrary subtree) next to `target`. See
/// `try_insert_node_at` for the case-by-case rules. Returns true when the
/// target was found and the subtree inserted.
pub(in crate::workspace) fn insert_node_at(
    layout: &mut PaneLayout,
    target: PaneId,
    direction: SplitDirection,
    new_node: PaneLayout,
    before: bool,
) -> bool {
    try_insert_node_at(layout, target, direction, new_node, before).is_ok()
}

/// Insert a single new leaf `new_id` next to `target`. Thin wrapper around
/// `insert_node_at` for the common single-pane case.
pub(in crate::workspace) fn insert_split_at(
    layout: &mut PaneLayout,
    target: PaneId,
    direction: SplitDirection,
    new_id: PaneId,
    before: bool,
) -> bool {
    insert_node_at(layout, target, direction, PaneLayout::Pane(new_id), before)
}

/// Move an existing leaf `dragged` to become a split sibling of `target`
/// in the half indicated by `half`. Pure tree transform: remove then
/// re-insert. Returns false on no-op (dragged == target) or if either id
/// is absent. Does NOT create/destroy panes — only edits the layout.
pub(in crate::workspace) fn rearrange_pane(
    layout: &mut PaneLayout,
    dragged: PaneId,
    target: PaneId,
    half: DropHalf,
) -> bool {
    if dragged == target {
        return false;
    }
    // Verify target exists before removing dragged, so a missing target is a
    // true no-op. After removal, target (a different id) still exists, so the
    // insert below cannot fail.
    if !layout.contains(target) {
        return false;
    }
    if !remove_pane_from_layout(layout, dragged) {
        return false;
    }
    insert_split_at(layout, target, half.direction(), dragged, half.before())
}

/// Remove `target` and renormalize the tree so there are no stray
/// single-child Splits and no Splits sharing direction with their parent.
///
/// Returns true if the target was found and removed.
pub(in crate::workspace) fn remove_pane_from_layout(
    layout: &mut PaneLayout,
    target: PaneId,
) -> bool {
    let removed = remove_pane_inner(layout, target);
    if removed {
        cleanup_after_remove(layout);
    }
    removed
}

fn remove_pane_inner(layout: &mut PaneLayout, target: PaneId) -> bool {
    let PaneLayout::Split {
        children, ratios, ..
    } = layout
    else {
        return false;
    };

    // Direct leaf removal.
    for i in 0..children.len() {
        if let PaneLayout::Pane(id) = children[i]
            && id == target
        {
            children.remove(i);
            ratios.remove(i);
            redistribute_ratios(ratios);
            return true;
        }
    }

    // Recurse into child Splits.
    for child in children.iter_mut() {
        if remove_pane_inner(child, target) {
            return true;
        }
    }
    false
}

/// Renormalize ratios to sum to 1.0.
fn redistribute_ratios(ratios: &mut [f32]) {
    if ratios.is_empty() {
        return;
    }
    let sum: f32 = ratios.iter().sum();
    if sum <= f32::EPSILON {
        let share = 1.0 / ratios.len() as f32;
        for r in ratios.iter_mut() {
            *r = share;
        }
    } else {
        for r in ratios.iter_mut() {
            *r /= sum;
        }
    }
}

/// Walk the tree post-removal and collapse/flatten degenerate nodes.
///
/// Mirrors `PTYTab.m -cleanupAfterRemove:`:
/// 1. A Split with exactly one child is replaced by that child.
/// 2. A child Split sharing direction with its parent has its children
///    spliced into the parent at its slot (direction-flattening).
pub(in crate::workspace) fn cleanup_after_remove(layout: &mut PaneLayout) {
    if let PaneLayout::Split { children, .. } = layout {
        for child in children.iter_mut() {
            cleanup_after_remove(child);
        }
    }

    // Collapse single-child Split into its child.
    loop {
        let collapse = matches!(layout, PaneLayout::Split { children, .. } if children.len() == 1);
        if !collapse {
            break;
        }
        if let PaneLayout::Split { children, .. } = layout {
            // Invariant: loop condition checked children.len() == 1 above.
            let only = children.pop().expect("collapse loop: children.len() == 1");
            *layout = only;
        }
    }

    // Flatten same-direction nested Splits into this level.
    if let PaneLayout::Split {
        direction,
        children,
        ratios,
    } = layout
    {
        let mut i = 0;
        while i < children.len() {
            let should_flatten = matches!(
                &children[i],
                PaneLayout::Split { direction: cd, .. } if cd == direction
            );
            if !should_flatten {
                i += 1;
                continue;
            }
            let parent_share = ratios[i];
            let (mut inner_children, inner_ratios) =
                match std::mem::replace(&mut children[i], PaneLayout::Pane(0)) {
                    PaneLayout::Split {
                        children: ic,
                        ratios: ir,
                        ..
                    } => (ic, ir),
                    _ => unreachable!(),
                };
            children.remove(i);
            ratios.remove(i);
            let n = inner_children.len();
            for j in (0..n).rev() {
                // Invariant: pop called n times on a vec of length n.
                let c = inner_children
                    .pop()
                    .expect("flatten loop: n == inner_children.len()");
                children.insert(i, c);
                ratios.insert(i, inner_ratios[j] * parent_share);
            }
            i += n;
        }
        redistribute_ratios(ratios);
    }
}

/// Pixel dimensions for each pane — wraps `collect_pane_rects`.
pub(in crate::workspace) fn collect_pane_sizes(
    layout: &PaneLayout,
    width: f32,
    height: f32,
    out: &mut Vec<(PaneId, f32, f32)>,
) {
    let mut rects = Vec::new();
    collect_pane_rects(layout, 0.0, 0.0, width, height, &mut rects);
    out.extend(rects.into_iter().map(|r| (r.id, r.w, r.h)));
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::workspace) struct PaneRect {
    pub(in crate::workspace) id: PaneId,
    pub(in crate::workspace) x: f32,
    pub(in crate::workspace) y: f32,
    pub(in crate::workspace) w: f32,
    pub(in crate::workspace) h: f32,
}

/// Recursively compute absolute rects for every leaf.
pub(in crate::workspace) fn collect_pane_rects(
    layout: &PaneLayout,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    out: &mut Vec<PaneRect>,
) {
    match layout {
        PaneLayout::Pane(id) => out.push(PaneRect {
            id: *id,
            x,
            y,
            w,
            h,
        }),
        PaneLayout::Split {
            direction,
            children,
            ratios,
        } => {
            let n = children.len();
            let divider_total = DIVIDER_PX * (n - 1) as f32;
            match direction {
                SplitDirection::Horizontal => {
                    let avail = (w - divider_total).max(1.0);
                    let mut cur_x = x;
                    for (i, c) in children.iter().enumerate() {
                        let cw = (avail * ratios[i]).max(1.0);
                        collect_pane_rects(c, cur_x, y, cw, h, out);
                        cur_x += cw;
                        if i + 1 < n {
                            cur_x += DIVIDER_PX;
                        }
                    }
                }
                SplitDirection::Vertical => {
                    let avail = (h - divider_total).max(1.0);
                    let mut cur_y = y;
                    for (i, c) in children.iter().enumerate() {
                        let ch = (avail * ratios[i]).max(1.0);
                        collect_pane_rects(c, x, cur_y, w, ch, out);
                        cur_y += ch;
                        if i + 1 < n {
                            cur_y += DIVIDER_PX;
                        }
                    }
                }
            }
        }
    }
}

/// Walk the layout assigning rects to each Split, and return the axis
/// extent (pixels) of the parent Split that contains the divider whose
/// left/top child has `left_first_leaf` as its first leaf.
pub(in crate::workspace) fn parent_axis_extent(
    layout: &PaneLayout,
    width: f32,
    height: f32,
    left_first_leaf: PaneId,
) -> Option<f32> {
    fn walk(layout: &PaneLayout, w: f32, h: f32, target: PaneId) -> Option<f32> {
        if let PaneLayout::Split {
            direction,
            children,
            ratios,
        } = layout
        {
            // Check if any child slot of *this* split is the target.
            let last = children.len().saturating_sub(1);
            for child in children.iter().take(last) {
                if child.first_leaf() == target {
                    return Some(match direction {
                        SplitDirection::Horizontal => w,
                        SplitDirection::Vertical => h,
                    });
                }
            }
            // Recurse into each child with its computed sub-rect.
            let n = children.len();
            let divider_total = DIVIDER_PX * (n - 1) as f32;
            match direction {
                SplitDirection::Horizontal => {
                    let avail = (w - divider_total).max(1.0);
                    for (i, c) in children.iter().enumerate() {
                        let cw = (avail * ratios[i]).max(1.0);
                        if let Some(r) = walk(c, cw, h, target) {
                            return Some(r);
                        }
                    }
                }
                SplitDirection::Vertical => {
                    let avail = (h - divider_total).max(1.0);
                    for (i, c) in children.iter().enumerate() {
                        let ch = (avail * ratios[i]).max(1.0);
                        if let Some(r) = walk(c, w, ch, target) {
                            return Some(r);
                        }
                    }
                }
            }
        }
        None
    }
    walk(layout, width, height, left_first_leaf)
}

/// Locate a divider by its left/top neighbor's first leaf id.
/// Returns the parent Split's direction and the ratio of the left/top child.
pub(in crate::workspace) fn find_divider(
    layout: &PaneLayout,
    left_first_leaf: PaneId,
) -> Option<(SplitDirection, f32)> {
    if let PaneLayout::Split {
        direction,
        children,
        ratios,
    } = layout
    {
        for i in 0..children.len().saturating_sub(1) {
            if children[i].first_leaf() == left_first_leaf {
                return Some((*direction, ratios[i]));
            }
        }
        for child in children {
            if let Some(r) = find_divider(child, left_first_leaf) {
                return Some(r);
            }
        }
    }
    None
}

/// Adjust the divider between two adjacent children of the Split that
/// contains `divider_key`, applying `delta` in the Split's axis.
///
/// `divider_key` identifies the divider by the pane_id of its left/top
/// neighbor's first leaf (stable even when the neighbor is itself a Split).
pub(in crate::workspace) fn adjust_divider(
    layout: &mut PaneLayout,
    left_first_leaf: PaneId,
    delta_ratio: f32,
) -> bool {
    if let PaneLayout::Split {
        children, ratios, ..
    } = layout
    {
        for i in 0..children.len().saturating_sub(1) {
            if children[i].first_leaf() == left_first_leaf {
                let combined = ratios[i] + ratios[i + 1];
                let mut left = (ratios[i] + delta_ratio).clamp(
                    MIN_RATIO.min(combined / 2.0),
                    combined - MIN_RATIO.min(combined / 2.0),
                );
                // Guard against NaN.
                if !left.is_finite() {
                    left = combined / 2.0;
                }
                ratios[i] = left;
                ratios[i + 1] = combined - left;
                return true;
            }
        }
        for child in children.iter_mut() {
            if adjust_divider(child, left_first_leaf, delta_ratio) {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(id: PaneId) -> PaneLayout {
        PaneLayout::Pane(id)
    }

    fn assert_ratios_sum_to_one(layout: &PaneLayout) {
        if let PaneLayout::Split {
            ratios, children, ..
        } = layout
        {
            let sum: f32 = ratios.iter().sum();
            assert!(
                (sum - 1.0).abs() < 1e-4,
                "ratios sum to {sum}, expected 1.0"
            );
            for child in children {
                assert_ratios_sum_to_one(child);
            }
        }
    }

    fn child_ids(layout: &PaneLayout) -> Vec<PaneId> {
        match layout {
            PaneLayout::Split { children, .. } => children.iter().map(|c| c.first_leaf()).collect(),
            PaneLayout::Pane(id) => vec![*id],
        }
    }

    #[test]
    fn compute_drop_half_returns_each_quadrant_edge() {
        let (w, h) = (100.0, 100.0);
        // Near the left edge, vertically centered → West.
        assert_eq!(compute_drop_half(5.0, 50.0, w, h), Some(DropHalf::West));
        // Near the right edge → East.
        assert_eq!(compute_drop_half(95.0, 50.0, w, h), Some(DropHalf::East));
        // Near the top edge (small y) → North.
        assert_eq!(compute_drop_half(50.0, 5.0, w, h), Some(DropHalf::North));
        // Near the bottom edge → South.
        assert_eq!(compute_drop_half(50.0, 95.0, w, h), Some(DropHalf::South));
    }

    #[test]
    fn compute_drop_half_center_is_dead_zone() {
        let (w, h) = (100.0, 100.0);
        // Exact center: both scores are 0.5 → outside the active region.
        assert_eq!(compute_drop_half(50.0, 50.0, w, h), None);
    }

    #[test]
    fn compute_drop_half_dead_zone_boundary() {
        let (w, h) = (100.0, 100.0);
        // x = 45 → score 0.45 (≥ 0.4) along X, y centered → no split.
        assert_eq!(compute_drop_half(45.0, 50.0, w, h), None);
        // x = 35 → score 0.35 (< 0.4) → West.
        assert_eq!(compute_drop_half(35.0, 50.0, w, h), Some(DropHalf::West));
    }

    #[test]
    fn compute_drop_half_rejects_zero_size() {
        assert_eq!(compute_drop_half(0.0, 0.0, 0.0, 100.0), None);
        assert_eq!(compute_drop_half(0.0, 0.0, 100.0, 0.0), None);
    }

    #[test]
    fn insert_split_at_before_case_c() {
        // Root leaf A, insert B before → [B, A].
        let mut layout = leaf(1);
        assert!(insert_split_at(
            &mut layout,
            1,
            SplitDirection::Horizontal,
            2,
            true
        ));
        assert_eq!(child_ids(&layout), vec![2, 1]);
        assert_ratios_sum_to_one(&layout);
    }

    #[test]
    fn insert_split_at_after_case_c() {
        // Regression: before=false still inserts after → [A, B].
        let mut layout = leaf(1);
        assert!(insert_split_at(
            &mut layout,
            1,
            SplitDirection::Horizontal,
            2,
            false
        ));
        assert_eq!(child_ids(&layout), vec![1, 2]);
        assert_ratios_sum_to_one(&layout);
    }

    #[test]
    fn insert_split_at_before_case_a() {
        // [A | B] horizontal, split B horizontally before → [A | C | B].
        let mut layout = PaneLayout::new_split(SplitDirection::Horizontal, vec![leaf(1), leaf(2)]);
        assert!(insert_split_at(
            &mut layout,
            2,
            SplitDirection::Horizontal,
            3,
            true
        ));
        assert_eq!(child_ids(&layout), vec![1, 3, 2]);
        assert_ratios_sum_to_one(&layout);
    }

    #[test]
    fn insert_split_at_after_case_a() {
        // Regression: [A | B], split B after → [A | B | C].
        let mut layout = PaneLayout::new_split(SplitDirection::Horizontal, vec![leaf(1), leaf(2)]);
        assert!(insert_split_at(
            &mut layout,
            2,
            SplitDirection::Horizontal,
            3,
            false
        ));
        assert_eq!(child_ids(&layout), vec![1, 2, 3]);
        assert_ratios_sum_to_one(&layout);
    }

    #[test]
    fn insert_node_at_case_a_inserts_multi_leaf_subtree() {
        // [A | B] horizontal, insert a 2-leaf vertical Split [C / D] next to
        // B (before=false) → [A | B | (C / D)] with the subtree landing
        // intact as a single unit, not flattened into individual leaves.
        let mut layout = PaneLayout::new_split(SplitDirection::Horizontal, vec![leaf(1), leaf(2)]);
        let subtree = PaneLayout::new_split(SplitDirection::Vertical, vec![leaf(3), leaf(4)]);
        assert!(insert_node_at(
            &mut layout,
            2,
            SplitDirection::Horizontal,
            subtree,
            false
        ));

        if let PaneLayout::Split {
            direction,
            children,
            ratios,
        } = &layout
        {
            assert_eq!(*direction, SplitDirection::Horizontal);
            assert_eq!(children.len(), 3);
            assert_eq!(ratios.len(), 3);
            // A and B (the original pair) retain their leaf identity.
            assert!(matches!(children[0], PaneLayout::Pane(1)));
            assert!(matches!(children[1], PaneLayout::Pane(2)));
            // The inserted subtree lands whole as the third child, both
            // leaves present and in order — not merged/flattened away.
            if let PaneLayout::Split {
                direction: inner_dir,
                children: inner,
                ..
            } = &children[2]
            {
                assert_eq!(*inner_dir, SplitDirection::Vertical);
                let ids: Vec<PaneId> = inner.iter().map(|c| c.first_leaf()).collect();
                assert_eq!(ids, vec![3, 4]);
            } else {
                panic!("expected inserted subtree to remain a nested Split");
            }
        } else {
            panic!("expected Split");
        }
        assert!(
            layout.contains(1) && layout.contains(2) && layout.contains(3) && layout.contains(4)
        );
        assert_ratios_sum_to_one(&layout);
    }

    #[test]
    fn insert_split_at_before_case_b() {
        // [A | B] horizontal, split B vertically before → [A | [C / B]].
        let mut layout = PaneLayout::new_split(SplitDirection::Horizontal, vec![leaf(1), leaf(2)]);
        assert!(insert_split_at(
            &mut layout,
            2,
            SplitDirection::Vertical,
            3,
            true
        ));
        if let PaneLayout::Split { children, .. } = &layout {
            assert_eq!(children.len(), 2);
            // Second child is the new vertical split with [C, B] order.
            if let PaneLayout::Split {
                direction,
                children: inner,
                ..
            } = &children[1]
            {
                assert_eq!(*direction, SplitDirection::Vertical);
                let ids: Vec<PaneId> = inner.iter().map(|c| c.first_leaf()).collect();
                assert_eq!(ids, vec![3, 2]);
            } else {
                panic!("expected nested vertical split");
            }
        } else {
            panic!("expected Split");
        }
        assert_ratios_sum_to_one(&layout);
    }

    #[test]
    fn insert_split_at_after_case_b() {
        // [A | B] horizontal, split B vertically after → [A | [B / C]].
        let mut layout = PaneLayout::new_split(SplitDirection::Horizontal, vec![leaf(1), leaf(2)]);
        assert!(insert_split_at(
            &mut layout,
            2,
            SplitDirection::Vertical,
            3,
            false
        ));
        if let PaneLayout::Split { children, .. } = &layout {
            assert_eq!(children.len(), 2);
            // Second child is the new vertical split with [B, C] order.
            if let PaneLayout::Split {
                direction,
                children: inner,
                ..
            } = &children[1]
            {
                assert_eq!(*direction, SplitDirection::Vertical);
                let ids: Vec<PaneId> = inner.iter().map(|c| c.first_leaf()).collect();
                assert_eq!(ids, vec![2, 3]);
            } else {
                panic!("expected nested vertical split");
            }
        } else {
            panic!("expected Split");
        }
        assert_ratios_sum_to_one(&layout);
    }

    #[test]
    fn rearrange_pane_absent_target_is_noop() {
        // target id 99 is not in the layout: returns false and leaves the
        // layout untouched (dragged must not be lost).
        let mut layout = PaneLayout::new_split(SplitDirection::Horizontal, vec![leaf(1), leaf(2)]);
        let before_ids = layout.pane_ids();
        let before_structure = child_ids(&layout);
        assert!(!rearrange_pane(&mut layout, 1, 99, DropHalf::West));
        assert_eq!(layout.pane_ids(), before_ids);
        assert_eq!(child_ids(&layout), before_structure);
    }

    #[test]
    fn rearrange_pane_absent_dragged_is_noop() {
        // target id 1 is present but dragged id 99 is not in the layout:
        // returns false and leaves the layout untouched.
        let mut layout = PaneLayout::new_split(SplitDirection::Horizontal, vec![leaf(1), leaf(2)]);
        let before_ids = layout.pane_ids();
        assert!(!rearrange_pane(&mut layout, 99, 1, DropHalf::West));
        assert_eq!(layout.pane_ids(), before_ids);
    }

    #[test]
    fn rearrange_pane_noop_same_id() {
        let mut layout = PaneLayout::new_split(SplitDirection::Horizontal, vec![leaf(1), leaf(2)]);
        let before_ids = layout.pane_ids();
        assert!(!rearrange_pane(&mut layout, 1, 1, DropHalf::West));
        assert_eq!(layout.pane_ids(), before_ids);
    }

    #[test]
    fn rearrange_pane_collapses_then_resplits() {
        // [A | B] horizontal; move B to A's South half.
        // Removing B collapses the root to leaf A, then insert_split_at
        // re-splits A vertically with B after → Vertical [A / B].
        let mut layout = PaneLayout::new_split(SplitDirection::Horizontal, vec![leaf(1), leaf(2)]);
        assert!(rearrange_pane(&mut layout, 2, 1, DropHalf::South));
        if let PaneLayout::Split {
            direction,
            children,
            ..
        } = &layout
        {
            assert_eq!(*direction, SplitDirection::Vertical);
            let ids: Vec<PaneId> = children.iter().map(|c| c.first_leaf()).collect();
            assert_eq!(ids, vec![1, 2]);
        } else {
            panic!("expected Vertical split");
        }
        assert_ratios_sum_to_one(&layout);
    }

    fn ratios_of(layout: &PaneLayout) -> Vec<f32> {
        match layout {
            PaneLayout::Split { ratios, .. } => ratios.clone(),
            PaneLayout::Pane(_) => vec![],
        }
    }

    #[test]
    fn rearrange_pane_to_adjacent_position_keeps_order_resets_ratios() {
        // [A | B | C] horizontal with skewed ratios; drop B onto A's East
        // half. East = same direction (Horizontal) + after A, so B lands
        // right back between A and C: leaf order is unchanged at [A, B, C].
        //
        // Ratios are NOT preserved: removing B redistributes [A, C] to
        // proportional shares, then the Case-A reinsert halves A's share and
        // hands the other half to B. So the rearranged pair (A, B) end up
        // equal to each other while C keeps its (renormalized) proportion.
        // This documents that an adjacent rearrange reshuffles ratios — it
        // is expected, not a bug.
        let mut layout =
            PaneLayout::new_split(SplitDirection::Horizontal, vec![leaf(1), leaf(2), leaf(3)]);
        if let PaneLayout::Split { ratios, .. } = &mut layout {
            *ratios = vec![0.6, 0.3, 0.1];
        }

        assert!(rearrange_pane(&mut layout, 2, 1, DropHalf::East));

        // Leaf order is preserved.
        assert_eq!(child_ids(&layout), vec![1, 2, 3]);
        assert_ratios_sum_to_one(&layout);

        // A and B (the rearranged pair) split their combined share equally.
        let ratios = ratios_of(&layout);
        assert_eq!(ratios.len(), 3);
        assert!(
            (ratios[0] - ratios[1]).abs() < 1e-4,
            "A and B should be equal, got {ratios:?}"
        );
        // C is distinct from the pair (the input was skewed), so this is not
        // a uniform three-way reset.
        assert!(
            (ratios[2] - ratios[0]).abs() > 1e-4,
            "C should differ from the rearranged pair, got {ratios:?}"
        );
    }

    #[test]
    fn rearrange_pane_three_pane_west_of_a() {
        // [A | B | C] horizontal; move C to A's West half → [C | A | B].
        let mut layout =
            PaneLayout::new_split(SplitDirection::Horizontal, vec![leaf(1), leaf(2), leaf(3)]);
        assert!(rearrange_pane(&mut layout, 3, 1, DropHalf::West));
        assert_eq!(child_ids(&layout), vec![3, 1, 2]);
        assert!(layout.contains(2) && layout.contains(3));
        assert_ratios_sum_to_one(&layout);
    }
}
