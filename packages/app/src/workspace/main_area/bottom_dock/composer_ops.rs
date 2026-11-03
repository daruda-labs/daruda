//! The composer's own keys and its slash-command list.
//!
//! The bottom-dock field is a plain multi-line `TextareaState`; what makes it
//! a composer lives here. `terminal_input::render_body` intercepts the field's
//! actions in the capture phase and dispatches to these ops, each of which
//! returns `true` when it handled the key, so the field never sees it, and
//! `false` to let the field's own handling run.

use gpui::{Context, Window};

use super::input_dock::SlashMenu;
use super::slash_command::{slash_match, with_builtin_clear};
use crate::ui::RopeExt as _;
use crate::workspace::Workspace;

/// Which way ↑/↓ walks the lane's input history.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::workspace) enum HistoryDir {
    Up,
    Down,
}

impl Workspace {
    /// Recompute the slash-command list from the composer's text and cursor.
    /// Only an Agent chat pane advertises commands.
    pub(in crate::workspace) fn refresh_slash_menu(&mut self, cx: &mut Context<Self>) {
        let focused = self.active_runtime().focused_pane_id;
        let matched = self
            .agent_chat_view(focused)
            .filter(|_| !self.focused_question_bound(cx))
            .and_then(|view| {
                let commands = with_builtin_clear(
                    view.read(cx).session_config().available_commands().to_vec(),
                );
                let input = self.input_dock.input.read(cx);
                slash_match(&input.value(), input.cursor(), &commands)
            });
        self.input_dock.slash = matched.map(|matched| SlashMenu {
            matched,
            selected: 0,
        });
        cx.notify();
    }

    /// Enter. An open list takes it as accept. Otherwise Cmd/Ctrl+Enter always
    /// submits, and a plain Enter submits while an Agent chat pane is focused
    /// and the user has not opted into modifier-to-send; Shift+Enter and every
    /// other Enter insert a newline.
    pub(in crate::workspace) fn composer_enter(
        &mut self,
        action: &crate::ui::InputEnter,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.input_dock.input.read(cx).has_marked_text() {
            return false;
        }
        if self.input_dock.slash.is_some() && !action.secondary && !action.shift {
            self.accept_slash_item(None, window, cx);
            return true;
        }
        let focused = self.active_runtime().focused_pane_id;
        let submits = action.secondary
            || (!action.shift
                && self.is_agent_chat_pane(focused)
                && !self.mirrors.agent.use_modifier_to_send);
        if submits {
            self.send_terminal_input(window, cx);
        }
        submits
    }

    /// Escape closes an open list first; otherwise it stops the focused agent
    /// pane's turn, the keyboard counterpart of the Stop button.
    pub(in crate::workspace) fn composer_escape(&mut self, cx: &mut Context<Self>) -> bool {
        if self.input_dock.slash.take().is_some() {
            cx.notify();
            return true;
        }
        if self.focused_question_bound(cx) {
            self.question_escape(cx);
            return true;
        }
        let focused = self.active_runtime().focused_pane_id;
        self.cancel_agent_turn_if_active(focused, cx)
    }

    /// ↑/↓ move through an open list; at the first / last shown row they walk
    /// the lane's input history (shell-style); anywhere else they move the
    /// cursor as usual.
    pub(in crate::workspace) fn composer_move(
        &mut self,
        dir: HistoryDir,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.input_dock.input.read(cx).has_marked_text() {
            return false;
        }
        if self.question_arrows_take(cx) {
            self.move_question_highlight(dir, window, cx);
            return true;
        }
        if let Some(menu) = &mut self.input_dock.slash {
            let len = menu.matched.items.len();
            menu.selected = match dir {
                HistoryDir::Up => (menu.selected + len - 1) % len,
                HistoryDir::Down => (menu.selected + 1) % len,
            };
            cx.notify();
            return true;
        }
        let (row, rows) = crate::ui::cursor_display_row(self.input_dock.input.read(cx));
        let at_edge = match dir {
            HistoryDir::Up => row == 0,
            HistoryDir::Down => row + 1 >= rows,
        };
        if !at_edge || !self.history_navigate_possible(dir, cx) {
            return false;
        }
        self.do_history_navigate(dir, window, cx);
        true
    }

    /// Tab accepts an open list's highlighted command.
    pub(in crate::workspace) fn composer_tab(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.input_dock.slash.is_none() {
            return false;
        }
        self.accept_slash_item(None, window, cx);
        true
    }

    /// Shift+Tab cycles the focused agent pane's session mode when it has
    /// more than one; otherwise the field outdents.
    pub(in crate::workspace) fn composer_shift_tab(&mut self, cx: &mut Context<Self>) -> bool {
        if self.focused_question_bound(cx) {
            return false;
        }
        let focused = self.active_runtime().focused_pane_id;
        self.cycle_agent_mode(focused, cx)
    }

    /// Write the command at `index` (the highlighted one when `None`) over the
    /// `/`-token, then let [`Self::complete_slash_command`] decide whether it
    /// sends at once.
    pub(in crate::workspace) fn accept_slash_item(
        &mut self,
        index: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(menu) = self.input_dock.slash.take() else {
            return;
        };
        let Some(item) = menu
            .matched
            .items
            .get(index.unwrap_or(menu.selected))
            .cloned()
        else {
            return;
        };
        let token = menu.matched.token;
        self.input_dock.input.update(cx, |state, cx_state| {
            let text = state.value();
            let Some((head, tail)) = text.get(..token.start).zip(text.get(token.end..)) else {
                return;
            };
            let next = format!("{head}{}{tail}", item.insert);
            let cursor = token.start + item.insert.len();
            state.set_value(next, window, cx_state);
            let position = state.text().offset_to_position(cursor);
            state.set_cursor_position(position, window, cx_state);
        });
        self.complete_slash_command(item.name, window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::HistoryDir;

    #[test]
    fn history_dirs_are_distinct() {
        assert_ne!(HistoryDir::Up, HistoryDir::Down);
    }
}
