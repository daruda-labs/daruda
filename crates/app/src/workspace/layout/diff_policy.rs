//! How a snapshot field takes part in the notify-on-change diff.
//!
//! A dock snapshot is compared against the previous frame's to decide whether
//! to dirty the cached dock view (see `render/mod.rs` and CLAUDE.md pitfall
//! #10). Most fields are content and must be compared; a few must not be, for
//! reasons that differ. Spelling that out as a wrapper type at the field, not
//! as a line in a hand-written comparison, is what makes the default safe:
//! a field added and forgotten is *included*, so the worst case is a repaint
//! that was not needed rather than a dock that never repaints at all.
//!
//! None of these wrappers implement `Clone` or `Copy`, on purpose. Without an
//! inherent one, `snap.field.clone()` resolves through `Deref` to the inner
//! value's `clone`, so wrapping a field leaves its read sites untouched. Add
//! `Clone` here and every one of them silently starts producing a wrapper.
//!
//! Nothing enforces that a wrapper's *name* is honest — `Handle<Vec<Lane>>`
//! compiles and would silently stop tracking real content. That is a review
//! check, which is why each wrapper is named for its reason rather than all of
//! them sharing one `Untracked`.
//!
//! One consequence to know in tests: `a.field == b.field` on two wrapped fields
//! is always `true`, so an `assert_eq!` on one passes vacuously. Compare the
//! inner values (`*a.now == *b.now`) when that is what you mean.

use std::ops::Deref;
use std::sync::Arc;

/// A GPUI handle or entity: the same instance for the whole window's lifetime,
/// so it can never be the thing that changed.
pub(in crate::workspace) struct Handle<T>(pub T);

impl<T> PartialEq for Handle<T> {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl<T> Deref for Handle<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

/// A value the staging step recomputes every frame — the render clock. Real
/// content (it is drawn), but comparing it would report a difference on every
/// frame and repaint forever.
///
/// This is sound only because staging assigns the snapshot unconditionally and
/// lets the diff gate the repaint alone (`render/mod.rs`). Gate the assignment
/// too and a `PerFrame` field freezes at whatever frame last changed something
/// else — drawn, but stale.
///
/// Reach for this only when the value genuinely changes on its own each frame.
/// It is not an escape hatch for content that repaints more often than you
/// would like: that is a staging problem, and silencing it here makes the dock
/// stale for every *other* reason the field changes.
pub(in crate::workspace) struct PerFrame<T>(pub T);

impl<T> PartialEq for PerFrame<T> {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl<T> Deref for PerFrame<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

/// Compared by `Arc` identity instead of content: for a cache that is cloned
/// forward until something invalidates it, so a shared pointer already means
/// "unchanged" — and whose element type may have no `PartialEq` at all.
pub(in crate::workspace) struct ByPointer<T>(pub Arc<T>);

impl<T> PartialEq for ByPointer<T> {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl<T> Deref for ByPointer<T> {
    type Target = Arc<T>;

    fn deref(&self) -> &Arc<T> {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole point of both wrappers: differing inner values still compare
    /// equal, so the field cannot contribute a difference.
    #[test]
    fn a_handle_and_a_per_frame_value_never_differ() {
        assert!(Handle(1) == Handle(2), "a handle never reports a change");
        assert!(
            PerFrame("monday") == PerFrame("tuesday"),
            "a per-frame value never reports a change"
        );
    }

    /// `ByPointer` tracks identity, not contents — two separately built `Arc`s
    /// holding equal data are a *difference*, because the cache was rebuilt.
    #[test]
    fn by_pointer_follows_arc_identity_not_contents() {
        let shared = Arc::new(vec![1, 2, 3]);
        assert!(
            ByPointer(shared.clone()) == ByPointer(shared),
            "the same Arc means the cache was carried forward"
        );
        assert!(
            ByPointer(Arc::new(vec![1, 2, 3])) != ByPointer(Arc::new(vec![1, 2, 3])),
            "equal contents behind a fresh Arc still mean the cache was rebuilt"
        );
    }

    /// Reading through a wrapper is what keeps render sites unchanged, and it
    /// only works while no wrapper has an inherent `clone`.
    #[test]
    fn a_wrapped_field_reads_as_the_value_it_wraps() {
        let name = Handle(String::from("dock"));
        assert_eq!(name.len(), 4);
        assert_eq!(name.clone(), String::from("dock"));

        let cache = ByPointer(Arc::new(vec![7]));
        let inner: Arc<Vec<i32>> = cache.clone();
        assert_eq!(inner.as_slice(), [7]);
    }
}
