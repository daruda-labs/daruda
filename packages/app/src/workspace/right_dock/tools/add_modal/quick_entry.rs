use super::*;
use crate::agent::mcp::{format_env_lines, import};

impl AddMcpServerModal {
    pub(super) fn review_paste(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match import::parse(&self.paste_input.read(cx).value()) {
            Ok(mut drafts) if drafts.len() == 1 => self.apply_import(drafts.remove(0), window, cx),
            Ok(drafts) => {
                self.candidates = drafts;
                self.error = None;
                cx.notify();
            }
            Err(_) => {
                self.error_field = None;
                self.error = Some(strings::mcp::import_error().into());
                window.focus(&self.paste_input.focus_handle(cx), cx);
                cx.notify();
            }
        }
    }

    pub(super) fn apply_import(
        &mut self,
        draft: McpServerDraft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for (input, value) in [
            (&self.name_input, draft.name.clone()),
            (
                &self.command_input,
                draft.command.clone().unwrap_or_default(),
            ),
            (
                &self.args_input,
                super::super::modal_shared::join_args(&draft.args),
            ),
            (&self.url_input, draft.url.clone().unwrap_or_default()),
            (&self.env_input, format_env_lines(&draft.env)),
            (&self.headers_input, format_env_lines(&draft.headers)),
        ] {
            input.update(cx, |input, cx| input.set_value(value, window, cx));
        }
        self.auth.seed(&draft.headers, window, cx);
        self.transport = draft.transport;
        self.disabled = draft.disabled;
        self.imported = Some(draft);
        self.pasting = false;
        self.candidates.clear();
        self.error = None;
        self.error_field = None;
        window.focus(&self.name_input.focus_handle(cx), cx);
        cx.notify();
    }

    pub(super) fn target_label(&self) -> String {
        let path = match self.scope {
            McpScope::Project => self
                .initial
                .project_root
                .as_ref()
                .map(|p| crate::agent::mcp::project_mcp_path(p).display().to_string())
                .unwrap_or_default(),
            McpScope::User => crate::agent::mcp::claude_json_path().display().to_string(),
            McpScope::Local => format!(
                "{} · projects[{:?}].mcpServers",
                crate::agent::mcp::claude_json_path().display(),
                self.initial
                    .project_root
                    .as_ref()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default()
            ),
        };
        strings::mcp::target_claude(path)
    }

    pub(super) fn render_paste(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(theme::FORM_MODAL_SECTION_GAP))
            .child(field_label(strings::mcp::quick_hint(), theme::current(cx)))
            .child(crate::ui::input(&self.paste_input, cx, 0));
        if !self.candidates.is_empty() {
            body = body.child(field_label(
                strings::mcp::choose_server(),
                theme::current(cx),
            ));
            for (ix, draft) in self.candidates.iter().enumerate() {
                let draft = draft.clone();
                body = body.child(
                    button(("import-server", ix), draft.name.clone())
                        .tab_stop(true)
                        .tab_index(ix as isize + 1)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.apply_import(draft.clone(), window, cx)
                        })),
                );
            }
        }
        if let Some(error) = &self.error {
            body = body.child(crate::ui::alert::error("import-error", error.clone()));
        }
        body.child(
            div()
                .flex()
                .justify_end()
                .gap(px(theme::MODAL_FOOTER_GAP))
                .child(
                    button("import-cancel", strings::common::btn_cancel())
                        .tab_stop(true)
                        .tab_index(100)
                        .on_click(cx.listener(|this, _, w, cx| this.leave(w, cx))),
                )
                .child(
                    button("import-manual", strings::common::btn_manual())
                        .tab_stop(true)
                        .tab_index(101)
                        .on_click(cx.listener(|this, _, w, cx| {
                            this.pasting = false;
                            this.error = None;
                            w.focus(&this.name_input.focus_handle(cx), cx);
                            cx.notify();
                        })),
                )
                .child(
                    button_primary("import-review", strings::common::btn_review())
                        .tab_stop(true)
                        .tab_index(102)
                        .disabled(self.paste_input.read(cx).value().trim().is_empty())
                        .on_click(cx.listener(|this, _, w, cx| this.review_paste(w, cx))),
                ),
        )
        .tab_group()
        .into_any_element()
    }
}
