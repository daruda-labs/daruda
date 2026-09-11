//! Controls how many trailing work steps remain visible in a response, and
//! how many calls remain visible inside one of those steps.

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

/// Which level of the recent-steps axis a value belongs to. One enum rather
/// than a pair of setters per level, so the panel's radio groups, the chip
/// menu's sections, the element ids and the reveal invalidation all derive
/// from one list.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::workspace) enum TailLevel {
    /// A response's work steps — one top-level tool run each.
    Steps,
    /// The calls inside one step: a tool group's own, and a subagent card's
    /// flattened children, which are one group of calls with no prose to split
    /// them further.
    Calls,
}

impl TailLevel {
    pub(in crate::workspace) const ALL: [Self; 2] = [Self::Steps, Self::Calls];

    /// Element-id and menu-section fragment.
    pub(in crate::workspace) fn token(self) -> &'static str {
        match self {
            Self::Steps => "steps",
            Self::Calls => "calls",
        }
    }
}

/// Both levels' resolved windows. The projection takes the pair rather than one
/// window per call site, so a level cannot read the other's value by accident.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::workspace) struct StepWindow {
    pub(in crate::workspace) steps: TailWindow,
    pub(in crate::workspace) calls: TailWindow,
}

impl StepWindow {
    #[cfg(test)]
    pub(in crate::workspace) fn get(self, level: TailLevel) -> TailWindow {
        match level {
            TailLevel::Steps => self.steps,
            TailLevel::Calls => self.calls,
        }
    }

    #[cfg(test)]
    pub(in crate::workspace) fn uniform(window: TailWindow) -> Self {
        Self {
            steps: window,
            calls: window,
        }
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
    fn a_pair_answers_per_level() {
        let pair = StepWindow {
            steps: TailWindow::Last(3),
            calls: TailWindow::All,
        };
        assert_eq!(pair.get(TailLevel::Steps), TailWindow::Last(3));
        assert_eq!(pair.get(TailLevel::Calls), TailWindow::All);
    }

    /// Every level the axis offers has to be reachable from `ALL` and carry a
    /// distinct id fragment — the panel, the menu and the element ids are all
    /// built from that list.
    #[test]
    fn every_level_is_listed_once_with_its_own_token() {
        let tokens: Vec<_> = TailLevel::ALL.iter().map(|l| l.token()).collect();
        assert_eq!(tokens, vec!["steps", "calls"]);
    }

    #[test]
    fn size_round_trips_every_offered_choice() {
        assert_eq!(TailWindow::All.size(), TAIL_WINDOW_ALL);
        for n in TAIL_WINDOW_CHOICES {
            assert_eq!(TailWindow::last(n).size(), n);
        }
    }
}
