//! The selected node's widget projection. Domain transitions live in inspector.
//!
//! One direction each way: [`FormBinding::write`] shows the model's values,
//! and [`FormBinding::read`] hands what the person typed back to it. Both walk
//! the same named table, so a text field cannot be read but never shown.
use super::super::inspector::{FormChrome, NodeDraft, RawInputs, UiHints};
use super::*;

impl FormBinding {
    pub(in crate::workspace::pages::flows::graph) fn bind(
        draft: &NodeDraft,
        chrome: FormChrome,
        window: &mut Window,
        cx: &mut Context<super::super::FlowGraphView>,
    ) -> Self {
        let mut binding = Self::build(draft, window, cx);
        binding.write(draft.raw(), draft.ui(), window, cx);
        binding.project_chrome(chrome);
        binding
    }

    /// Every text box beside the raw field it holds. The exhaustive
    /// destructure makes a new `RawInputs` field a compile error here rather
    /// than a box that is silently never shown or read.
    fn text_slots<'a>(
        &'a self,
        raw: &'a mut RawInputs,
    ) -> [(&'a Entity<InputState>, &'a mut String); 19] {
        let RawInputs {
            id,
            deps,
            timeout,
            max_turns,
            cwd,
            agent_id,
            agent_model,
            agent_mode,
            agent_effort,
            prompt_inline,
            prompt_file,
            output,
            run,
            hint_inline,
            hint_file,
            fix,
            rerun,
            max_attempts,
            wait,
            kind: _,
            prompt_is_file: _,
            hint_is_file: _,
            acting: _,
            permission: _,
        } = raw;
        let fail = &self.body.on_fail;
        [
            (&self.id, id),
            (&self.deps, deps),
            (&self.timeout, timeout),
            (&self.max_turns, max_turns),
            (&self.cwd, cwd),
            (&self.agent.id, agent_id),
            (&self.agent.model, agent_model),
            (&self.agent.mode, agent_mode),
            (&self.agent.effort, agent_effort),
            (&self.body.prompt.inline, prompt_inline),
            (&self.body.prompt.file, prompt_file),
            (&self.body.output, output),
            (&self.body.run, run),
            (&fail.hint.inline, hint_inline),
            (&fail.hint.file, hint_file),
            (&fail.fix, fix),
            (&fail.rerun, rerun),
            (&fail.max_attempts, max_attempts),
            (&fail.wait, wait),
        ]
    }

    pub(in crate::workspace) fn read(&self, cx: &App) -> RawInputs {
        let mut raw = RawInputs {
            kind: selected_kind(&self.body.kind, cx),
            permission: selected_permission(&self.agent.permission, cx),
            prompt_is_file: is_file_source(&self.body.prompt, cx),
            hint_is_file: is_file_source(&self.body.on_fail.hint, cx),
            acting: acting(&self.body.on_fail, cx),
            ..RawInputs::default()
        };
        for (input, value) in self.text_slots(&mut raw) {
            *value = read(input, cx);
        }
        raw
    }

    /// The person's own choices that live in widgets: custom-entry mode. The
    /// open sections are the model's, so they come from `current` unchanged.
    pub(in crate::workspace::pages::flows::graph) fn hints(
        &self,
        current: &UiHints,
        cx: &App,
    ) -> UiHints {
        UiHints {
            agent_open: current.agent_open,
            settings_open: current.settings_open,
            agent_id_custom: self.agent_choices[0].read(cx).is_custom(cx),
            agent_model_custom: self.agent_choices[1].read(cx).is_custom(cx),
            agent_mode_custom: self.agent_choices[2].read(cx).is_custom(cx),
            timeout_custom: self.timeout_choice.read(cx).is_custom(cx),
            turns_custom: self.turns_choice.read(cx).is_custom(cx),
        }
    }

    /// Show `raw` and `hints`. Only boxes that differ are written; the Change
    /// events that follow read back exactly the model's values, which the
    /// view's sync recognizes as no edit, so no guard is needed.
    pub(in crate::workspace::pages::flows::graph) fn write(
        &mut self,
        raw: &RawInputs,
        hints: &UiHints,
        window: &mut Window,
        cx: &mut Context<super::super::FlowGraphView>,
    ) {
        let mut target = raw.clone();
        for (input, value) in self.text_slots(&mut target) {
            if read(input, cx) != *value {
                let value = value.clone();
                input.update(cx, |input, cx| input.set_value(value, window, cx));
            }
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
        for (choice, custom) in [
            (&self.agent_choices[0], hints.agent_id_custom),
            (&self.agent_choices[1], hints.agent_model_custom),
            (&self.agent_choices[2], hints.agent_mode_custom),
            (&self.timeout_choice, hints.timeout_custom),
            (&self.turns_choice, hints.turns_custom),
        ] {
            choice.update(cx, |choice, cx| choice.set_custom(custom, window, cx));
        }
    }

    pub(in crate::workspace::pages::flows::graph) fn watch_inputs(
        &self,
    ) -> Vec<Entity<InputState>> {
        let mut raw = RawInputs::default();
        self.text_slots(&mut raw)
            .into_iter()
            .map(|(input, _)| input.clone())
            .collect()
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
mod tests {
    // Widget round-trips need a window; `apply_inspector_effects` asserts in
    // every workspace flow test that a bound form reads back the model's raw.
}
