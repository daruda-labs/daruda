//! Public API tests for history.

use daruda_project::lane::history::HistoryBuffer;

#[test]
fn empty_push_ignored() {
    let mut h = HistoryBuffer::new();
    h.push("");
    assert!(h.prev("").is_none());
}

#[test]
fn single_entry_prev_returns_it() {
    let mut h = HistoryBuffer::new();
    h.push("ls");
    assert_eq!(h.prev(""), Some("ls"));
}

#[test]
fn dedup_consecutive() {
    let mut h = HistoryBuffer::new();
    h.push("ls");
    h.push("ls"); // duplicate — should be ignored
    // Only one entry: prev returns "ls" once, second prev stays at "ls".
    assert_eq!(h.prev(""), Some("ls"));
    // Clamped at oldest.
    assert_eq!(h.prev("ls"), Some("ls"));
}

#[test]
fn prev_next_traversal() {
    let mut h = HistoryBuffer::new();
    h.push("a");
    h.push("b");
    h.push("c");

    // ↑ → c (newest)
    assert_eq!(h.prev("draft"), Some("c"));
    // ↑ → b
    assert_eq!(h.prev("c"), Some("b"));
    // ↑ → a (oldest)
    assert_eq!(h.prev("b"), Some("a"));
    // ↑ again → clamp at a
    assert_eq!(h.prev("a"), Some("a"));
    // ↓ → b
    assert_eq!(h.forward(), Some("b"));
    // ↓ → c
    assert_eq!(h.forward(), Some("c"));
    // ↓ past newest → restore draft
    assert_eq!(h.forward(), Some("draft"));
    // ↓ again → None (not navigating)
    assert!(h.forward().is_none());
}

#[test]
fn draft_saved_and_restored() {
    let mut h = HistoryBuffer::new();
    h.push("cmd1");
    h.push("cmd2");

    let draft_text = "partial input";
    assert_eq!(h.prev(draft_text), Some("cmd2"));
    // Navigate all the way back, then forward past newest.
    h.prev("cmd2");
    h.forward();
    assert_eq!(h.forward(), Some(draft_text));
}

#[test]
fn push_resets_cursor() {
    let mut h = HistoryBuffer::new();
    h.push("old");
    h.prev(""); // start navigating
    h.push("new"); // should reset navigation
    // next() should return None (not navigating any more)
    assert!(h.forward().is_none());
    // prev() should start fresh from "new" (newest)
    assert_eq!(h.prev(""), Some("new"));
}

#[test]
fn next_when_not_navigating_returns_none() {
    let mut h = HistoryBuffer::new();
    h.push("x");
    assert!(h.forward().is_none());
}

#[test]
fn per_lane_isolation() {
    let mut lane1 = HistoryBuffer::new();
    let mut lane2 = HistoryBuffer::new();
    lane1.push("from-lane-1");
    // lane2 has no entries, prev returns None.
    assert!(lane2.prev("").is_none());
    // lane1 returns its own entry.
    assert_eq!(lane1.prev(""), Some("from-lane-1"));
}
