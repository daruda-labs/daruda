use super::super::create_modal::CreateSkillModal;
use super::*;

impl EditSkillModal {
    pub(super) fn duplicate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.submitting || !self.body_loaded {
            return;
        }
        let draft = self.build_draft(cx);
        let Some(original) = self.original_source.as_ref() else {
            return;
        };
        let source = match crate::agent::skills::frontmatter::render_edited_skill_md(
            original,
            &self.original_frontmatter,
            &draft.frontmatter,
            &draft.body,
        ) {
            Ok(source) => source,
            Err(error) => {
                self.error = Some(error.to_string().into());
                cx.notify();
                return;
            }
        };
        let config = self.config.clone();
        let creation_targets = self.creation_targets.clone();
        let state = self.state_snapshot.clone();
        let workspace = self.workspace.clone();
        let scope = self.scope;
        let name = self.name.clone();
        crate::workspace::dialog_helpers::open_form_modal(
            strings::skills::duplicate_instructions(),
            Some(px(theme::FORM_MODAL_WIDE)),
            move |window, cx| {
                let mut modal = CreateSkillModal::new(
                    workspace,
                    Some(scope),
                    state.project_root.clone(),
                    state,
                    window,
                    cx,
                );
                modal.config = config;
                modal.creation_targets = creation_targets;
                modal.seed_duplicate(source, name, window, cx);
                modal
            },
            window,
            cx,
        );
    }
}
