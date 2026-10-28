//! Inspector commands at the GPUI boundary.
use super::{
    FlowGraphView,
    inspector::{Effects, Notice, SaveBlock, SavePlan},
};
use daruda_flow::NodeId;
use gpui::{App, Context, Window};

impl FlowGraphView {
    /// The sole input-to-model synchronization point, also used before commands
    /// that can run in the same update before queued Change events are delivered.
    pub(super) fn sync_form(&mut self, cx: &App) -> Effects {
        if let Some(form) = &self.form {
            return self
                .inspector
                .edit(&form.node, form.read(cx), form.hints(cx));
        }
        Effects::default()
    }

    pub(super) fn project_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let (Some(form), Some(draft)) = (&mut self.form, self.inspector.draft()) {
            form.write(draft.raw(), draft.ui(), window, cx);
        }
    }

    pub(super) fn apply_inspector_effects(
        &mut self,
        effects: Effects,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if effects.rebind {
            let issues = self.current_issues();
            self._form_watches.clear();
            self.form = self
                .inspector
                .draft()
                .map(|draft| super::form::FormBinding::bind(draft, &issues, window, cx));
            self.watch_form(window, cx);
        }
        if let Some(node) = effects.select_on_canvas {
            self.select_node(&node, window, cx);
        }
        if effects.refresh_choices && !effects.rebind {
            self.refresh_form_choices(window, cx);
        }
        if effects.refresh_links && !effects.rebind {
            self.refresh_form_links(window, cx);
        }
        #[cfg(test)]
        self.assert_binding(cx);
        cx.notify();
    }

    #[cfg(test)]
    fn assert_binding(&self, cx: &App) {
        match (&self.form, self.inspector.draft()) {
            (Some(binding), Some(draft)) => {
                assert_eq!(&binding.node, draft.node());
                assert_eq!(&binding.read(cx), draft.raw());
            }
            (None, None) => {}
            _ => panic!("selected draft and form binding must exist together"),
        }
    }

    pub(in crate::workspace) fn discarded_drafts(&self) -> &[NodeId] {
        match self.inspector.notice() {
            Some(Notice::Discarded(ids)) => ids,
            _ => &[],
        }
    }
    pub(in crate::workspace) fn dismiss_discarded_drafts(&mut self, cx: &mut Context<Self>) {
        self.inspector.dismiss_discarded();
        cx.notify();
    }
    pub(in crate::workspace) fn prepare_form_save(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<SavePlan> {
        self.sync_form(cx);
        match self.inspector.save_plan() {
            Ok(plan) => Some(plan),
            Err(SaveBlock::Conflict) => {
                self.inspector.set_notice(Some(Notice::Conflict));
                cx.notify();
                None
            }
            Err(SaveBlock::Invalid(node, refusal)) => {
                self.select_node(&node, window, cx);
                if let Some(form) = &mut self.form
                    && matches!(
                        refusal,
                        super::form::Refusal::Timeout(_)
                            | super::form::Refusal::Attempts(_)
                            | super::form::Refusal::Turns(_)
                    )
                {
                    form.settings_open = true;
                }
                self.inspector.set_notice(Some(Notice::InvalidDraft(node)));
                cx.notify();
                None
            }
        }
    }
    #[cfg(test)]
    pub(in crate::workspace) fn draft_conflict(&self) -> bool {
        self.inspector.status().conflict
    }
    pub(in crate::workspace) fn finish_canvas_edit(
        &mut self,
        deleted: &[NodeId],
        before: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sync_form(cx);
        let effects = self.inspector.canvas_edit(deleted, before);
        self.project_form(window, cx);
        self.apply_inspector_effects(effects, window, cx);
    }
    pub(in crate::workspace) fn finish_form_save(
        &mut self,
        selected: Option<NodeId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let effects = self.inspector.save_succeeded(selected);
        self.apply_inspector_effects(effects, window, cx);
    }
}

#[cfg(test)]
mod tests {}
