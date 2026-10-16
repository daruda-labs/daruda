//! Turn outcomes the user has not seen yet, one per pane.
//!
//! In memory only: a restart starts with nothing unseen. When a pane counts
//! as seen is the owner's call (`agent_outcome_ops`); this type only keeps
//! the marks.

use std::collections::HashMap;

use daruda_agent::AgentOutcome;
use daruda_store::project::LaneRef;

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

    pub(in crate::workspace) fn by_lane(
        &self,
        pane_lane: &[(PaneId, LaneRef)],
    ) -> HashMap<LaneRef, AgentOutcome> {
        let mut outcomes = HashMap::new();
        for (pane, lane) in pane_lane {
            if let Some(outcome) = self.by_pane.get(pane) {
                outcomes
                    .entry(*lane)
                    .and_modify(|current| {
                        if *outcome == AgentOutcome::Errored {
                            *current = *outcome;
                        }
                    })
                    .or_insert(*outcome);
            }
        }
        outcomes
    }
}

#[cfg(test)]
mod tests;
