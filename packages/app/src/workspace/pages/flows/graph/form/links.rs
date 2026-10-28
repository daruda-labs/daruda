//! Node choices distinguish execution order from output-file references.

use super::super::FlowGraphView;
use crate::surface::strings as s;
use crate::ui::select::{SelectEvent, SelectOption};
use crate::ui::{InputEvent, RopeExt as _};
use gpui::{Context, Window};

impl FlowGraphView {
    pub(in crate::workspace) fn refresh_form_links(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(form) = &self.form else {
            return;
        };
        let Some((deps, outputs)) = self.inspector.link_candidates() else {
            return;
        };
        for (state, label, values) in [
            (&form.dep_picker, s::flow::form_choose_dep(), deps),
            (&form.result_picker, s::flow::form_insert_result(), outputs),
        ] {
            let mut options = vec![SelectOption::new("", label)];
            options.extend(values.into_iter().map(SelectOption::simple));
            state.update(cx, |state, cx| {
                state.set_items(options, window, cx);
                state.set_selected_value(&"".into(), window, cx);
                cx.notify();
            });
        }
        cx.notify();
    }

    pub(in crate::workspace) fn choose_form_link(
        &mut self,
        result: bool,
        event: &crate::ui::select::ConfirmEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let SelectEvent::Confirm(Some(value)) = event else {
            return;
        };
        if value.is_empty() {
            return;
        }
        let Some(form) = &self.form else {
            return;
        };
        if result {
            let target = if form.body_states(cx).kind == super::KindChoice::Agent {
                &form.body.prompt.inline
            } else {
                &form.body.run
            };
            let placed = form.placed_carets.contains(&target.entity_id());
            target.update(cx, |input, cx| {
                if !placed {
                    let end = input.text().offset_to_position(input.text().len());
                    input.set_cursor_position(end, window, cx);
                }
                input.replace(format!("{{{{node.{value}.output}}}}"), window, cx);
                input.focus(window, cx);
                cx.notify();
            });
        } else {
            let mut deps = form.fields(cx).deps;
            if !deps.iter().any(|dep| dep == value.as_ref()) {
                deps.push(value.to_string());
            }
            self.set_form_deps(deps, window, cx);
            return;
        }
        self.on_form_changed(window, cx);
        self.refresh_form_links(window, cx);
    }

    pub(in crate::workspace) fn watch_result_targets(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(form) = &self.form else {
            return;
        };
        for target in [form.body.prompt.inline.clone(), form.body.run.clone()] {
            self._link_watches.push(cx.subscribe_in(
                &target,
                window,
                |view, input, event, _, _| {
                    if matches!(event, InputEvent::Focus) {
                        view.note_placed_caret(input.entity_id());
                    }
                },
            ));
        }
    }

    fn note_placed_caret(&mut self, input: gpui::EntityId) {
        if let Some(form) = &mut self.form
            && !form.placed_carets.contains(&input)
        {
            form.placed_carets.push(input);
        }
    }

    pub(in crate::workspace) fn remove_form_dep(
        &mut self,
        node: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(form) = &self.form {
            let mut deps = form.fields(cx).deps;
            deps.retain(|dep| dep != node);
            self.set_form_deps(deps, window, cx);
        }
    }
    fn set_form_deps(&mut self, deps: Vec<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_form(cx);
        let effects = self.inspector.edit_deps(deps.join(", "));
        self.project_form(window, cx);
        self.apply_inspector_effects(effects, window, cx);
    }
}

#[cfg(test)]
mod tests {}
