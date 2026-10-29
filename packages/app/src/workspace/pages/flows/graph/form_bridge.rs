//! The selected node's widgets, kept in step with the inspector model.
//!
//! Model → widgets through [`FlowGraphView::apply_inspector_effects`]; widgets
//! → model through [`FlowGraphView::sync_form`] only. Nothing here decides a
//! transition; it carries out what the model returned.

use gpui::{App, Context, Window};

use super::{
    FlowGraphView, form,
    inspector::{Effects, InspectorStatus},
};

pub(in crate::workspace) type FormStatus = InspectorStatus;

impl FlowGraphView {
    pub(in crate::workspace) fn set_catalog(
        &mut self,
        catalog: form::catalog::Catalog,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.catalog = catalog;
        self.refresh_form_choices(window, cx);
    }

    pub(super) fn refresh_form_choices(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(form) = &mut self.form {
            form.refresh_choices(&self.catalog, window, cx);
        }
        cx.notify();
    }

    /// The sole widget-to-model synchronization point. Also run before any
    /// command, since queued Change events may not have been delivered yet.
    /// Reading back exactly the model's values is no edit, which is what
    /// makes the Change events a projection causes harmless.
    pub(super) fn sync_form(&mut self, cx: &App) -> Effects {
        let (Some(form), Some(draft)) = (&self.form, self.inspector.draft()) else {
            return Effects::default();
        };
        let raw = form.read(cx);
        let hints = form.hints(draft.ui(), cx);
        if &raw == draft.raw() && &hints == draft.ui() {
            return Effects::default();
        }
        let node = form.node.clone();
        self.inspector.edit(&node, raw, hints)
    }

    /// Show the model's values in the existing widgets.
    pub(super) fn project_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let (Some(form), Some(draft)) = (&mut self.form, self.inspector.draft()) {
            form.write(draft.raw(), draft.ui(), window, cx);
        }
        self.project_chrome();
    }

    pub(super) fn project_chrome(&mut self) {
        let chrome = self.inspector.chrome();
        if let Some(form) = &mut self.form {
            form.project_chrome(chrome);
        }
    }

    pub(super) fn apply_inspector_effects(
        &mut self,
        effects: Effects,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if effects.rebind {
            self._form_watches.clear();
            let chrome = self.inspector.chrome();
            self.form = self
                .inspector
                .draft()
                .map(|draft| form::FormBinding::bind(draft, chrome, window, cx));
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
        self.project_chrome();
        #[cfg(test)]
        self.assert_binding(cx);
        cx.notify();
    }

    /// Effects of a transition that leaves nothing selected — the file going
    /// away — carried out where no window is at hand.
    pub(super) fn apply_unbound_effects(&mut self, effects: Effects, cx: &mut Context<Self>) {
        debug_assert!(self.inspector.draft().is_none() && effects.select_on_canvas.is_none());
        if effects.rebind {
            self.form = None;
            self._form_watches.clear();
            self._link_watches.clear();
        }
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

    pub(super) fn watch_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self._form_watches.clear();
        if let Some(form) = &self.form {
            for input in form.watch_inputs() {
                self._form_watches.push(cx.subscribe_in(
                    &input,
                    window,
                    |view, _, event, window, cx| {
                        if matches!(event, crate::ui::InputEvent::Change) {
                            view.on_form_changed(window, cx);
                        }
                    },
                ));
            }
            for select in form.watch_selects(cx) {
                self._form_watches.push(cx.subscribe_in(
                    &select,
                    window,
                    |view, _, _: &crate::ui::select::ConfirmEvent, window, cx| {
                        view.on_form_changed(window, cx)
                    },
                ));
            }
        }
        self._link_watches.clear();
        if let Some(form) = &self.form {
            self._link_watches.push(cx.subscribe_in(
                &form.dep_picker,
                window,
                |view, _, event, window, cx| view.choose_form_link(false, event, window, cx),
            ));
            self._link_watches.push(cx.subscribe_in(
                &form.result_picker,
                window,
                |view, _, event, window, cx| view.choose_form_link(true, event, window, cx),
            ));
        }
        self.watch_result_targets(window, cx);
        self.refresh_form_choices(window, cx);
        self.refresh_form_links(window, cx);
    }

    pub(super) fn on_form_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let effects = self.sync_form(cx);
        self.apply_inspector_effects(effects, window, cx);
    }

    /// Open or close the inspector's agent-override block.
    pub(in crate::workspace) fn toggle_agent_section(&mut self, cx: &mut Context<Self>) {
        self.sync_form(cx);
        self.inspector.toggle_agent_section();
        self.project_chrome();
        cx.notify();
    }

    pub(super) fn toggle_execution_section(&mut self, cx: &mut Context<Self>) {
        self.sync_form(cx);
        self.inspector.toggle_settings_section();
        self.project_chrome();
        cx.notify();
    }

    /// The form for the selected node, when there is one.
    #[cfg(any(test, feature = "screenshot"))]
    pub(in crate::workspace) fn form(&self) -> Option<&form::FormBinding> {
        self.form.as_ref()
    }
}

#[cfg(test)]
mod tests {
    // Binding behavior needs a window; the workspace flow tests drive it, and
    // `assert_binding` checks model/widget agreement after every effect.
}
