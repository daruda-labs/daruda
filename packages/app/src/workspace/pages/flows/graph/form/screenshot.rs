//! Isolated native form captures, including long prose and the expanded editor.
use crate::workspace::Workspace;
use gpui::{Context, Window};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FlowFormShot {
    Basic,
    English,
    Expanded,
    Command,
}

impl Workspace {
    pub(in crate::workspace) fn open_flow_form_for_shot(
        &mut self,
        shot: FlowFormShot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if shot == FlowFormShot::English {
            crate::globals::apply_locale_str("en");
        }
        self.open_authoring_flow_graph_for_shot(window, cx);
        let Some((_, _, view)) = self.open_graph() else {
            return;
        };
        view.update(cx, |view, cx| {
            view.select_node_for_shot(&"build".into(), window, cx);
            view.update_inspector(|model| model.set_sections(false, shot == FlowFormShot::Command), cx);
            if let Some(form) = &mut view.form {
                if shot == FlowFormShot::Command {
                    form.body.kind.update(cx, |select, cx| select.set_selected_value(&super::KIND_COMMAND.into(), window, cx));
                    form.body.run.update(cx, |input, cx| input.set_value("cargo test --workspace", window, cx));
                } else {
                    form.body.prompt.inline.update(cx, |input, cx| input.set_value("Read {{node.design.output}} as a file.\nReview the implementation against the acceptance criteria.\n\nPreserve existing behavior and explain the changes. Check keyboard navigation, long labels, and narrow windows before saving the final report.", window, cx));
                }
            }
            if shot == FlowFormShot::Expanded { view.expand_prompt(window, cx); }
            cx.notify();
        });
    }
}

#[cfg(test)]
mod tests {
    // Capture setup; `workspace::tests::flow::authoring` opens each shot.
}
