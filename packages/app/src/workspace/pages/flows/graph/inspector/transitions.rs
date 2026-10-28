//! Named state transitions; each finishes by checking the model invariants.
use super::{
    baseline::Baseline,
    draft::NodeDraft,
    model::{Effects, InspectorModel, Selection},
};
use daruda_flow::NodeId;
use std::sync::Arc;

impl InspectorModel {
    pub(in crate::workspace) fn select(&mut self, node: Option<NodeId>) -> Effects {
        if self.selected() == node.as_ref() {
            return Effects::default();
        }
        self.selection = node
            .and_then(|id| {
                if !self.drafts.contains_key(&id) {
                    let draft = NodeDraft::new(self.baseline.clone()?, id.clone())?;
                    self.drafts.insert(id.clone(), draft);
                }
                Some(Selection::Node(id))
            })
            .unwrap_or_default();
        self.reconcile_notice();
        self.debug_assert_invariants();
        Effects {
            rebind: true,
            refresh_links: true,
            ..Effects::default()
        }
    }

    pub(in crate::workspace) fn reload(&mut self, text: Option<Arc<str>>) -> Effects {
        let next = text.map(Baseline::new);
        let wanted = self.selected().or(self.remembered()).cloned();
        let mut rebind = false;
        let Some(base) = next.as_ref().filter(|b| b.file().is_some()) else {
            self.drafts.retain(|_, draft| draft.has_input());
            self.baseline = next;
            self.selection = wanted.map(Selection::Remembered).unwrap_or_default();
            self.reconcile_notice();
            self.debug_assert_invariants();
            return Effects {
                rebind: true,
                ..Effects::default()
            };
        };
        let mut discarded = self.discarded.clone();
        self.drafts.retain(|id, draft| {
            if base.node(id).is_none() {
                if draft.has_input() && !discarded.contains(id) {
                    discarded.push(id.clone());
                }
                rebind |= wanted.as_ref() == Some(id);
                return false;
            }
            if draft.matches(base) {
                draft.adopt(base.clone());
                true
            } else if draft.has_input() {
                true
            } else {
                rebind |= wanted.as_ref() == Some(id);
                false
            }
        });
        self.discarded = discarded;
        let restored = matches!(self.selection, Selection::Remembered(_));
        self.baseline = next;
        self.selection = Selection::None;
        self.reconcile_notice();
        if let Some(id) = wanted
            .as_ref()
            .filter(|id| self.baseline.as_ref().is_some_and(|b| b.node(id).is_some()))
        {
            self.select(Some(id.clone()));
        } else {
            rebind = true;
        }
        self.reconcile_notice();
        self.debug_assert_invariants();
        Effects {
            rebind: rebind || restored,
            refresh_links: true,
            select_on_canvas: self.selected().cloned(),
            ..Effects::default()
        }
    }

    pub(in crate::workspace) fn canvas_edit(
        &mut self,
        deleted: &[NodeId],
        before: &str,
    ) -> Effects {
        let Some(after) = self.baseline.clone().filter(|b| b.file().is_some()) else {
            return Effects::default();
        };
        let before = Baseline::new(Arc::from(before));
        let selected_before = self.draft().map(|d| d.raw().clone());
        self.drafts.retain(|id, _| !deleted.contains(id));
        for draft in self.drafts.values_mut() {
            draft.rebase_onto(&before, &after, deleted);
        }
        self.discarded.retain(|id| !deleted.contains(id));
        if self.selected().is_some_and(|id| deleted.contains(id)) {
            self.selection = Selection::None;
        }
        self.reconcile_notice();
        self.debug_assert_invariants();
        Effects {
            rebind: false,
            refresh_links: selected_before != self.draft().map(|d| d.raw().clone()),
            ..Effects::default()
        }
    }

    pub(in crate::workspace) fn revert_node(&mut self) -> Effects {
        let id = self.selected().cloned();
        if let Some(id) = &id {
            self.drafts.remove(id);
        }
        self.selection = Selection::None;
        let mut effects = self.select(id);
        effects.rebind = true;
        if !self.status().conflict {
            self.notice = None;
        }
        self.reconcile_notice();
        self.debug_assert_invariants();
        effects
    }

    pub(in crate::workspace) fn revert_all(&mut self) -> Effects {
        let selected = self.selected().cloned();
        let remembered = self.remembered().cloned();
        self.drafts.clear();
        self.discarded.clear();
        self.selection = Selection::None;
        self.notice = None;
        let mut effects = self.select(selected);
        if let Some(id) = remembered {
            self.selection = Selection::Remembered(id);
        }
        effects.rebind = true;
        self.debug_assert_invariants();
        effects
    }

    pub(in crate::workspace) fn save_succeeded(&mut self, selected: Option<NodeId>) -> Effects {
        self.drafts.clear();
        self.discarded.clear();
        self.selection = Selection::None;
        self.notice = None;
        let mut effects = self.select(selected.clone());
        effects.rebind = true;
        effects.select_on_canvas = selected;
        self.debug_assert_invariants();
        effects
    }

    pub(in crate::workspace) fn file_lost(&mut self) -> Effects {
        self.reload(None)
    }
}

#[cfg(test)]
mod tests {}
