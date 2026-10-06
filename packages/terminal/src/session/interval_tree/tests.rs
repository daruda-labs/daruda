use super::coord::{LineCoord, LineRange};
use super::payload::{AnnotationPayload, MarkPayload};
use super::persistence::{MarkRecord, RECORD_VERSION, replay};
use super::tree::{Color, IntervalTree, MarkId, NodeIdx};
use crate::session::line_buffer::LineBufferPosition;

// Tiny xorshift32 — deterministic, no dependency. Plenty of mixing for
// the 1000-op invariant test.
struct Rng(u32);

impl Rng {
    fn new(seed: u64) -> Self {
        // Avoid the zero fixpoint.
        let s = (seed as u32) ^ ((seed >> 32) as u32);
        Self(if s == 0 { 0xDA80_DA80 } else { s })
    }

    fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    fn range_u32(&mut self, n: u32) -> u32 {
        if n == 0 { 0 } else { self.next_u32() % n }
    }
}

fn buf(i: u64) -> LineCoord {
    LineCoord::Buffered(LineBufferPosition { abs_index: i })
}

fn vp(y: u64) -> LineCoord {
    LineCoord::Viewport { abs_y: y }
}

fn range(start: u64, end: u64) -> LineRange {
    LineRange::new(buf(start), buf(end))
}

// ----- coord.rs tests -----

#[test]
fn line_coord_ord_within_buffered() {
    assert!(buf(0) < buf(1));
    assert!(buf(5) > buf(4));
    assert_eq!(buf(7), buf(7));
}

#[test]
fn line_coord_ord_within_viewport() {
    assert!(vp(0) < vp(1));
    assert!(vp(100) > vp(50));
    assert_eq!(vp(3), vp(3));
}

#[test]
fn line_coord_buffered_strictly_less_than_viewport() {
    assert!(buf(u64::MAX) < vp(0));
    assert!(vp(0) > buf(u64::MAX));
    assert!(buf(10) < vp(5));
}

#[test]
fn line_range_overlaps_handcrafted() {
    let a = range(0, 5);
    // Identical
    assert!(a.overlaps(&range(0, 5)));
    // Fully inside
    assert!(a.overlaps(&range(1, 3)));
    // Partial left
    assert!(a.overlaps(&range(0, 0)));
    // Partial right
    assert!(a.overlaps(&range(5, 9)));
    // Adjacent on the right (inclusive ranges share endpoint 5)
    assert!(a.overlaps(&range(5, 10)));
    // Adjacent on the left (share endpoint 0)
    assert!(a.overlaps(&range(0, 0)));
    // Strictly above
    assert!(!a.overlaps(&range(6, 9)));
    // Strictly below — impossible here since start = 0; build a different a.
    let b = range(3, 5);
    assert!(!b.overlaps(&range(0, 2)));
}

#[test]
fn line_range_overlaps_across_kinds() {
    let a = LineRange::new(buf(0), vp(5));
    let b = LineRange::new(vp(1), vp(2));
    assert!(a.overlaps(&b));

    let only_buf = LineRange::new(buf(0), buf(100));
    let only_vp = LineRange::new(vp(0), vp(100));
    assert!(!only_buf.overlaps(&only_vp));
}

#[test]
fn line_range_contains_line() {
    let r = range(2, 8);
    assert!(r.contains_line(buf(2)));
    assert!(r.contains_line(buf(5)));
    assert!(r.contains_line(buf(8)));
    assert!(!r.contains_line(buf(1)));
    assert!(!r.contains_line(buf(9)));
    assert!(!r.contains_line(vp(5)));
}

// ----- Tree invariant checks -----

#[derive(Default)]
struct InvariantState {
    failure: Option<String>,
}

fn black_height<P>(tree: &IntervalTree<P>, idx: Option<NodeIdx>, st: &mut InvariantState) -> u32 {
    let Some(i) = idx else {
        return 1; // Conceptual NIL is black, contributes 1.
    };
    let node = tree.node(i);
    let lh = black_height(tree, node.left, st);
    let rh = black_height(tree, node.right, st);
    if lh != rh && st.failure.is_none() {
        st.failure = Some(format!(
            "black-height mismatch at idx {} (left={}, right={})",
            i, lh, rh
        ));
    }
    lh + if node.color == Color::Black { 1 } else { 0 }
}

fn check_no_red_red<P>(tree: &IntervalTree<P>, idx: Option<NodeIdx>, st: &mut InvariantState) {
    let Some(i) = idx else { return };
    let node = tree.node(i);
    if node.color == Color::Red {
        for child in [node.left, node.right].iter().flatten() {
            if tree.node(*child).color == Color::Red && st.failure.is_none() {
                st.failure = Some(format!("red-red violation at idx {}", i));
            }
        }
    }
    check_no_red_red(tree, node.left, st);
    check_no_red_red(tree, node.right, st);
}

fn check_max_end<P>(tree: &IntervalTree<P>, idx: Option<NodeIdx>, st: &mut InvariantState) {
    let Some(i) = idx else { return };
    let node = tree.node(i);
    let mut expected = node.range.end;
    if let Some(l) = node.left {
        let lm = tree.node(l).max_end;
        if lm > expected {
            expected = lm;
        }
    }
    if let Some(r) = node.right {
        let rm = tree.node(r).max_end;
        if rm > expected {
            expected = rm;
        }
    }
    if node.max_end != expected && st.failure.is_none() {
        st.failure = Some(format!(
            "max_end mismatch at idx {}: stored={:?}, expected={:?}",
            i, node.max_end, expected
        ));
    }
    check_max_end(tree, node.left, st);
    check_max_end(tree, node.right, st);
}

// Walks the tree with exclusive `(min, max)` bounds so every node — not just
// direct children — is checked against the BST ordering key. Catches
// grandchild violations a parent-only check would miss.
fn check_bst_order<P>(
    tree: &IntervalTree<P>,
    idx: Option<NodeIdx>,
    min: Option<(LineCoord, u64)>,
    max: Option<(LineCoord, u64)>,
    st: &mut InvariantState,
) {
    let Some(i) = idx else { return };
    let node = tree.node(i);
    let key = (node.range.start, node.id.0);
    if let Some(lo) = min
        && key <= lo
        && st.failure.is_none()
    {
        st.failure = Some(format!("BST lower-bound violation at idx {}", i));
    }
    if let Some(hi) = max
        && key >= hi
        && st.failure.is_none()
    {
        st.failure = Some(format!("BST upper-bound violation at idx {}", i));
    }
    check_bst_order(tree, node.left, min, Some(key), st);
    check_bst_order(tree, node.right, Some(key), max, st);
}

fn assert_invariants<P>(tree: &IntervalTree<P>) {
    let mut st = InvariantState::default();
    // Root is black (or empty).
    if let Some(r) = tree.root
        && tree.node(r).color != Color::Black
    {
        panic!("root is not black");
    }
    black_height(tree, tree.root, &mut st);
    check_no_red_red(tree, tree.root, &mut st);
    check_max_end(tree, tree.root, &mut st);
    check_bst_order(tree, tree.root, None, None, &mut st);

    if let Some(msg) = st.failure {
        panic!("RB invariant violated: {}", msg);
    }

    // by_id lookup matches the in-order traversal set.
    let in_order: std::collections::HashSet<_> = tree.iter().map(|m| m.id).collect();
    assert_eq!(in_order.len(), tree.len(), "in-order count mismatch");
    for id in in_order {
        assert!(
            tree.contains(id),
            "in-order traversal yielded id missing from by_id"
        );
    }
}

// ----- Random insert/remove stress -----

#[test]
fn rb_invariants_under_random_ops() {
    let mut rng = Rng::new(0xDA80_DA80);
    let mut tree: IntervalTree<u32> = IntervalTree::new();
    let mut live_ids: Vec<super::tree::MarkId> = Vec::new();

    for _ in 0..1000 {
        // 70% insert, 30% remove (when something is live).
        let do_insert = live_ids.is_empty() || rng.range_u32(10) < 7;
        if do_insert {
            let start = rng.range_u32(200) as u64;
            let span = rng.range_u32(20) as u64;
            let r = range(start, start + span);
            let payload = rng.next_u32();
            let id = tree.insert(r, payload);
            live_ids.push(id);
        } else {
            let idx = rng.range_u32(live_ids.len() as u32) as usize;
            let id = live_ids.swap_remove(idx);
            let removed = tree.remove(id);
            assert!(removed.is_some());
        }
        assert_invariants(&tree);
        assert_eq!(tree.len(), live_ids.len());
    }
}

// ----- overlap() correctness -----

#[test]
fn overlap_matches_brute_force() {
    let mut rng = Rng::new(0xDA80_DA80 ^ 0x1234_5678);
    let mut tree: IntervalTree<u32> = IntervalTree::new();
    for i in 0..200u32 {
        let start = (rng.range_u32(300)) as u64;
        let span = (rng.range_u32(40)) as u64;
        tree.insert(range(start, start + span), i);
    }

    for _ in 0..100 {
        let qs = (rng.range_u32(300)) as u64;
        let qe = qs + (rng.range_u32(50)) as u64;
        let q = range(qs, qe);

        let mut got: Vec<_> = tree.overlap(q).map(|m| m.id.0).collect();
        let mut expected: Vec<_> = tree
            .iter()
            .filter(|m| m.range.overlaps(&q))
            .map(|m| m.id.0)
            .collect();
        got.sort();
        expected.sort();
        assert_eq!(got, expected, "overlap mismatch for query {:?}", q);
    }
}

#[test]
fn at_line_equals_overlap_singleton() {
    let mut rng = Rng::new(0xDA80_DA80 ^ 0xC0FF_EE99);
    let mut tree: IntervalTree<u32> = IntervalTree::new();
    for i in 0..150u32 {
        let start = (rng.range_u32(200)) as u64;
        let span = (rng.range_u32(30)) as u64;
        tree.insert(range(start, start + span), i);
    }

    for _ in 0..50 {
        let l = (rng.range_u32(220)) as u64;
        let mut a: Vec<_> = tree.at_line(buf(l)).map(|m| m.id.0).collect();
        let mut b: Vec<_> = tree
            .overlap(LineRange::new(buf(l), buf(l)))
            .map(|m| m.id.0)
            .collect();
        a.sort();
        b.sort();
        assert_eq!(a, b);
    }
}

// ----- update_range parity with remove+insert -----

#[test]
fn update_range_matches_remove_insert() {
    // Build a tree, capture its iter() snapshot, then mutate one mark's
    // range two different ways and check the final iter() snapshots match.
    let mut rng = Rng::new(0xDA80_DA80 ^ 0xA5A5_A5A5);
    let mut tree_a: IntervalTree<u32> = IntervalTree::new();
    let mut tree_b: IntervalTree<u32> = IntervalTree::new();
    let mut ids_a = Vec::new();
    let mut ids_b = Vec::new();
    for i in 0..100u32 {
        let start = (rng.range_u32(200)) as u64;
        let span = (rng.range_u32(30)) as u64;
        let r = range(start, start + span);
        ids_a.push(tree_a.insert(r, i));
        ids_b.push(tree_b.insert(r, i));
    }

    // Pick 20 marks, swap each to a fresh range. tree_a uses update_range,
    // tree_b uses remove+insert. ids will diverge (re-insert allocates a
    // new id), so compare the (range, payload) multiset.
    for k in 0..20 {
        let pick = (rng.range_u32(ids_a.len() as u32)) as usize;
        let new_start = (rng.range_u32(250)) as u64;
        let new_span = (rng.range_u32(20)) as u64;
        let new_range = range(new_start, new_start + new_span);

        let id_a = ids_a[pick];
        let updated = tree_a.update_range(id_a, new_range);
        assert!(updated, "update_range on iter {} failed", k);

        let id_b = ids_b[pick];
        let payload = tree_b.remove(id_b).expect("remove failed");
        let new_id_b = tree_b.insert(new_range, payload);
        ids_b[pick] = new_id_b;
        assert_invariants(&tree_a);
        assert_invariants(&tree_b);
    }

    let mut va: Vec<(LineRange, u32)> = tree_a.iter().map(|m| (m.range, *m.payload)).collect();
    let mut vb: Vec<(LineRange, u32)> = tree_b.iter().map(|m| (m.range, *m.payload)).collect();
    va.sort_by_key(|(r, p)| (r.start.cmp_key(), r.end.cmp_key(), *p));
    vb.sort_by_key(|(r, p)| (r.start.cmp_key(), r.end.cmp_key(), *p));
    assert_eq!(va, vb);
}

// Helper trait so the test can derive a comparable sort key for LineCoord.
trait CoordKey {
    fn cmp_key(&self) -> (u8, u64);
}

impl CoordKey for LineCoord {
    fn cmp_key(&self) -> (u8, u64) {
        match self {
            LineCoord::Buffered(p) => (0, p.abs_index),
            LineCoord::Viewport { abs_y } => (1, *abs_y),
        }
    }
}

// ----- Smoke tests for basic API -----

#[test]
fn empty_tree() {
    let t: IntervalTree<()> = IntervalTree::new();
    assert!(t.is_empty());
    assert_eq!(t.len(), 0);
    assert_eq!(t.iter().count(), 0);
    assert_eq!(t.overlap(range(0, 10)).count(), 0);
    assert_invariants(&t);
}

#[test]
fn update_payload() {
    let mut t: IntervalTree<u32> = IntervalTree::new();
    let id = t.insert(range(0, 5), 1);
    assert!(t.update(id, |p| *p = 99));
    let got = t.iter().next().expect("one mark");
    assert_eq!(*got.payload, 99);
    assert_invariants(&t);
}

#[test]
fn iter_yields_inorder() {
    let mut t: IntervalTree<u32> = IntervalTree::new();
    // Insert in a deliberately unsorted order.
    t.insert(range(50, 55), 0);
    t.insert(range(10, 12), 1);
    t.insert(range(30, 40), 2);
    t.insert(range(20, 21), 3);
    t.insert(range(45, 46), 4);
    let starts: Vec<u64> = t
        .iter()
        .map(|m| match m.range.start {
            LineCoord::Buffered(p) => p.abs_index,
            LineCoord::Viewport { abs_y } => abs_y,
        })
        .collect();
    let mut sorted = starts.clone();
    sorted.sort();
    assert_eq!(starts, sorted);
    assert_invariants(&t);
}

// ----- Lifecycle hooks (payload.rs + lifecycle.rs) -----

fn annotation(text: &str) -> MarkPayload {
    MarkPayload::Annotation(AnnotationPayload::new(text.to_string()))
}

fn annotation_with_cols(text: &str, start_col: Option<u16>, end_col: Option<u16>) -> MarkPayload {
    let mut a = AnnotationPayload::new(text.to_string());
    a.start_col = start_col;
    a.end_col = end_col;
    MarkPayload::Annotation(a)
}

fn annotation_hidden(text: &str, hidden: bool) -> MarkPayload {
    let mut a = AnnotationPayload::new(text.to_string());
    a.hidden_in_alt_screen = hidden;
    MarkPayload::Annotation(a)
}

#[test]
fn evict_below_removes_correct_marks() {
    let mut t: IntervalTree<MarkPayload> = IntervalTree::new();
    // Build 5 marks with disjoint end positions so the contract
    // `range.end < threshold` is unambiguous.
    let id0 = t.insert(range(0, 5), annotation("a")); // end = 5  -> evicted
    let id1 = t.insert(range(6, 9), annotation("b")); // end = 9  -> evicted
    let id2 = t.insert(range(10, 10), annotation("c")); // end = 10 -> kept (not strictly less)
    let id3 = t.insert(range(11, 20), annotation("d")); // end = 20 -> kept
    let id4 = t.insert(range(30, 40), annotation("e")); // end = 40 -> kept

    let removed = t.evict_below(buf(10));

    // The two marks with end < 10 must be gone, the rest preserved.
    assert!(!t.contains(id0));
    assert!(!t.contains(id1));
    assert!(t.contains(id2));
    assert!(t.contains(id3));
    assert!(t.contains(id4));
    assert_eq!(t.len(), 3);

    // Iteration order matches in-order traversal at the time of the call.
    // id0 (start=0) comes before id1 (start=6).
    assert_eq!(removed, vec![id0, id1]);

    assert_invariants(&t);
}

#[test]
fn alt_screen_visibility_toggle() {
    let mut t: IntervalTree<MarkPayload> = IntervalTree::new();
    let hidden_id = t.insert(range(0, 1), annotation_hidden("hidden", true));
    let visible_id = t.insert(range(2, 3), annotation_hidden("visible", false));

    assert!(!t.alt_screen_active());

    // Inactive alt screen: both marks visible.
    let ids: Vec<_> = t.iter_visible().map(|m| m.id).collect();
    assert!(ids.contains(&hidden_id));
    assert!(ids.contains(&visible_id));
    assert_eq!(ids.len(), 2);

    // Activate alt screen: only the explicitly-visible one comes through.
    t.set_alt_screen_active(true);
    assert!(t.alt_screen_active());
    let ids: Vec<_> = t.iter_visible().map(|m| m.id).collect();
    assert_eq!(ids, vec![visible_id]);
    // Underlying iter() is unaffected by visibility filtering.
    let unfiltered: Vec<_> = t.iter().map(|m| m.id).collect();
    assert_eq!(unfiltered.len(), 2);
    assert!(unfiltered.contains(&hidden_id));
    assert!(unfiltered.contains(&visible_id));

    // Deactivate: both visible again.
    t.set_alt_screen_active(false);
    assert!(!t.alt_screen_active());
    let ids: Vec<_> = t.iter_visible().map(|m| m.id).collect();
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&hidden_id));
    assert!(ids.contains(&visible_id));
    // iter() still yields both regardless of the toggle.
    assert_eq!(t.iter().count(), 2);
}

#[test]
fn clamp_payload_cols_clamps_and_reports() {
    let mut t: IntervalTree<MarkPayload> = IntervalTree::new();
    let in_bounds = t.insert(range(0, 0), annotation_with_cols("in", Some(40), Some(60)));
    let needs_clamp = t.insert(range(1, 1), annotation_with_cols("clamp", None, Some(100)));
    let no_cols = t.insert(range(2, 2), annotation_with_cols("none", None, None));

    let changed = t.clamp_payload_cols(80);

    assert_eq!(changed, vec![needs_clamp]);

    // First mark stays put.
    let m0 = t.iter().find(|m| m.id == in_bounds).expect("present");
    match m0.payload {
        MarkPayload::Annotation(a) => {
            assert_eq!(a.start_col, Some(40));
            assert_eq!(a.end_col, Some(60));
        }
    }

    // Second mark's end_col has been clamped; start_col remains None
    // (None must NOT be promoted to Some(max_cols)).
    let m1 = t.iter().find(|m| m.id == needs_clamp).expect("present");
    match m1.payload {
        MarkPayload::Annotation(a) => {
            assert_eq!(a.start_col, None);
            assert_eq!(a.end_col, Some(80));
        }
    }

    // Third mark stays None / None.
    let m2 = t.iter().find(|m| m.id == no_cols).expect("present");
    match m2.payload {
        MarkPayload::Annotation(a) => {
            assert_eq!(a.start_col, None);
            assert_eq!(a.end_col, None);
        }
    }
}

#[test]
fn mark_payload_kind_tag() {
    let p = annotation("hello");
    assert_eq!(p.kind_tag(), "annotation");
}

// ----- Persistence (persistence.rs) -----

use std::sync::{Arc, Mutex};

use super::persistence::MarkRecordSink;

/// Test sink that shares its backing `Vec<MarkRecord>` with the test body
/// via `Arc<Mutex<_>>`. The `Box<dyn MarkRecordSink>` API does not provide
/// downcasting (no `Any` bound), so tests use shared state instead of
/// recovering records out of the boxed sink.
struct SharedSink(Arc<Mutex<Vec<MarkRecord>>>);

impl MarkRecordSink for SharedSink {
    fn append(&mut self, record: &MarkRecord) -> std::io::Result<()> {
        self.0.lock().expect("poisoned").push(record.clone());
        Ok(())
    }
}

fn install_shared_sink(tree: &mut IntervalTree<MarkPayload>) -> Arc<Mutex<Vec<MarkRecord>>> {
    let cell = Arc::new(Mutex::new(Vec::new()));
    tree.set_sink(Box::new(SharedSink(Arc::clone(&cell))));
    cell
}

fn drain(cell: &Arc<Mutex<Vec<MarkRecord>>>) -> Vec<MarkRecord> {
    std::mem::take(&mut *cell.lock().expect("poisoned"))
}

/// Build a `[(MarkId, LineRange, MarkPayload)]` snapshot of a tree's
/// in-order traversal so two trees can be compared by content without
/// caring about node identity.
fn snapshot(tree: &IntervalTree<MarkPayload>) -> Vec<(MarkId, LineRange, MarkPayload)> {
    let mut v: Vec<_> = tree
        .iter()
        .map(|m| (m.id, m.range, m.payload.clone()))
        .collect();
    v.sort_by_key(|(id, _, _)| id.0);
    v
}

#[test]
fn record_round_trip() {
    let mut tree: IntervalTree<MarkPayload> = IntervalTree::new();
    let cell = install_shared_sink(&mut tree);

    // 5 inserts.
    let id0 = tree.insert_payload(range(0, 5), annotation("a"));
    let id1 = tree.insert_payload(range(6, 9), annotation("b"));
    let id2 = tree.insert_payload(range(10, 15), annotation("c"));
    let id3 = tree.insert_payload(range(20, 25), annotation("d"));
    let id4 = tree.insert_payload(range(30, 40), annotation("e"));

    // 2 payload updates.
    assert!(tree.update_payload(id1, |p| match p {
        MarkPayload::Annotation(a) => a.text = "b-updated".to_string(),
    }));
    assert!(tree.update_payload(id3, |p| match p {
        MarkPayload::Annotation(a) => a.text = "d-updated".to_string(),
    }));

    // 1 range update.
    assert!(tree.update_payload_range(id4, range(35, 50)));

    // 2 removes.
    assert!(tree.remove_payload(id0).is_some());
    assert!(tree.remove_payload(id2).is_some());

    let records = drain(&cell);
    assert_eq!(records[0].seq(), 1, "first emitted record must carry seq=1");
    // 5 inserts + 2 payload updates + 1 range update + 2 removes = 10 emits.
    assert_eq!(records.len(), 10);
    let original = snapshot(&tree);

    // Wire the records to NDJSON lines, then replay.
    let lines: Vec<std::io::Result<String>> = records
        .iter()
        .map(|r| Ok(serde_json::to_string(r).expect("serialize")))
        .collect();
    let (mut replayed, stats) = replay(lines);

    assert_eq!(stats.applied, records.len());
    assert_eq!(stats.skipped_partial, 0);
    assert_eq!(stats.skipped_version, 0);
    assert_eq!(stats.skipped_unknown, 0);

    let restored = snapshot(&replayed);
    assert_eq!(
        restored, original,
        "replayed tree must match original by id+range+payload"
    );

    // next_id must be bumped past every replayed id so a fresh insert
    // does not collide with any replayed id.
    let max_replayed = restored.iter().map(|(id, _, _)| id.0).max().unwrap_or(0);
    let fresh_id = replayed.insert(range(100, 101), annotation("post"));
    assert!(
        fresh_id.0 > max_replayed,
        "post-replay insert id {} must exceed max replayed id {}",
        fresh_id.0,
        max_replayed
    );
}

#[test]
fn replay_skips_partial_last_line() {
    let mut tree: IntervalTree<MarkPayload> = IntervalTree::new();
    let cell = install_shared_sink(&mut tree);
    let _ = tree.insert_payload(range(0, 5), annotation("a"));
    let _ = tree.insert_payload(range(6, 9), annotation("b"));
    let _ = tree.insert_payload(range(10, 15), annotation("c"));

    let records = drain(&cell);

    let mut lines: Vec<std::io::Result<String>> = records
        .iter()
        .map(|r| Ok(serde_json::to_string(r).expect("serialize")))
        .collect();

    // Truncate the last line so it no longer parses.
    let last = lines.pop().expect("at least one line");
    let truncated = match last {
        Ok(s) => Ok(s[..s.len() / 2].to_string()),
        Err(_) => panic!("unreachable"),
    };
    lines.push(truncated);

    let (replayed, stats) = replay(lines);
    assert_eq!(stats.applied, records.len() - 1);
    assert_eq!(stats.skipped_partial, 1);
    assert_eq!(stats.skipped_version, 0);
    assert_eq!(stats.skipped_unknown, 0);

    // The tree should contain exactly the prefix (all records except the
    // last truncated Add).
    let prefix_ids: Vec<MarkId> = records[..records.len() - 1]
        .iter()
        .filter_map(|r| match r {
            MarkRecord::Add { id, .. } => Some(*id),
            _ => None,
        })
        .collect();
    let restored_ids: Vec<MarkId> = replayed.iter().map(|m| m.id).collect();
    let mut a = prefix_ids;
    let mut b = restored_ids;
    a.sort_by_key(|id| id.0);
    b.sort_by_key(|id| id.0);
    assert_eq!(a, b);
}

#[test]
fn replay_skips_wrong_version() {
    // Two valid records flanking one wrong-version record sandwiched in
    // the middle.
    let valid_a = MarkRecord::Add {
        v: RECORD_VERSION,
        ts: std::time::UNIX_EPOCH,
        seq: 1,
        id: MarkId(0),
        kind: "annotation".to_string(),
        range: range(0, 5),
        payload: annotation("a"),
    };
    let wrong = MarkRecord::Add {
        v: 999,
        ts: std::time::UNIX_EPOCH,
        seq: 2,
        id: MarkId(42),
        kind: "annotation".to_string(),
        range: range(6, 9),
        payload: annotation("future"),
    };
    let valid_b = MarkRecord::Add {
        v: RECORD_VERSION,
        ts: std::time::UNIX_EPOCH,
        seq: 3,
        id: MarkId(2),
        kind: "annotation".to_string(),
        range: range(10, 15),
        payload: annotation("c"),
    };

    let lines: Vec<std::io::Result<String>> = [&valid_a, &wrong, &valid_b]
        .iter()
        .map(|r| Ok(serde_json::to_string(r).expect("serialize")))
        .collect();

    let (tree, stats) = replay(lines);
    assert_eq!(stats.applied, 2);
    assert_eq!(stats.skipped_version, 1);
    assert_eq!(stats.skipped_partial, 0);
    assert_eq!(stats.skipped_unknown, 0);

    // Only the valid ids should be present.
    let mut ids: Vec<u64> = tree.iter().map(|m| m.id.0).collect();
    ids.sort();
    assert_eq!(ids, vec![0, 2]);
}

#[test]
fn set_alt_screen_active_does_not_emit() {
    let mut tree: IntervalTree<MarkPayload> = IntervalTree::new();
    let cell = install_shared_sink(&mut tree);

    tree.set_alt_screen_active(true);
    tree.set_alt_screen_active(false);
    tree.set_alt_screen_active(true);

    let records = drain(&cell);
    assert!(
        records.is_empty(),
        "alt-screen toggles must produce no NDJSON records"
    );
}

#[test]
fn evict_below_emits_remove_records() {
    let mut tree: IntervalTree<MarkPayload> = IntervalTree::new();
    let id0 = tree.insert(range(0, 5), annotation("a"));
    let id1 = tree.insert(range(6, 9), annotation("b"));
    let id2 = tree.insert(range(20, 30), annotation("c"));

    // Configure sink AFTER initial population so the sink only sees the
    // evictions, not the inserts.
    let cell = install_shared_sink(&mut tree);

    let removed = tree.evict_below(buf(15));
    assert_eq!(removed, vec![id0, id1]);

    let records = drain(&cell);
    assert_eq!(records.len(), 2);
    match &records[0] {
        MarkRecord::Remove { id, .. } => assert_eq!(*id, id0),
        other => panic!("expected Remove, got {:?}", other),
    }
    match &records[1] {
        MarkRecord::Remove { id, .. } => assert_eq!(*id, id1),
        other => panic!("expected Remove, got {:?}", other),
    }
    // The retained mark survives.
    assert!(tree.contains(id2));
}

// ----- Replay edge cases -----

#[test]
fn replay_empty_input_returns_empty_tree() {
    let (tree, stats) = replay(std::iter::empty::<std::io::Result<String>>());
    assert_eq!(tree.len(), 0, "empty input must yield an empty tree");
    assert_eq!(
        stats,
        super::persistence::ReplayStats {
            applied: 0,
            skipped_partial: 0,
            skipped_version: 0,
            skipped_unknown: 0,
        }
    );
}

#[test]
fn replay_update_for_missing_id_is_silently_ignored() {
    // An Update record for an id that was never Add-ed must be consumed
    // without error: applied increments (record parsed and version matched),
    // the tree stays empty, and the target id is absent.
    let target_id = MarkId(99);
    let update = MarkRecord::Update {
        v: RECORD_VERSION,
        ts: std::time::UNIX_EPOCH,
        seq: 1,
        id: target_id,
        range: None,
        payload: annotation("ghost"),
    };
    let lines: Vec<std::io::Result<String>> =
        vec![Ok(serde_json::to_string(&update).expect("serialize"))];

    let (tree, stats) = replay(lines);
    assert_eq!(
        stats.applied, 1,
        "Update must count as applied even when id is absent"
    );
    assert_eq!(stats.skipped_partial, 0);
    assert_eq!(stats.skipped_version, 0);
    assert_eq!(stats.skipped_unknown, 0);
    assert!(
        !tree.contains(target_id),
        "missing-id Update must leave the tree empty"
    );
    assert_eq!(tree.len(), 0);
}

#[test]
fn replay_mid_stream_parse_failure_is_not_misclassified_as_partial() {
    // Sequence: valid Add, garbage line, valid Add.
    // The last non-empty line is a valid record, so the garbage in the
    // middle must be counted as skipped_unknown, not skipped_partial.
    let payload_a = annotation("a");
    let add_a = MarkRecord::Add {
        v: RECORD_VERSION,
        ts: std::time::UNIX_EPOCH,
        seq: 1,
        id: MarkId(1),
        kind: payload_a.kind_tag().to_string(),
        range: range(0, 5),
        payload: payload_a,
    };
    let payload_b = annotation("b");
    // seq skips 2 on purpose — the garbage line occupies that slot; replay
    // does not enforce sequence ordering so this mirrors a real partial-write
    // scenario.
    let add_b = MarkRecord::Add {
        v: RECORD_VERSION,
        ts: std::time::UNIX_EPOCH,
        seq: 3,
        id: MarkId(2),
        kind: payload_b.kind_tag().to_string(),
        range: range(10, 15),
        payload: payload_b,
    };
    let lines: Vec<std::io::Result<String>> = vec![
        Ok(serde_json::to_string(&add_a).expect("serialize")),
        Ok("not json".to_string()),
        Ok(serde_json::to_string(&add_b).expect("serialize")),
    ];

    let (tree, stats) = replay(lines);
    assert_eq!(stats.applied, 2, "both valid Adds must be applied");
    assert_eq!(
        stats.skipped_unknown, 1,
        "garbage mid-stream counts as skipped_unknown"
    );
    assert_eq!(
        stats.skipped_partial, 0,
        "partial-write detector must not fire"
    );
    assert_eq!(stats.skipped_version, 0);
    assert!(tree.contains(add_a.id()), "first Add must be in the tree");
    assert!(tree.contains(add_b.id()), "second Add must be in the tree");
    assert_eq!(tree.len(), 2);
}

#[test]
fn clamp_payload_cols_emits_updates() {
    let mut tree: IntervalTree<MarkPayload> = IntervalTree::new();
    let _in_bounds = tree.insert(range(0, 0), annotation_with_cols("in", Some(40), Some(60)));
    let needs_clamp = tree.insert(range(1, 1), annotation_with_cols("clamp", None, Some(200)));
    let _no_cols = tree.insert(range(2, 2), annotation_with_cols("none", None, None));

    // Sink after population so we only see the clamp.
    let cell = install_shared_sink(&mut tree);

    let changed = tree.clamp_payload_cols(80);
    assert_eq!(changed, vec![needs_clamp]);

    let records = drain(&cell);
    assert_eq!(records.len(), 1);
    match &records[0] {
        MarkRecord::Update {
            id, range, payload, ..
        } => {
            assert_eq!(*id, needs_clamp);
            assert!(range.is_none(), "clamp emits payload-only updates");
            match payload {
                MarkPayload::Annotation(a) => {
                    assert_eq!(a.start_col, None);
                    assert_eq!(a.end_col, Some(80), "end_col must reflect post-clamp value");
                }
            }
        }
        other => panic!("expected Update, got {:?}", other),
    }
}

// ----- NdjsonFileSink adapter (ndjson_adapter.rs) -----

use daruda_store::marks::{NdjsonFileSink, marks_path, replay_iter};
use tempfile::TempDir;

/// Round-trip: open an NdjsonFileSink, drive Add/Update/Remove via the
/// payload wrappers, drop the tree, then replay the file and assert tree
/// equivalence (length and ids).
#[test]
fn ndjson_sink_adapter_replay_round_trip() {
    let dir = TempDir::new().expect("tempdir");
    let sink = NdjsonFileSink::open(dir.path()).expect("open NdjsonFileSink");
    let mut tree: IntervalTree<MarkPayload> = IntervalTree::new();
    tree.set_sink(Box::new(sink));

    let id0 = tree.insert_payload(range(0, 5), annotation("alpha"));
    let id1 = tree.insert_payload(range(10, 20), annotation("beta"));
    let id2 = tree.insert_payload(range(30, 40), annotation("gamma"));

    // Update id0's payload.
    tree.update_payload(id0, |p| {
        let MarkPayload::Annotation(a) = p;
        a.text = "alpha-updated".to_string();
    });
    // Remove id1.
    tree.remove_payload(id1);

    // Capture the pre-drop snapshot to verify against replay.
    let pre_drop = snapshot(&tree);

    // Drop the tree (and the sink it owns) to flush BufWriter.
    drop(tree);

    // Replay from the written file.
    let path = marks_path(dir.path());
    let iter = replay_iter(&path).expect("replay_iter");
    let (replayed, stats) = super::persistence::replay(iter);

    assert_eq!(stats.skipped_partial, 0, "no partial writes expected");
    assert_eq!(stats.skipped_unknown, 0, "no unknown records expected");
    assert_eq!(stats.skipped_version, 0, "no version mismatches expected");

    // Applied = 3 Add + 1 Update + 1 Remove = 5 records.
    assert_eq!(stats.applied, 5, "all five records must be applied");

    // id2 and id0 (updated) should be present; id1 should be absent.
    assert_eq!(replayed.len(), 2, "two marks remain after replay");
    assert!(replayed.contains(id0), "id0 must survive replay");
    assert!(!replayed.contains(id1), "id1 was removed — must be absent");
    assert!(replayed.contains(id2), "id2 must survive replay");

    // Content snapshot must match.
    let post_replay = snapshot(&replayed);
    assert_eq!(
        pre_drop, post_replay,
        "tree snapshot must be identical after replay"
    );
}

/// Truncation → replay skips the partial last line (`skipped_partial == 1`).
#[test]
fn ndjson_sink_truncation_replay_skips_partial() {
    use std::fs::OpenOptions;
    use std::io::Read;

    let dir = TempDir::new().expect("tempdir");
    let mut tree: IntervalTree<MarkPayload> = IntervalTree::new();
    let sink = NdjsonFileSink::open(dir.path()).expect("open");
    tree.set_sink(Box::new(sink));
    // Write three records.
    tree.insert_payload(range(0, 5), annotation("a"));
    tree.insert_payload(range(10, 15), annotation("b"));
    tree.insert_payload(range(20, 25), annotation("c"));
    // Drop the tree so the NdjsonFileSink's BufWriter flushes before we truncate.
    drop(tree);

    let path = marks_path(dir.path());
    let original_len = std::fs::metadata(&path).expect("metadata").len();
    assert!(
        original_len >= 5,
        "expected at least 5 bytes written, got {original_len}"
    );

    // Truncate the last 5 bytes to corrupt the final line.
    OpenOptions::new()
        .write(true)
        .open(&path)
        .expect("open for truncate")
        .set_len(original_len - 5)
        .expect("set_len");

    let iter = replay_iter(&path).expect("replay_iter");
    let (_replayed_tree, stats) = super::persistence::replay(iter);

    assert_eq!(stats.applied, 2, "first two records applied");
    assert_eq!(
        stats.skipped_partial, 1,
        "truncated last line is skipped_partial"
    );
    assert_eq!(stats.skipped_unknown, 0);
    assert_eq!(stats.skipped_version, 0);

    // Confirm the file was actually readable even without the bytes.
    let mut body = String::new();
    std::fs::File::open(&path)
        .expect("open for read")
        .read_to_string(&mut body)
        .expect("read");
    // There must be exactly 2 complete lines + a partial last line.
    let complete_lines = body
        .lines()
        .filter(|l| serde_json::from_str::<serde_json::Value>(l).is_ok())
        .count();
    assert!(
        complete_lines >= 2,
        "at least 2 complete JSON lines must survive"
    );
}
