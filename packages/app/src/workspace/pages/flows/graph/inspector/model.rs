//! Encapsulated draft collection, selection and save protocol.
use super::{
    baseline::Baseline,
    draft::NodeDraft,
    fields::{NodeFields, Refusal},
    notice::Notice,
    raw::{RawInputs, UiHints},
};
use daruda_flow::{NodeId, error::ValidationIssue};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) enum Selection {
    #[default]
    None,
    Node(NodeId),
    Remembered(NodeId),
}

#[derive(Default)]
pub(in crate::workspace) struct Effects {
    pub rebind: bool,
    pub refresh_links: bool,
    pub refresh_choices: bool,
    pub select_on_canvas: Option<NodeId>,
}

pub(in crate::workspace) struct InspectorStatus {
    pub unsaved: bool,
    pub node_refused: bool,
    pub conflict: bool,
    pub node_revertable: bool,
}

pub(in crate::workspace) struct SavePlan {
    pub nodes: Vec<(NodeId, NodeFields)>,
    pub selected: Option<NodeId>,
}

#[derive(Debug)]
pub(in crate::workspace) enum SaveBlock {
    Conflict,
    Invalid(NodeId, Refusal),
}

pub(in crate::workspace) struct InspectorModel {
    pub(super) baseline: Option<Baseline>,
    pub(super) drafts: BTreeMap<NodeId, NodeDraft>,
    pub(super) selection: Selection,
    pub(super) notice: Option<Notice>,
    pub(super) discarded: Vec<NodeId>,
}

impl InspectorModel {
    pub(in crate::workspace) fn new(text: Option<Arc<str>>) -> Self {
        Self {
            baseline: text.map(Baseline::new),
            drafts: BTreeMap::new(),
            selection: Selection::None,
            notice: None,
            discarded: Vec::new(),
        }
    }
    pub(in crate::workspace) fn selected(&self) -> Option<&NodeId> {
        match &self.selection {
            Selection::Node(id) => Some(id),
            _ => None,
        }
    }
    pub(in crate::workspace) fn remembered(&self) -> Option<&NodeId> {
        match &self.selection {
            Selection::Remembered(id) => Some(id),
            _ => None,
        }
    }
    pub(in crate::workspace) fn draft(&self) -> Option<&NodeDraft> {
        self.drafts.get(self.selected()?)
    }
    pub(in crate::workspace) fn notice(&self) -> Option<&Notice> {
        self.notice.as_ref()
    }
    pub(in crate::workspace) fn set_notice(&mut self, notice: Option<Notice>) {
        self.notice = notice;
        self.reconcile_notice();
        self.debug_assert_invariants();
    }
    pub(in crate::workspace) fn dismiss_discarded(&mut self) {
        self.discarded.clear();
        self.reconcile_notice();
        self.debug_assert_invariants();
    }
    pub(in crate::workspace) fn title_edited(&mut self) {
        if !matches!(self.notice, Some(Notice::Discarded(_))) {
            self.notice = None;
        }
        self.reconcile_notice();
        self.debug_assert_invariants();
    }
    pub(in crate::workspace) fn edit(
        &mut self,
        node: &NodeId,
        raw: RawInputs,
        ui: UiHints,
    ) -> Effects {
        let mut links = false;
        let mut choices = false;
        if let Some(draft) = self.drafts.get_mut(node) {
            choices = draft.raw().agent_id != raw.agent_id;
            links = draft.raw().deps != raw.deps || draft.raw().kind != raw.kind;
            draft.edit(raw, ui);
        }
        self.reconcile_notice();
        self.debug_assert_invariants();
        Effects {
            refresh_links: links,
            refresh_choices: choices,
            ..Effects::default()
        }
    }
    pub(in crate::workspace) fn edit_deps(&mut self, deps: String) -> Effects {
        let Some(draft) = self.draft() else {
            return Effects::default();
        };
        let id = draft.node().clone();
        let mut raw = draft.raw().clone();
        raw.deps = deps;
        self.edit(&id, raw, draft.ui().clone())
    }

    pub(in crate::workspace) fn status(&self) -> InspectorStatus {
        let conflict = self.drafts.values().any(|draft| {
            self.baseline
                .as_ref()
                .is_none_or(|base| !draft.matches(base))
        });
        let local = self.draft().is_some_and(NodeDraft::has_input);
        InspectorStatus {
            unsaved: self.drafts.values().any(NodeDraft::has_input),
            conflict,
            node_refused: self
                .draft()
                .is_some_and(|d| d.introduced_refusal().is_some()),
            node_revertable: local
                || self
                    .draft()
                    .is_some_and(|d| self.baseline.as_ref().is_none_or(|base| !d.matches(base))),
        }
    }
    pub(in crate::workspace) fn pending_fields(&self) -> Vec<(NodeId, NodeFields)> {
        self.drafts
            .values()
            .filter(|d| d.is_modified())
            .map(|d| (d.node().clone(), d.fields()))
            .collect()
    }
    pub(in crate::workspace) fn save_plan(&self) -> Result<SavePlan, SaveBlock> {
        if self.status().conflict {
            return Err(SaveBlock::Conflict);
        }
        if let Some((id, refusal)) = self
            .drafts
            .values()
            .find_map(|d| d.introduced_refusal().map(|r| (d.node().clone(), r)))
        {
            return Err(SaveBlock::Invalid(id, refusal));
        }
        Ok(SavePlan {
            nodes: self.pending_fields(),
            selected: self
                .draft()
                .map(|draft| NodeId::from(draft.raw().id.trim())),
        })
    }
    pub(in crate::workspace) fn save_refused(
        &mut self,
        message: String,
        issues: &[ValidationIssue],
    ) -> Effects {
        let matches = |draft: &NodeDraft| {
            issues.iter().any(|issue| {
                issue
                    .node
                    .as_ref()
                    .is_some_and(|id| draft.node_ref().matches(id))
            })
        };
        let target = if self.draft().is_some_and(matches) {
            None
        } else {
            self.drafts
                .values()
                .find(|d| d.has_input() && matches(d))
                .map(|d| d.node().clone())
        };
        let mut effects = target
            .as_ref()
            .map(|id| self.select(Some(id.clone())))
            .unwrap_or_default();
        effects.select_on_canvas = target;
        self.notice = Some(Notice::Refused {
            message,
            issues: issues.to_vec(),
        });
        self.debug_assert_invariants();
        effects
    }
    pub(super) fn reconcile_notice(&mut self) {
        if self.status().conflict {
            self.notice = Some(Notice::Conflict);
            return;
        }
        if matches!(self.notice, Some(Notice::Conflict | Notice::Discarded(_)))
            || matches!(&self.notice, Some(Notice::InvalidDraft(id)) if self.drafts.get(id).is_none_or(|d| d.introduced_refusal().is_none()))
        {
            self.notice = None;
        }
        if self.notice.is_none() && !self.discarded.is_empty() {
            self.notice = Some(Notice::Discarded(self.discarded.clone()));
        }
    }
    pub(super) fn debug_assert_invariants(&self) {
        #[cfg(test)]
        self.assert_invariants();
    }
    #[cfg(test)]
    pub(super) fn assert_invariants(&self) {
        for (id, draft) in &self.drafts {
            assert_eq!(id, draft.node());
            if let Some(base) = &self.baseline
                && base.file().is_some()
            {
                assert!(base.node(id).is_some());
            }
            if let Some(base) = &self.baseline {
                assert!(draft.source() == base.text().as_ref() || !draft.matches(base));
            }
        }
        if let Selection::Node(id) = &self.selection {
            assert!(self.drafts.contains_key(id));
        }
        if matches!(self.notice, Some(Notice::Conflict)) {
            assert!(self.status().conflict);
        }
        if let Some(Notice::InvalidDraft(id)) = &self.notice {
            assert!(
                self.drafts
                    .get(id)
                    .is_some_and(|d| d.introduced_refusal().is_some())
            );
        }
        if let Some(Notice::Discarded(ids)) = &self.notice {
            assert_eq!(ids, &self.discarded);
        }
        if self.baseline.is_none() {
            assert!(!matches!(self.selection, Selection::Node(_)));
        }
    }
}

#[cfg(test)]
mod tests {}
