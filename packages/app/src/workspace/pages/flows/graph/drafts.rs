//! Draft lifetime and optimistic conflict checks, outside the render walk.

use super::{FlowGraphView, form};
use daruda_flow::NodeId;
use gpui::{App, Context, Window};

impl FlowGraphView {
    pub(super) fn stash_form(&mut self, cx: &App) {
        if let Some(form) = self.form.take().filter(|form| form.has_local_input(cx)) {
            self.drafts.insert(form.node.clone(), form.snapshot(cx));
        }
    }

    pub(super) fn restore_form_drafts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.reconcile_selection(window, cx);
        let conflict = crate::surface::strings::flow::form_draft_conflict();
        if self.draft_conflict() {
            self.save_error = Some(conflict);
        } else if self.save_error.as_ref() == Some(&conflict) {
            // Only the conflict banner is this reload's to retract; a refused
            // save's message stays until the next save attempt.
            self.save_error = None;
        }
    }

    pub(in crate::workspace) fn validate_form_drafts(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.draft_conflict() {
            self.set_save_error(
                Some(crate::surface::strings::flow::form_draft_conflict()),
                cx,
            );
            return false;
        }
        let invalid = self
            .drafts
            .values()
            .find(|draft| draft.invalid)
            .map(|draft| draft.node.clone())
            .or_else(|| {
                self.form
                    .as_ref()
                    .filter(|form| form.refusal(cx).is_some())
                    .map(|form| form.node.clone())
            });
        if let Some(node) = invalid {
            self.select_node(&node, window, cx);
            if let Some(form) = &mut self.form
                && matches!(
                    form.refusal(cx),
                    Some(
                        form::Refusal::Timeout(_)
                            | form::Refusal::Attempts(_)
                            | form::Refusal::Turns(_)
                    )
                )
            {
                form.settings_open = true;
            }
            self.set_save_error(
                Some(crate::surface::strings::flow::form_invalid_draft(node)),
                cx,
            );
            return false;
        }
        true
    }

    pub(in crate::workspace) fn draft_conflict(&self) -> bool {
        let Some(text) = self.text.as_deref() else {
            return !self.drafts.is_empty() || self.form.is_some();
        };
        self.drafts
            .values()
            .any(|draft| !draft.source_matches(text))
            || self
                .form
                .as_ref()
                .is_some_and(|form| !form.source_matches(text))
    }

    pub(in crate::workspace) fn pending_node_fields(
        &self,
        cx: &App,
    ) -> Vec<(NodeId, form::NodeFields)> {
        self.drafts
            .values()
            .map(|draft| (draft.node.clone(), draft.fields.clone()))
            .chain(
                self.form
                    .iter()
                    .filter(|form| form.is_dirty(cx))
                    .map(|form| (form.node.clone(), form.fields(cx))),
            )
            .collect()
    }

    /// Rebase drafts onto an edit this pane wrote — a deletion, connect, or
    /// disconnect — so the pane's own change does not read as a conflict.
    pub(in crate::workspace) fn finish_canvas_edit(
        &mut self,
        deleted: &[NodeId],
        before: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.stash_form(cx);
        let Some(after) = self.text.clone() else {
            return;
        };
        let issues = self.current_issues();
        self.drafts = std::mem::take(&mut self.drafts)
            .into_iter()
            .filter(|(node, _)| !deleted.contains(node))
            .map(|(node, draft)| {
                (
                    node,
                    draft.rebase(deleted, before, &after, &issues, window, cx),
                )
            })
            .collect();
        self.restore_form_drafts(window, cx);
        cx.notify();
    }

    pub(in crate::workspace) fn finish_form_save(
        &mut self,
        selected: Option<NodeId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.drafts.clear();
        self.form = None;
        if let Some(node) = selected {
            self.select_node(&node, window, cx);
        }
        self.save_error = None;
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    // Entity-backed draft switching and conflict behavior are covered by the
    // workspace flow inspector tests, where file reloads use the real host.
}
