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

pub(in crate::workspace) enum PaneLayout {
    Pane(PaneId),
    Split {
        direction: SplitDirection,
        children: Vec<PaneLayout>,
        ratios: Vec<f32>,
    },
}

impl PaneLayout {
    pub(in crate::workspace) fn new_split(direction: SplitDirection, children: Vec<PaneLayout>) -> PaneLayout {
        let n = children.len();
        debug_assert!(n >= 2, "Split must have at least two children");
        let ratios = vec![1.0 / n as f32; n];
        PaneLayout::Split {
            direction,
            children,
            ratios,
        }
    }

    #[cfg(test)]
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

/// Insert `new_id` next to `target`, following iTerm2's split rules.
///
/// Returns true when the target was found and the split applied.
///
/// Mirrors `PTYTab.m -splitVertically:newSession:before:targetSession:`:
///
/// - **Case C (root leaf):** replace root with a 2-child Split.
/// - **Case A (same direction parent):** insert `new_id` immediately after
///   `target` in the parent's children; redistribute ratios so the new pane
///   takes half of `target`'s share.
/// - **Case B (opposite direction parent):** replace the target leaf with a
///   new Split whose children are `[target, new]` in the requested direction.
pub(in crate::workspace) fn insert_split_at(
    layout: &mut PaneLayout,
    target: PaneId,
    direction: SplitDirection,
    new_id: PaneId,
) -> bool {
    // Case C: root is the target leaf.
    if let PaneLayout::Pane(id) = layout
        && *id == target
    {
        let original = PaneLayout::Pane(*id);
        *layout = PaneLayout::new_split(direction, vec![original, PaneLayout::Pane(new_id)]);
        return true;
    }

    if let PaneLayout::Split {
        direction: parent_dir,
        children,
        ratios,
    } = layout
    {
        // Direct child matches target?
        for i in 0..children.len() {
            if let PaneLayout::Pane(id) = children[i]
                && id == target
            {
                if *parent_dir == direction {
                    // Case A: splice next to target, halve its ratio.
                    let share = ratios[i] / 2.0;
                    ratios[i] = share;
                    ratios.insert(i + 1, share);
                    children.insert(i + 1, PaneLayout::Pane(new_id));
                } else {
                    // Case B: wrap target in a new Split.
                    let target_leaf = std::mem::replace(&mut children[i], PaneLayout::Pane(0));
                    children[i] = PaneLayout::new_split(
                        direction,
                        vec![target_leaf, PaneLayout::Pane(new_id)],
                    );
                }
                return true;
            }
        }

        // Target lives deeper — recurse. If a child Split contains target and
        // is in same direction as the requested split AND target is a direct
        // leaf of *this* level, it was already handled above. Otherwise recurse.
        for child in children.iter_mut() {
            if insert_split_at(child, target, direction, new_id) {
                return true;
            }
        }
    }

    false
}

/// Remove `target` and renormalize the tree so there are no stray
/// single-child Splits and no Splits sharing direction with their parent.
///
/// Returns true if the target was found and removed.
pub(in crate::workspace) fn remove_pane_from_layout(layout: &mut PaneLayout, target: PaneId) -> bool {
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
