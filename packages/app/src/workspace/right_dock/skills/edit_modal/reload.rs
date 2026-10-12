use super::*;

impl EditSkillModal {
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
        let config = self.config.clone();
        *self = Self::new(
            self.workspace.clone(),
            self.reload_skill.clone(),
            self.state_snapshot.clone(),
            window,
            cx,
        );
        self.config = config;
        window.focus(&self.panel_focus_handle, cx);
        cx.notify();
    }
}
