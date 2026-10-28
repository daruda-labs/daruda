//! The selected node's widget projection. Domain transitions live in inspector.
use super::super::inspector::{NodeDraft, RawInputs, UiHints};
use super::*;

impl FormBinding {
    pub(in crate::workspace::pages::flows::graph) fn bind(
        draft: &NodeDraft,
        issues: &[daruda_flow::error::ValidationIssue],
        window: &mut Window,
        cx: &mut Context<super::super::FlowGraphView>,
    ) -> Self {
        let mut binding = Self::build(draft, issues, window, cx);
        binding.write(draft.raw(), draft.ui(), window, cx);
        binding.set_notes(binding.notes.clone());
        binding
    }

    pub(in crate::workspace) fn read(&self, cx: &App) -> RawInputs {
        RawInputs {
            id: read(&self.id, cx),
            deps: read(&self.deps, cx),
            timeout: read(&self.timeout, cx),
            max_turns: read(&self.max_turns, cx),
            cwd: read(&self.cwd, cx),
            agent_id: read(&self.agent.id, cx),
            agent_model: read(&self.agent.model, cx),
            agent_mode: read(&self.agent.mode, cx),
            agent_effort: read(&self.agent.effort, cx),
            prompt_inline: read(&self.body.prompt.inline, cx),
            prompt_file: read(&self.body.prompt.file, cx),
            output: read(&self.body.output, cx),
            run: read(&self.body.run, cx),
            hint_inline: read(&self.body.on_fail.hint.inline, cx),
            hint_file: read(&self.body.on_fail.hint.file, cx),
            fix: read(&self.body.on_fail.fix, cx),
            rerun: read(&self.body.on_fail.rerun, cx),
            max_attempts: read(&self.body.on_fail.max_attempts, cx),
            wait: read(&self.body.on_fail.wait, cx),
            kind: selected_kind(&self.body.kind, cx),
            permission: selected_permission(&self.agent.permission, cx),
            prompt_is_file: is_file_source(&self.body.prompt, cx),
            hint_is_file: is_file_source(&self.body.on_fail.hint, cx),
            acting: acting(&self.body.on_fail, cx),
        }
    }

    pub(in crate::workspace::pages::flows::graph) fn hints(&self, cx: &App) -> UiHints {
        UiHints {
            agent_open: self.agent_open,
            settings_open: self.settings_open,
            agent_id_custom: self.agent_choices[0].read(cx).is_custom(cx),
            agent_model_custom: self.agent_choices[1].read(cx).is_custom(cx),
            agent_mode_custom: self.agent_choices[2].read(cx).is_custom(cx),
            timeout_custom: self.timeout_choice.read(cx).is_custom(cx),
            turns_custom: self.turns_choice.read(cx).is_custom(cx),
        }
    }

    /// Change notifications are drained before the applying guard is released.
    pub(in crate::workspace::pages::flows::graph) fn write(
        &mut self,
        raw: &RawInputs,
        hints: &UiHints,
        window: &mut Window,
        cx: &mut Context<super::super::FlowGraphView>,
    ) {
        self.applying.set(self.applying.get() + 1);
        let applying = self.applying.clone();
        if read(&self.id, cx) != raw.id {
            self.id
                .update(cx, |input, cx| input.set_value(raw.id.clone(), window, cx));
        }
        if read(&self.deps, cx) != raw.deps {
            self.deps.update(cx, |input, cx| {
                input.set_value(raw.deps.clone(), window, cx)
            });
        }
        if read(&self.timeout, cx) != raw.timeout {
            self.timeout.update(cx, |input, cx| {
                input.set_value(raw.timeout.clone(), window, cx)
            });
        }
        if read(&self.max_turns, cx) != raw.max_turns {
            self.max_turns.update(cx, |input, cx| {
                input.set_value(raw.max_turns.clone(), window, cx)
            });
        }
        if read(&self.cwd, cx) != raw.cwd {
            self.cwd
                .update(cx, |input, cx| input.set_value(raw.cwd.clone(), window, cx));
        }
        if read(&self.agent.id, cx) != raw.agent_id {
            self.agent.id.update(cx, |input, cx| {
                input.set_value(raw.agent_id.clone(), window, cx)
            });
        }
        if read(&self.agent.model, cx) != raw.agent_model {
            self.agent.model.update(cx, |input, cx| {
                input.set_value(raw.agent_model.clone(), window, cx)
            });
        }
        if read(&self.agent.mode, cx) != raw.agent_mode {
            self.agent.mode.update(cx, |input, cx| {
                input.set_value(raw.agent_mode.clone(), window, cx)
            });
        }
        if read(&self.agent.effort, cx) != raw.agent_effort {
            self.agent.effort.update(cx, |input, cx| {
                input.set_value(raw.agent_effort.clone(), window, cx)
            });
        }
        if read(&self.body.prompt.inline, cx) != raw.prompt_inline {
            self.body.prompt.inline.update(cx, |input, cx| {
                input.set_value(raw.prompt_inline.clone(), window, cx)
            });
        }
        if read(&self.body.prompt.file, cx) != raw.prompt_file {
            self.body.prompt.file.update(cx, |input, cx| {
                input.set_value(raw.prompt_file.clone(), window, cx)
            });
        }
        if read(&self.body.output, cx) != raw.output {
            self.body.output.update(cx, |input, cx| {
                input.set_value(raw.output.clone(), window, cx)
            });
        }
        if read(&self.body.run, cx) != raw.run {
            self.body
                .run
                .update(cx, |input, cx| input.set_value(raw.run.clone(), window, cx));
        }
        if read(&self.body.on_fail.hint.inline, cx) != raw.hint_inline {
            self.body.on_fail.hint.inline.update(cx, |input, cx| {
                input.set_value(raw.hint_inline.clone(), window, cx)
            });
        }
        if read(&self.body.on_fail.hint.file, cx) != raw.hint_file {
            self.body.on_fail.hint.file.update(cx, |input, cx| {
                input.set_value(raw.hint_file.clone(), window, cx)
            });
        }
        if read(&self.body.on_fail.fix, cx) != raw.fix {
            self.body
                .on_fail
                .fix
                .update(cx, |input, cx| input.set_value(raw.fix.clone(), window, cx));
        }
        if read(&self.body.on_fail.rerun, cx) != raw.rerun {
            self.body.on_fail.rerun.update(cx, |input, cx| {
                input.set_value(raw.rerun.clone(), window, cx)
            });
        }
        if read(&self.body.on_fail.max_attempts, cx) != raw.max_attempts {
            self.body.on_fail.max_attempts.update(cx, |input, cx| {
                input.set_value(raw.max_attempts.clone(), window, cx)
            });
        }
        if read(&self.body.on_fail.wait, cx) != raw.wait {
            self.body.on_fail.wait.update(cx, |input, cx| {
                input.set_value(raw.wait.clone(), window, cx)
            });
        }
        for (state, value) in [
            (
                &self.body.kind,
                if raw.kind == KindChoice::Agent {
                    KIND_AGENT
                } else {
                    KIND_COMMAND
                },
            ),
            (&self.agent.permission, permission_value(raw.permission)),
            (
                &self.body.prompt.choice,
                if raw.prompt_is_file {
                    SOURCE_FILE
                } else {
                    SOURCE_INLINE
                },
            ),
            (
                &self.body.on_fail.hint.choice,
                if raw.hint_is_file {
                    SOURCE_FILE
                } else {
                    SOURCE_INLINE
                },
            ),
            (
                &self.body.on_fail.policy,
                if raw.acting { FAIL_ACT } else { FAIL_HALT },
            ),
        ] {
            state.update(cx, |state, cx| {
                state.set_selected_value(&value.into(), window, cx)
            });
        }
        self.agent_open = hints.agent_open;
        self.settings_open = hints.settings_open;
        for (choice, custom) in [
            (&self.agent_choices[0], hints.agent_id_custom),
            (&self.agent_choices[1], hints.agent_model_custom),
            (&self.agent_choices[2], hints.agent_mode_custom),
            (&self.timeout_choice, hints.timeout_custom),
            (&self.turns_choice, hints.turns_custom),
        ] {
            choice.update(cx, |choice, cx| choice.set_custom(custom, window, cx));
        }
        cx.defer(move |_| applying.set(applying.get() - 1));
    }

    pub(in crate::workspace::pages::flows::graph) fn applying(
        &self,
    ) -> std::rc::Rc<std::cell::Cell<usize>> {
        self.applying.clone()
    }

    pub(in crate::workspace::pages::flows::graph) fn watch_inputs(
        &self,
    ) -> Vec<Entity<InputState>> {
        vec![
            self.id.clone(),
            self.deps.clone(),
            self.timeout.clone(),
            self.max_turns.clone(),
            self.cwd.clone(),
            self.agent.id.clone(),
            self.agent.model.clone(),
            self.agent.mode.clone(),
            self.agent.effort.clone(),
            self.body.prompt.inline.clone(),
            self.body.prompt.file.clone(),
            self.body.output.clone(),
            self.body.run.clone(),
            self.body.on_fail.hint.inline.clone(),
            self.body.on_fail.hint.file.clone(),
            self.body.on_fail.fix.clone(),
            self.body.on_fail.rerun.clone(),
            self.body.on_fail.max_attempts.clone(),
            self.body.on_fail.wait.clone(),
        ]
    }

    pub(in crate::workspace::pages::flows::graph) fn watch_selects(
        &self,
        cx: &App,
    ) -> Vec<Entity<crate::ui::select::SelectState>> {
        let mut selects = vec![
            self.body.kind.clone(),
            self.agent.permission.clone(),
            self.body.prompt.choice.clone(),
            self.body.on_fail.policy.clone(),
            self.body.on_fail.hint.choice.clone(),
        ];
        selects.extend(
            self.agent_choices
                .iter()
                .map(|choice| choice.read(cx).select.clone()),
        );
        selects.push(self.timeout_choice.read(cx).select.clone());
        selects.push(self.turns_choice.read(cx).select.clone());
        selects
    }
}

#[cfg(test)]
mod tests {}
