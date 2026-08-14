//! What the Flows panel reads off disk, kept until something makes it wrong.
//!
//! Two caches with one rule, which is why they are one type twice rather than
//! two fields and two paragraphs of comment. Both are read only while the panel
//! is showing — a tab nobody is on costs nothing — both belong to a single lane,
//! and both are dropped rather than patched, so there is exactly one way for
//! either to become right again.
//!
//! GPUI-free. "Is this still the lane's?" and "does this event make it wrong?"
//! are the questions that go wrong, and neither needs a window to ask.

use daruda_store::project::LaneRef;

/// A value read for one lane. `None` means "not read yet, or something made it
/// wrong" — the two are deliberately the same state, because the answer to both
/// is to read again.
pub(in crate::workspace) struct LaneCache<T> {
    held: Option<(LaneRef, T)>,
}

// Hand-written rather than derived: `derive(Default)` would demand `T: Default`,
// and nothing here needs an empty history or an empty listing to exist.
impl<T> Default for LaneCache<T> {
    fn default() -> Self {
        Self { held: None }
    }
}

impl<T> LaneCache<T> {
    /// What is held for `lane`, if what is held is that lane's.
    pub(in crate::workspace) fn get(&self, lane: LaneRef) -> Option<&T> {
        match &self.held {
            Some((held, value)) if *held == lane => Some(value),
            _ => None,
        }
    }

    pub(in crate::workspace) fn put(&mut self, lane: LaneRef, value: T) -> &T {
        self.held = Some((lane, value));
        // Just assigned.
        &self.held.as_ref().expect("just put").1
    }

    /// Drop whatever is held, whichever lane it belongs to. For a change that
    /// could have touched any lane's answer.
    pub(in crate::workspace) fn invalidate(&mut self) {
        self.held = None;
    }

    /// Drop it only if it is `lane`'s.
    ///
    /// Scoped because another lane's run says nothing about this lane's
    /// directory, and dropping the wrong one costs a re-read of the panel the
    /// person is actually looking at.
    pub(in crate::workspace) fn invalidate_for(&mut self, lane: LaneRef) {
        if self.get(lane).is_some() {
            self.held = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lane(id: u64) -> LaneRef {
        LaneRef {
            project: 1,
            lane: id,
        }
    }

    #[test]
    fn what_is_held_is_only_the_lane_it_was_read_for() {
        let mut cache: LaneCache<u32> = LaneCache::default();
        assert_eq!(cache.get(lane(1)), None, "nothing read yet");
        cache.put(lane(1), 7);
        assert_eq!(cache.get(lane(1)), Some(&7));
        assert_eq!(
            cache.get(lane(2)),
            None,
            "another lane's answer is not this one's"
        );
    }

    /// The rule worth a test: a run in another lane must not throw away the
    /// listing for the lane on screen.
    #[test]
    fn invalidating_for_a_lane_leaves_another_lanes_answer_alone() {
        let mut cache: LaneCache<u32> = LaneCache::default();
        cache.put(lane(1), 7);
        cache.invalidate_for(lane(2));
        assert_eq!(
            cache.get(lane(1)),
            Some(&7),
            "not that lane's, so untouched"
        );
        cache.invalidate_for(lane(1));
        assert_eq!(cache.get(lane(1)), None);
    }

    #[test]
    fn invalidating_outright_drops_whichever_lane_it_was() {
        let mut cache: LaneCache<u32> = LaneCache::default();
        cache.put(lane(3), 7);
        cache.invalidate();
        assert_eq!(cache.get(lane(3)), None);
    }

    /// Reading for a second lane replaces the first — one lane's worth is all
    /// the panel ever shows.
    #[test]
    fn a_second_lane_takes_the_place_of_the_first() {
        let mut cache: LaneCache<u32> = LaneCache::default();
        cache.put(lane(1), 7);
        cache.put(lane(2), 9);
        assert_eq!(cache.get(lane(2)), Some(&9));
        assert_eq!(cache.get(lane(1)), None);
    }
}
