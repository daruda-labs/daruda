//! The ask-first gate in front of closing a pane that is still running
//! something — a job in a terminal, or an agent turn. Every close path runs
//! through it before its unsaved-edits prompt: pane, tab, window and quit.

use gpui::{App, Context, SharedString, Window};

use crate::surface::strings;
use crate::workspace::Workspace;
use crate::workspace::main_area::pane_tree::PaneId;

impl Workspace {
    /// Titles of the panes among `pane_ids` still running something, found in
    /// any lane.
    pub(in crate::workspace) fn running_pane_titles(
        &self,
        pane_ids: &[PaneId],
        cx: &App,
    ) -> Vec<SharedString> {
        self.main_area
            .runtimes
            .values()
            .flat_map(|rt| rt.panes.iter())
            .filter(|p| pane_ids.contains(&p.id) && p.runs_work(cx))
            .map(|p| p.title(cx))
            .collect()
    }

    /// Every pane in every lane, for the window-close gate.
    pub(in crate::workspace) fn all_pane_ids(&self) -> Vec<PaneId> {
        self.main_area
            .runtimes
            .values()
            .flat_map(|rt| rt.panes.iter().map(|p| p.id))
            .collect()
    }

    /// Run `then` now when none of `pane_ids` runs anything; otherwise ask,
    /// and run it only once the user confirms.
    pub(in crate::workspace) fn confirm_stopping_then(
        &mut self,
        pane_ids: &[PaneId],
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let running = self.running_pane_titles(pane_ids, cx);
        if running.is_empty() {
            then(self, window, cx);
            return;
        }
        let receiver = prompt_stop_running(&running, window, cx);
        cx.spawn_in(window, async move |this, cx| {
            let Ok(0) = receiver.await else {
                return;
            };
            // SILENT-OK: the window may close before the answer arrives
            let _ = this.update_in(cx, |this, window, cx| then(this, window, cx));
        })
        .detach();
    }
}

/// Put the stop-running question up. Answer `0` confirms.
pub(in crate::workspace) fn prompt_stop_running(
    running: &[SharedString],
    window: &mut Window,
    cx: &mut App,
) -> futures::channel::oneshot::Receiver<usize> {
    let titles: Vec<&str> = running.iter().map(|t| t.as_ref()).collect();
    let heading = strings::close_running_heading();
    let detail = strings::close_running_detail(&titles);
    let confirm = strings::close_running_confirm();
    let cancel = strings::task_edit_cancel();
    window.prompt(
        gpui::PromptLevel::Warning,
        &heading,
        Some(&detail),
        &[confirm.as_str(), cancel.as_str()],
        cx,
    )
}
