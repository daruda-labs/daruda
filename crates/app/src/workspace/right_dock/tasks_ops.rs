//! Workspace-side ops for the right-dock Tasks tab. Currently just the
//! search-input clear handler — extracted so the View closure can
//! dispatch in one line.

use gpui::{Context, Window};

use crate::workspace::Workspace;

impl Workspace {
    pub(in crate::workspace) fn clear_task_search(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = self.task_search_input.clone();
        input.update(cx, |inp, cx_state| {
            inp.set_value("".to_string(), window, cx_state);
        });
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    // Behavioral coverage requires a live `Window` and
    // `Context<Workspace>` plus a stub Input entity; the underlying
    // `InputState::set_value` is exercised by `gpui_component` tests.
}
