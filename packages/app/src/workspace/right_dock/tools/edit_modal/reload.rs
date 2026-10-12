use super::*;

impl EditMcpServerModal {
    pub(super) fn request_reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.submitting {
            return;
        }
        let dirty = self.form.is_dirty(&self.values(cx));
        if !dirty {
            self.reload(window, cx);
            return;
        }
        super::super::super::form_state::ask_to_leave(
            &mut self.form,
            true,
            false,
            window,
            cx,
            |this, choice, window, cx| {
                this.form.asking = false;
                match choice {
                    super::super::super::form_state::LeaveChoice::Save => this.submit(window, cx),
                    super::super::super::form_state::LeaveChoice::Discard => {
                        this.reload(window, cx)
                    }
                    super::super::super::form_state::LeaveChoice::Continue => {}
                }
            },
        );
    }
    fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(config) = self.initial.config.clone() else {
            return;
        };
        let Some(target) = self.initial.target.clone() else {
            return;
        };
        let name = self.initial.name.clone();
        self.submitting = true;
        super::super::super::form_state::freeze_inputs(&self.inputs(), true, cx);
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let input = config.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut inventory = crate::agent::configuration::library::discover(&input)?;
                    let group = inventory
                        .groups
                        .into_iter()
                        .find(|g| {
                            g.target.path == target.path
                                && g.target.location == target.location
                                && g.target.owner.is_none()
                        })
                        .ok_or_else(strings::mcp::source_changed)?;
                    match target.scope {
                        McpScope::User => inventory.mcp.user = group.servers,
                        McpScope::Local => inventory.mcp.local = group.servers,
                        McpScope::Project => inventory.mcp.project = group.servers,
                    }
                    let mut initial = EditMcpInitial::from_state(
                        &inventory.mcp,
                        target.scope,
                        &name,
                        input.cwd.clone(),
                    )
                    .ok_or_else(strings::mcp::source_changed)?;
                    initial.config = Some(input);
                    initial.target = Some(group.target);
                    Ok::<_, String>(initial)
                })
                .await;
            // SILENT-OK: a closed modal no longer owns a draft to reload.
            let _ = this.update_in(cx, |this, window, cx| {
                this.submitting = false;
                super::super::super::form_state::freeze_inputs(&this.inputs(), false, cx);
                match result {
                    Ok(initial) => {
                        *this = Self::new(this.workspace.clone(), initial, window, cx);
                        window.focus(&this.focus_handle(cx), cx);
                    }
                    Err(error) => this.error = Some(error.into()),
                }
                cx.notify();
            });
        })
        .detach();
    }
}
