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

    #[test]
    fn size_round_trips_every_offered_choice() {
        assert_eq!(TailWindow::All.size(), TAIL_WINDOW_ALL);
        for n in TAIL_WINDOW_CHOICES {
            assert_eq!(TailWindow::last(n).size(), n);
        }
    }
}
