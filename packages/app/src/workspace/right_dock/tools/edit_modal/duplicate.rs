use super::super::add_modal::{AddMcpInitial, AddMcpServerModal};
use super::*;

impl EditMcpServerModal {
    pub(super) fn duplicate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.submitting {
            return;
        }
        let mut draft = match self.build_draft(cx) {
            Ok(draft) => draft,
            Err((_, error)) => {
                self.error = Some(error);
                self.error_field = None;
                cx.notify();
                return;
            }
        };
        draft.name = format!("{}-copy", draft.name);
        draft.env.clear();
        draft.headers.clear();
        draft.extra.clear();
        let initial = AddMcpInitial {
            default_scope: self.initial.scope,
            project_root: self.initial.project_root.clone(),
            existing_names: vec![(self.initial.scope, self.initial.name.clone())],
        };
        let workspace = self.workspace.clone();
        crate::workspace::dialog_helpers::open_form_modal(
            strings::mcp::new_title(),
            Some(px(theme::FORM_MODAL_WIDE)),
            move |window, cx| {
                let mut modal = AddMcpServerModal::new(workspace, initial, window, cx);
                modal.seed_duplicate(draft, window, cx);
                modal
            },
            window,
            cx,
        );
    }
}
