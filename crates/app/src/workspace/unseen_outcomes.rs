//! Turn outcomes the user has not seen yet, one per pane.
//!
//! In memory only: a restart starts with nothing unseen. When a pane counts
//! as seen is the owner's call (`agent_outcome_ops`); this type only keeps
//! the marks.

use std::collections::HashMap;

use daruda_agent::AgentOutcome;

use super::main_area::pane_tree::PaneId;

#[derive(Default)]
pub(in crate::workspace) struct UnseenOutcomes {
    by_pane: HashMap<PaneId, AgentOutcome>,
}

impl UnseenOutcomes {
    /// Keep `outcome` for `pane`. A later turn replaces an earlier one's
    /// outcome: the newest result is the one worth looking at.
    pub(in crate::workspace) fn record(&mut self, pane: PaneId, outcome: AgentOutcome) {
        self.by_pane.insert(pane, outcome);
    }

    /// Drop the marks on `panes`. Returns whether any were there.
    pub(in crate::workspace) fn forget(&mut self, panes: &[PaneId]) -> bool {
        let before = self.by_pane.len();
        self.by_pane.retain(|pane, _| !panes.contains(pane));
        self.by_pane.len() != before
    }

    /// Every pane holding a mark.
    pub(in crate::workspace) fn panes(&self) -> impl Iterator<Item = PaneId> + '_ {
        self.by_pane.keys().copied()
    }

    /// The unseen outcomes among `panes`. Which one a tab shows is
    /// `TabIndicator`'s call.
    pub(in crate::workspace) fn for_panes<'a>(
        &'a self,
        panes: &'a [PaneId],
    ) -> impl Iterator<Item = AgentOutcome> + 'a {
        panes
            .iter()
            .filter_map(|pane| self.by_pane.get(pane).copied())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: PaneId = 1;
    const B: PaneId = 2;

    #[test]
    fn a_later_outcome_replaces_an_earlier_one() {
        let mut unseen = UnseenOutcomes::default();
        unseen.record(A, AgentOutcome::Errored);
        unseen.record(A, AgentOutcome::Completed);
        assert_eq!(
            unseen.for_panes(&[A]).collect::<Vec<_>>(),
            [AgentOutcome::Completed]
        );
    }

    #[test]
    fn only_the_asked_panes_are_reported() {
        let mut unseen = UnseenOutcomes::default();
        unseen.record(A, AgentOutcome::Completed);
        unseen.record(B, AgentOutcome::Errored);
        assert_eq!(
            unseen.for_panes(&[B]).collect::<Vec<_>>(),
            [AgentOutcome::Errored]
        );
        assert_eq!(unseen.for_panes(&[]).count(), 0);
    }

    #[test]
    fn forgetting_reports_whether_anything_went() {
        let mut unseen = UnseenOutcomes::default();
        unseen.record(A, AgentOutcome::Completed);
        assert!(!unseen.forget(&[B]));
        assert!(unseen.forget(&[A, B]));
        assert_eq!(unseen.panes().count(), 0);
    }
}
