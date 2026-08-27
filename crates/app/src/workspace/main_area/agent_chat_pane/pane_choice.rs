//! Tracks whether a pane preference still follows config or was user-chosen.

/// A setting seeded from config and persisted only after user selection.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::workspace) enum PaneChoice<T> {
    Seeded(T),
    Chosen(T),
}

impl<T: Copy> PaneChoice<T> {
    pub(in crate::workspace) fn value(self) -> T {
        match self {
            Self::Seeded(value) | Self::Chosen(value) => value,
        }
    }

    pub(in crate::workspace) fn chosen(self) -> Option<T> {
        match self {
            Self::Chosen(value) => Some(value),
            Self::Seeded(_) => None,
        }
    }

    /// Follow a new config default. A pane the user has already decided for
    /// keeps its own value — that is what makes `Seeded` mean "still following
    /// config" rather than "happens to equal the old config".
    pub(in crate::workspace) fn reseed(&mut self, value: T) {
        if matches!(self, Self::Seeded(_)) {
            *self = Self::Seeded(value);
        }
    }
}

impl<T: Default> Default for PaneChoice<T> {
    fn default() -> Self {
        Self::Seeded(T::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_seeded_value_is_in_effect_but_not_the_users() {
        let seeded = PaneChoice::Seeded(7u8);
        assert_eq!(seeded.value(), 7);
        assert_eq!(seeded.chosen(), None, "nothing to persist");
    }

    #[test]
    fn a_chosen_value_is_both_in_effect_and_persisted() {
        let chosen = PaneChoice::Chosen(7u8);
        assert_eq!(chosen.value(), 7);
        assert_eq!(chosen.chosen(), Some(7));
    }

    #[test]
    fn choosing_the_seeded_value_still_records_a_choice() {
        let seeded = PaneChoice::Seeded(7u8);
        let chosen = PaneChoice::Chosen(seeded.value());
        assert_eq!(chosen.value(), seeded.value());
        assert_ne!(chosen, seeded);
        assert_eq!(chosen.chosen(), Some(7));
    }

    #[test]
    fn reseeding_moves_a_seeded_value_and_leaves_a_chosen_one() {
        let mut seeded = PaneChoice::Seeded(7u8);
        seeded.reseed(9);
        assert_eq!(seeded, PaneChoice::Seeded(9), "an untouched pane follows");

        let mut chosen = PaneChoice::Chosen(7u8);
        chosen.reseed(9);
        assert_eq!(
            chosen,
            PaneChoice::Chosen(7),
            "a user choice is not config's"
        );
    }

    #[test]
    fn default_seeds_the_types_own_default() {
        assert_eq!(PaneChoice::<u8>::default(), PaneChoice::Seeded(0));
    }
}
