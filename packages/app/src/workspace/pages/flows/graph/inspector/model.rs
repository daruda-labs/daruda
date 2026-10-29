//! Encapsulated draft collection, selection and save protocol.
use super::{
    baseline::Baseline,
    draft::NodeDraft,
    fields::{NodeFields, Refusal},
    notes::{FieldNote, FormField, notes_for},
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

/// What the view must do to its widgets after a transition. The model never
/// touches GPUI; `rebind` rebuilds the selected node's widgets from the model.
#[derive(Default)]
pub(in crate::workspace) struct Effects {
    pub rebind: bool,
    /// The chrome (sections, banner, notes) may have changed and must be
    /// projected again. Off for an edit that left the notice alone, so a
    /// keystroke does not recompute it.
    pub chrome: bool,
    pub refresh_links: bool,
    pub refresh_choices: bool,
    pub select_on_canvas: Option<NodeId>,
}

/// The inspector chrome the widgets show, derived from the model only: which
/// sections are open, the refusal banner, and the notes pinned to fields.
#[derive(Clone, Debug, Default, PartialEq)]
pub(in crate::workspace) struct FormChrome {
    pub agent_open: bool,
    pub settings_open: bool,
    pub banner: Option<String>,
    pub notes: Vec<FieldNote>,
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
    pub(in crate::workspace) fn new(text: Option<Arc<str>>, issues: Vec<ValidationIssue>) -> Self {
        Self {
            baseline: text.map(|text| Baseline::new(text, issues)),
            drafts: BTreeMap::new(),
            selection: Selection::None,
            notice: None,
            discarded: Vec::new(),
        }
    }
    /// The bytes the pane draws and edits are made against; the model is
    /// their only owner.
    pub(in crate::workspace) fn text(&self) -> Option<&Arc<str>> {
        self.baseline.as_ref().map(Baseline::text)
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
        let notice = self.notice.clone();
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
            chrome: self.notice != notice,
            ..Effects::default()
        }
    }
    /// Notes for the selected node: a refused save's issues while that notice
    /// stands, the file's standing issues otherwise. A renamed node is matched
    /// by both ids, since the engine names it by the new one.
    pub(in crate::workspace) fn notes(&self) -> Vec<FieldNote> {
        let issues = match self.selected_refusal() {
            Some((_, issues)) => issues,
            None => self.standing_issues(),
        };
        self.notes_in(issues)
    }

    fn standing_issues(&self) -> &[ValidationIssue] {
        self.baseline.as_ref().map_or(&[][..], Baseline::issues)
    }

    /// The refused save, if it is about the selected node: the node it was
    /// shown on, or one its issues name by either identity.
    fn selected_refusal(&self) -> Option<(&str, &[ValidationIssue])> {
        let Some(Notice::Refused {
            message,
            issues,
            at,
        }) = &self.notice
        else {
            return None;
        };
        let draft = self.draft()?;
        let node = draft.node_ref();
        let named = issues
            .iter()
            .any(|issue| issue.node.as_ref().is_some_and(|id| node.matches(id)));
        (named || at.as_ref() == Some(draft.node()))
            .then_some((message.as_str(), issues.as_slice()))
    }

    fn notes_in(&self, issues: &[ValidationIssue]) -> Vec<FieldNote> {
        let Some(draft) = self.draft() else {
            return Vec::new();
        };
        let node = draft.node_ref();
        let mut notes = notes_for(issues, draft.node());
        if notes.is_empty() && node.renamed() {
            notes = notes_for(issues, &NodeId::from(node.edited()));
        }
        notes
    }

    pub(in crate::workspace) fn chrome(&self) -> FormChrome {
        let Some(draft) = self.draft() else {
            return FormChrome::default();
        };
        FormChrome {
            agent_open: draft.ui().agent_open,
            settings_open: draft.ui().settings_open,
            banner: self
                .selected_refusal()
                .map(|(message, _)| message.to_owned()),
            notes: self.notes(),
        }
    }

    pub(in crate::workspace) fn toggle_agent_section(&mut self) {
        if let Some(ui) = self.selected_ui() {
            ui.agent_open = !ui.agent_open;
        }
    }
    pub(in crate::workspace) fn toggle_settings_section(&mut self) {
        if let Some(ui) = self.selected_ui() {
            ui.settings_open = !ui.settings_open;
        }
    }
    #[cfg(feature = "screenshot")]
    pub(in crate::workspace) fn set_sections(&mut self, agent_open: bool, settings_open: bool) {
        if let Some(ui) = self.selected_ui() {
            ui.agent_open = agent_open;
            ui.settings_open = settings_open;
        }
    }
    fn selected_ui(&mut self) -> Option<&mut UiHints> {
        let id = self.selected()?.clone();
        Some(self.drafts.get_mut(&id)?.ui_mut())
    }
    /// A note is only seen when its section is open, so notes open theirs.
    /// `standing` limits this to the file's own issues — what a freshly
    /// selected node shows, whatever an earlier save left as a notice.
    pub(super) fn open_for_notes(&mut self, standing: bool) {
        let notes = if standing {
            self.notes_in(self.standing_issues())
        } else {
            self.notes()
        };
        let Some(ui) = self.selected_ui() else {
            return;
        };
        if notes.iter().any(|note| note.field == FormField::Agent) {
            ui.agent_open = true;
        }
        if notes.iter().any(|note| {
            matches!(
                note.field,
                FormField::Cwd | FormField::Fix | FormField::Rerun
            )
        }) {
            ui.settings_open = true;
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
        let local = self.draft().is_some_and(NodeDraft::has_unsaved_typing);
        InspectorStatus {
            unsaved: self.drafts.values().any(NodeDraft::has_unsaved_typing),
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
            .filter(|d| d.writes_changes())
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
    /// Show why `save_plan` refused: the conflict, or the invalid draft —
    /// selected, with the section holding its unreadable value opened.
    pub(in crate::workspace) fn save_blocked(&mut self, block: SaveBlock) -> Effects {
        match block {
            SaveBlock::Conflict => {
                self.notice = Some(Notice::Conflict);
                self.debug_assert_invariants();
                Effects {
                    chrome: true,
                    ..Effects::default()
                }
            }
            SaveBlock::Invalid(node, refusal) => {
                let mut effects = self.select(Some(node.clone()));
                effects.select_on_canvas = Some(node.clone());
                if matches!(
                    refusal,
                    Refusal::Timeout(_) | Refusal::Attempts(_) | Refusal::Turns(_)
                ) && let Some(ui) = self.selected_ui()
                {
                    ui.settings_open = true;
                }
                self.notice = Some(Notice::InvalidDraft(node));
                self.reconcile_notice();
                self.debug_assert_invariants();
                effects.chrome = true;
                effects
            }
        }
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
                .find(|d| d.has_unsaved_typing() && matches(d))
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
            at: self.selected().cloned(),
        });
        self.open_for_notes(false);
        self.debug_assert_invariants();
        effects.chrome = true;
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
mod tests {
    // See `inspector::tests`: the model is tested as a whole state machine.
}
