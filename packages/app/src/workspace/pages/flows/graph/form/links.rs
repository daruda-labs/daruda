//! Node choices distinguish execution order from output-file references.

use super::super::FlowGraphView;
use crate::surface::strings as s;
use crate::ui::select::{SelectEvent, SelectOption};
use crate::ui::{InputEvent, RopeExt as _};
use daruda_flow::{
    NodeId,
    parse::{FlowFile, NodeKindFile},
};
use gpui::{Context, Window};
use std::collections::BTreeSet;

pub(super) fn candidates(
    file: &FlowFile,
    node: &NodeId,
    deps: &[String],
) -> (Vec<String>, Vec<String>) {
    let mut ancestors: BTreeSet<String> = deps.iter().cloned().collect();
    loop {
        let before = ancestors.len();
        for entry in &file.nodes {
            if ancestors.contains(entry.id.as_str()) {
                ancestors.extend(entry.deps.iter().map(ToString::to_string));
            }
        }
        if ancestors.len() == before {
            break;
        }
    }
    let mut descendants = BTreeSet::from([node.to_string()]);
    loop {
        let before = descendants.len();
        for entry in &file.nodes {
            if entry
                .deps
                .iter()
                .any(|dep| descendants.contains(dep.as_str()))
            {
                descendants.insert(entry.id.to_string());
            }
        }
        if descendants.len() == before {
            break;
        }
    }
    let available = file
        .nodes
        .iter()
        .filter(|n| {
            !descendants.contains(n.id.as_str()) && !deps.iter().any(|dep| dep == n.id.as_str())
        })
        .map(|n| n.id.to_string())
        .collect();
    let outputs = file
        .nodes
        .iter()
        .filter(|n| {
            n.id != *node
                && ancestors.contains(n.id.as_str())
                && matches!(n.kind, NodeKindFile::Agent { .. })
        })
        .map(|n| n.id.to_string())
        .collect();
    (available, outputs)
}

impl FlowGraphView {
    pub(in crate::workspace) fn refresh_form_links(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(form) = &self.form else {
            return;
        };
        let Some(mut file) = self
            .text
            .as_deref()
            .and_then(|text| daruda_flow::parse::parse_flow_file(text).ok())
        else {
            return;
        };
        // Draft dependency edges must participate in cycle prevention too.
        for (id, mut fields) in self.pending_node_fields(cx) {
            // Draft renames are swept atomically on Save; pickers still address
            // the canvas IDs while honoring draft kind and dependency changes.
            fields.id = id.to_string();
            super::apply::node_fields(&mut file, &id, &fields);
        }
        let (deps, outputs) = candidates(&file, &form.node, &form.fields(cx).deps);
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
            form.deps
                .update(cx, |input, cx| input.set_value(deps.join(", "), window, cx));
        }
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
            form.deps
                .update(cx, |input, cx| input.set_value(deps.join(", "), window, cx));
        }
        self.refresh_form_links(window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dependencies_exclude_cycles_and_results_require_agent_ancestors() {
        let file = daruda_flow::parse::parse_flow_file("version: 1\nnodes:\n  - id: a\n    kind: agent\n    prompt: hello\n    output: a.md\n  - id: b\n    deps: [a]\n    kind: command\n    run: echo ok\n  - id: c\n    deps: [b]\n    kind: agent\n    prompt: hello\n    output: c.md\n").unwrap();
        let (deps, outputs) = candidates(&file, &NodeId::from("b"), &["a".into()]);
        assert!(deps.is_empty());
        assert_eq!(outputs, ["a"]);
        let (_, outputs) = candidates(&file, &NodeId::from("c"), &["b".into()]);
        assert_eq!(outputs, ["a"]);
    }
}
