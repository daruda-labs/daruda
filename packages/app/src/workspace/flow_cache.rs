//! What the Flows panel reads off disk, kept until something makes it wrong.
//!
//! Two caches with one rule, which is why they are one type twice rather than
//! two fields and two paragraphs of comment. Both are read only while the panel
//! is showing — a tab nobody is on costs nothing — both belong to a single lane,
//! both are read off the main thread, and both are re-read rather than
//! patched, so there is exactly one way for either to become right again.
//!
//! GPUI-free. "Is this still the lane's?" and "does this event make it wrong?"
//! are the questions that go wrong, and neither needs a window to ask.

use daruda_store::project::LaneRef;

/// A value read for one lane. [`LaneCache::get`] answers `None` for "not read
/// yet, or something made it wrong" — the two are deliberately the same state,
/// because the answer to both is to read again. The wrong value is kept only
/// to stay on screen until the re-read lands ([`LaneCache::shown`]).
pub(in crate::workspace) struct LaneCache<T> {
    held: Option<(LaneRef, T)>,
    /// Something made `held` wrong; it is shown only until a read replaces it.
    stale: bool,
    /// A read in flight off the main thread: its lane and the epoch it began
    /// in. Any invalidation moves the epoch on, so a read started before it
    /// lands nothing.
    reading: Option<(LaneRef, u64)>,
    epoch: u64,
}

// Hand-written rather than derived: `derive(Default)` would demand `T: Default`,
// and nothing here needs an empty history or an empty listing to exist.
impl<T> Default for LaneCache<T> {
    fn default() -> Self {
        Self {
            held: None,
            stale: false,
            reading: None,
            epoch: 0,
        }
    }
}

impl<T> LaneCache<T> {
    /// What is held for `lane`, if it is that lane's and still right.
    pub(in crate::workspace) fn get(&self, lane: LaneRef) -> Option<&T> {
        self.shown(lane).filter(|_| !self.stale)
    }

    /// What to draw for `lane` while a re-read is under way: the last value
    /// read for it, right or not, so the panel does not blank between reads.
    pub(in crate::workspace) fn shown(&self, lane: LaneRef) -> Option<&T> {
        match &self.held {
            Some((held, value)) if *held == lane => Some(value),
            _ => None,
        }
    }

    /// Hold `value` as right. Any read still in flight is older than it, so it
    /// lands nothing.
    pub(in crate::workspace) fn put(&mut self, lane: LaneRef, value: T) -> &T {
        self.reading = None;
        self.epoch += 1;
        self.held = Some((lane, value));
        self.stale = false;
        // Just assigned.
        &self.held.as_ref().expect("just put").1
    }

    /// Claim the read of `lane`'s value. `Some(ticket)` when nothing right is
    /// held for it and no current read is already under way; the ticket goes
    /// back with the value to [`Self::finish_read`].
    pub(in crate::workspace) fn begin_read(&mut self, lane: LaneRef) -> Option<u64> {
        if self.get(lane).is_some() || self.reading == Some((lane, self.epoch)) {
            return None;
        }
        self.reading = Some((lane, self.epoch));
        Some(self.epoch)
    }

    /// Mark it wrong, whichever lane it belongs to. For a change that could
    /// have touched any lane's answer.
    pub(in crate::workspace) fn invalidate(&mut self) {
        self.stale = true;
        self.epoch += 1;
    }

    /// Mark it wrong only if it is `lane`'s.
    ///
    /// Scoped because another lane's run says nothing about this lane's
    /// directory, and dropping the wrong one costs a re-read of the panel the
    /// person is actually looking at.
    pub(in crate::workspace) fn invalidate_for(&mut self, lane: LaneRef) {
        if self.shown(lane).is_some() {
            self.stale = true;
        }
        if matches!(self.reading, Some((reading, _)) if reading == lane) {
            self.epoch += 1;
        }
    }
}

impl<T: PartialEq> LaneCache<T> {
    /// Hold what read `ticket` found, unless something made it wrong since
    /// the read began. `true` when that changes what is drawn — a re-read
    /// that found what was already on screen costs no frame.
    pub(in crate::workspace) fn finish_read(
        &mut self,
        lane: LaneRef,
        ticket: u64,
        value: T,
    ) -> bool {
        if self.reading != Some((lane, ticket)) || ticket != self.epoch {
            return false;
        }
        self.reading = None;
        let changed = self.shown(lane) != Some(&value);
        self.put(lane, value);
        changed
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

    /// A read is claimed once, and what it finds is dropped when the lane's
    /// answer went wrong while it was under way.
    #[test]
    fn a_read_overtaken_by_an_invalidation_lands_nothing() {
        let mut cache: LaneCache<u32> = LaneCache::default();
        let ticket = cache.begin_read(lane(1)).expect("nothing held");
        assert_eq!(cache.begin_read(lane(1)), None, "already under way");
        cache.invalidate_for(lane(1));
        assert!(
            !cache.finish_read(lane(1), ticket, 7),
            "stale before it landed"
        );
        assert_eq!(cache.get(lane(1)), None);

        let ticket = cache.begin_read(lane(1)).expect("a fresh read");
        cache.invalidate_for(lane(2));
        assert!(
            cache.finish_read(lane(1), ticket, 9),
            "another lane's change"
        );
        assert_eq!(cache.get(lane(1)), Some(&9));
        assert_eq!(cache.begin_read(lane(1)), None, "held, nothing to read");
    }

    /// A wrong value stays on screen until its re-read lands, and a re-read
    /// that found the same thing changes nothing drawn.
    #[test]
    fn a_re_read_keeps_the_old_value_shown_until_it_lands() {
        let mut cache: LaneCache<u32> = LaneCache::default();
        cache.put(lane(1), 7);
        cache.invalidate_for(lane(1));
        assert_eq!(cache.get(lane(1)), None, "wrong, so read again");
        assert_eq!(cache.shown(lane(1)), Some(&7), "but not blanked");
        let ticket = cache.begin_read(lane(1)).expect("a re-read");
        assert!(!cache.finish_read(lane(1), ticket, 7), "the same, no frame");
        assert_eq!(cache.get(lane(1)), Some(&7));
        assert_eq!(cache.shown(lane(2)), None, "never another lane's");
    }

    /// A change to every lane, or a value put by hand, also overtakes a read;
    /// and a first read lands even when it found nothing.
    #[test]
    fn a_read_lands_once_and_only_while_current() {
        let mut cache: LaneCache<Vec<u32>> = LaneCache::default();
        let ticket = cache.begin_read(lane(1)).unwrap();
        cache.invalidate();
        assert!(!cache.finish_read(lane(1), ticket, vec![1]));

        let ticket = cache.begin_read(lane(1)).unwrap();
        cache.put(lane(1), vec![2]);
        assert!(
            !cache.finish_read(lane(1), ticket, vec![1]),
            "the put is newer"
        );
        assert_eq!(cache.get(lane(1)), Some(&vec![2]));

        let ticket = cache.begin_read(lane(2)).unwrap();
        assert!(
            cache.finish_read(lane(2), ticket, Vec::new()),
            "empty is news"
        );
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
