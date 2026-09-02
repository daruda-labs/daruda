//! Controls how many trailing work steps remain visible in a response.

use daruda_config::TAIL_WINDOW_ALL;

/// How many of a response's trailing work Steps render.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::workspace) enum TailWindow {
    #[default]
    All,
    /// Keep only the last `n` steps.
    Last(usize),
}

impl TailWindow {
    /// `0` is the configured sentinel for [`Self::All`].
    pub(in crate::workspace) fn last(n: u8) -> Self {
        if n == TAIL_WINDOW_ALL {
            Self::All
        } else {
            Self::Last(usize::from(n))
        }
    }

    pub(in crate::workspace) fn size(self) -> u8 {
        match self {
            Self::All => TAIL_WINDOW_ALL,
            Self::Last(n) => u8::try_from(n).unwrap_or(u8::MAX),
        }
    }

    /// Number of leading steps outside the window.
    pub(in crate::workspace) fn hidden_steps(self, step_count: usize) -> usize {
        match self {
            Self::All => 0,
            Self::Last(n) => step_count.saturating_sub(n),
        }
    }

    pub(in crate::workspace) fn hides(self, step_ix: usize, step_count: usize) -> bool {
        step_ix < self.hidden_steps(step_count)
    }
}

/// Whether a subagent card's own boundary is holding this child back.
///
/// `pos`/`count` are the child's place among the calls the card collected. A
/// subagent contributes only tool calls to the conversation, so there is no
/// prose to split them into steps and the card's children are one group of
/// calls — the axis counts them the way a tool group counts its own. A revealed
/// boundary and a running call both escape it, the latter exactly as a live
/// call escapes its tool group's boundary one level up.
pub(in crate::workspace) fn subagent_child_withheld(
    pos: usize,
    count: usize,
    tail: TailWindow,
    revealed: bool,
    live: bool,
) -> bool {
    !revealed && !live && tail.hides(pos, count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use daruda_config::TAIL_WINDOW_CHOICES;

    #[test]
    fn all_hides_nothing() {
        assert_eq!(TailWindow::All.hidden_steps(58), 0);
        assert!(!TailWindow::All.hides(0, 58));
        assert!(!TailWindow::All.hides(57, 58));
    }

    #[test]
    fn a_window_keeps_the_last_n_steps() {
        let tail = TailWindow::Last(3);
        assert_eq!(tail.hidden_steps(10), 7);
        assert!(tail.hides(6, 10));
        assert!(!tail.hides(7, 10));
        assert!(!tail.hides(9, 10));
    }

    #[test]
    fn a_response_with_fewer_steps_than_the_window_is_untouched() {
        let tail = TailWindow::Last(5);
        assert_eq!(tail.hidden_steps(5), 0);
        assert_eq!(tail.hidden_steps(2), 0);
        assert_eq!(tail.hidden_steps(0), 0);
        assert!(!tail.hides(0, 2));
    }

    #[test]
    fn a_window_of_one_keeps_only_the_running_cycle() {
        let tail = TailWindow::Last(1);
        assert_eq!(tail.hidden_steps(4), 3);
        assert!(tail.hides(2, 4));
        assert!(!tail.hides(3, 4));
    }

    #[test]
    fn the_zero_sentinel_is_the_no_window_state() {
        assert_eq!(TailWindow::last(TAIL_WINDOW_ALL), TailWindow::All);
        assert_eq!(TailWindow::last(5), TailWindow::Last(5));
    }

    /// The card's boundary keeps the last `n` of its children, and both
    /// escapes work: opening it, and a child that is still running.
    #[test]
    fn a_subagent_cards_boundary_holds_back_all_but_the_last_calls() {
        let tail = TailWindow::Last(2);
        let held: Vec<bool> = (0..5)
            .map(|pos| subagent_child_withheld(pos, 5, tail, false, false))
            .collect();
        assert_eq!(held, vec![true, true, true, false, false]);

        assert!(
            (0..5).all(|pos| !subagent_child_withheld(pos, 5, tail, true, false)),
            "opening the boundary releases every child"
        );
        assert!(
            !subagent_child_withheld(0, 5, tail, false, true),
            "a running child stays surfaced through a shut boundary"
        );
    }

    #[test]
    fn a_card_with_no_window_holds_back_nothing() {
        for count in [0usize, 1, 9] {
            assert!(
                (0..count).all(|pos| !subagent_child_withheld(
                    pos,
                    count,
                    TailWindow::All,
                    false,
                    false
                )),
                "count={count}"
            );
        }
        // A window at or above the child count is the same answer.
        assert!((0..3).all(|pos| !subagent_child_withheld(
            pos,
            3,
            TailWindow::Last(3),
            false,
            false
        )));
    }

    #[test]
    fn size_round_trips_every_offered_choice() {
        assert_eq!(TailWindow::All.size(), TAIL_WINDOW_ALL);
        for n in TAIL_WINDOW_CHOICES {
            assert_eq!(TailWindow::last(n).size(), n);
        }
    }
}
