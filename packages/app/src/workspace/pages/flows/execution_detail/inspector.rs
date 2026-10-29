//! Selection and evidence loading change together, including stale-read rejection.

use super::{Evidence, EvidenceTab, ExecutionSelection};
use daruda_flow::{NodeId, history::RunHistory};
use std::path::{Path, PathBuf};

#[derive(Default)]
pub(in crate::workspace) struct AttemptInspector {
    selection: Option<ExecutionSelection>,
    tab: EvidenceTab,
    evidence: Evidence,
    epoch: u64,
}

pub(super) struct EvidenceRead {
    pub epoch: u64,
    pub path: Option<PathBuf>,
}

impl AttemptInspector {
    pub fn selection(&self) -> Option<&ExecutionSelection> {
        self.selection.as_ref()
    }
    pub fn tab(&self) -> EvidenceTab {
        self.tab
    }
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }

    fn invalidate(&mut self) {
        self.epoch += 1;
        self.evidence = Evidence::Empty;
    }

    pub fn restore_selection(&mut self, selection: ExecutionSelection) {
        self.selection = Some(selection);
        self.invalidate();
    }

    pub fn reconcile(&mut self, history: &RunHistory) {
        let selection = match &self.selection {
            None => history
                .latest_selection()
                .map(|(node, seq)| ExecutionSelection::Attempt(node, seq))
                .or_else(|| {
                    history
                        .nodes()
                        .into_iter()
                        .next()
                        .map(ExecutionSelection::Node)
                }),
            Some(ExecutionSelection::Node(node)) => {
                Some(ExecutionSelection::for_node(history, node.clone()))
            }
            Some(selection) => Some(selection.clone()),
        };
        if self.selection != selection {
            self.selection = selection;
            self.invalidate();
        }
    }

    pub fn select_node(&mut self, history: &RunHistory, node: NodeId) -> bool {
        if self.selection.as_ref().is_some_and(|s| s.node() == &node) {
            return false;
        }
        self.restore_selection(ExecutionSelection::for_node(history, node));
        true
    }

    pub fn select_attempt(&mut self, history: &RunHistory, evidence: u32) -> bool {
        let Some(selection) = &self.selection else {
            return false;
        };
        let node = selection.node();
        if !history
            .attempts(node)
            .iter()
            .any(|(_, seq)| *seq == evidence)
        {
            return false;
        }
        self.restore_selection(ExecutionSelection::Attempt(node.clone(), evidence));
        true
    }

    pub fn select_tab(&mut self, tab: EvidenceTab) -> bool {
        if self.tab == tab {
            return false;
        }
        self.tab = tab;
        self.invalidate();
        true
    }

    pub fn selected_path(&self, history: &RunHistory, dir: &Path) -> Option<PathBuf> {
        let (node, seq) = self.selection.as_ref()?.attempt()?;
        match self.tab {
            EvidenceTab::Conversation => history.transcript(dir, node, *seq),
            EvidenceTab::Output => history.output(dir, node, *seq),
            EvidenceTab::Metadata => None,
        }
    }

    pub(super) fn begin_read(&mut self, history: &RunHistory, dir: &Path) -> Option<EvidenceRead> {
        self.invalidate();
        if self.tab == EvidenceTab::Metadata
            || self
                .selection
                .as_ref()
                .and_then(ExecutionSelection::attempt)
                .is_none()
        {
            return None;
        }
        self.evidence = Evidence::Loading;
        Some(EvidenceRead {
            epoch: self.epoch,
            path: self.selected_path(history, dir),
        })
    }

    pub(super) fn accept(&mut self, epoch: u64, evidence: Evidence) -> bool {
        if epoch != self.epoch {
            return false;
        }
        self.evidence = evidence;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn navigation_invalidates_pending_evidence() {
        let mut inspector = AttemptInspector::default();
        let epoch = inspector.epoch;
        inspector.select_tab(EvidenceTab::Output);
        assert!(!inspector.accept(epoch, Evidence::Missing));
        let epoch = inspector.epoch;
        inspector.restore_selection(ExecutionSelection::Node("pending".into()));
        assert!(!inspector.accept(epoch, Evidence::Missing));
        assert!(matches!(inspector.evidence(), Evidence::Empty));
    }
}
