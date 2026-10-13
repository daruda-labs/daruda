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
    assert!(layout.contains(1) && layout.contains(2) && layout.contains(3) && layout.contains(4));
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
