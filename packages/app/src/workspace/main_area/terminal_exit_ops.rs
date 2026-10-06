//! A shell that exits in a pane kept open (`close_pane_on_exit` off): the pane
//! says so, and its menu starts a fresh shell in the same place.

use gpui::{Context, Window};

use super::pane::{AccountDomain, PaneContent, resolve_pane_account};
use super::pane_tree::PaneId;
use crate::surface::strings;
use crate::workspace::Workspace;

impl Workspace {
    /// The shell in `pane_id` exited; mark the pane and write a notice under
    /// its last output.
    pub(in crate::workspace) fn note_terminal_exited(
        &mut self,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) {
        let Some(terminal) = self
            .main_area
            .runtimes
            .values_mut()
            .flat_map(|rt| rt.panes.iter_mut())
            .find(|p| p.id == pane_id)
            .and_then(|p| match &mut p.content {
                PaneContent::Terminal(t) => Some(t),
                _ => None,
            })
        else {
            return;
        };
        if terminal.exited {
            return;
        }
        terminal.exited = true;
        let notice = format!("\r\n{}\r\n", strings::terminal::process_exited());
        terminal.view.update(cx, |view, cx| {
            view.queue_output_bytes(notice.as_bytes(), cx);
            view.flush_pending_output(cx);
        });
        cx.notify();
    }

    /// Start a fresh shell in `pane_id`, whose shell exited, at the directory
    /// it last reported. The pane keeps its id, so its place in the split tree
    /// and its tab stay as they were.
    pub(in crate::workspace) fn restart_terminal_pane(
        &mut self,
        pane_id: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pane) = self.active_runtime().panes.iter().find(|p| p.id == pane_id) else {
            return;
        };
        let PaneContent::Terminal(terminal) = &pane.content else {
            return;
        };
        if !terminal.exited {
            return;
        }
        let cwd = terminal
            .cached_cwd
            .clone()
            .or_else(|| self.default_cwd_for_new_pane());
        let account = terminal.account;
        let prepared =
            resolve_pane_account(&self.accounts, &self.data_dir, account, AccountDomain::Any);
        let fresh =
            match self.spawn_terminal_pane(pane_id, cwd, account, prepared.as_ref(), window, cx) {
                Ok(fresh) => fresh,
                Err(err) => {
                    self.report_pane_error(&strings::ctx::restart_shell(), err, cx);
                    return;
                }
            };
        if let Some(pane) = self
            .active_runtime_mut()
            .panes
            .iter_mut()
            .find(|p| p.id == pane_id)
        {
            pane.content = fresh.content;
        }
        self.main_area.pending_resize = true;
        if self.active_runtime().focused_pane_id == pane_id {
            self.focus_pane(pane_id, window, cx);
        }
        cx.notify();
    }
}
