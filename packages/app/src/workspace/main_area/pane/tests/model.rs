use super::*;

#[test]
fn grid_resize_skips_unchanged_grid() {
    // Live-drag coalescing gate: a recomputed grid equal to the applied
    // grid must NOT be forwarded — skips the redundant PTY SIGWINCH +
    // ghostty reflow that sub-cell pixel churn fires on every bounds
    // notification during a drag (Retina: 1pt = 2px).
    assert!(!grid_resize_needed((80, 24), (80, 24)));
    // A real cell-boundary crossing in either dimension is forwarded.
    assert!(grid_resize_needed((80, 24), (81, 24)));
    assert!(grid_resize_needed((80, 24), (80, 25)));
}
