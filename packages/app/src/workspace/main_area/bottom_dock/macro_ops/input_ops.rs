//! Bottom-dock input bar: focusing it, sending its text to the focused pane,
//! prompt-history navigation, and slash-command completion.

use crate::workspace::Workspace;
use crate::workspace::main_area::pane_input_ops::{PaneTextInput, PaneTextIntent};
use crate::workspace::main_area::pane_tree::PaneId;
use daruda_chat::view::PromptId;
use gpui::{App, Context, Window};

impl Workspace {
    /// Switch the bottom dock to the built-in "Input" panel.
    pub(in crate::workspace) fn activate_bottom_input(&mut self, cx: &mut Context<Self>) {
        if self.input_dock.visible {
            return;
        }
        self.input_dock.visible = true;
        self.sync_bottom_dock_min_to_panel(cx);
        cx.notify();
    }

    /// Send the current terminal input text to the focused pane.
    /// Appends `\r` so the shell receives the command as if Enter was
    /// pressed, then clears the input field.
    ///
    /// `gpui_component::Input`'s multi-line `enter` action inserts a
    /// `\n` into the textarea *before* it emits the `PressEnter`
    /// event (see `gpui_component/src/input/state.rs::enter`), so the
    /// raw `state.value()` always has at least one trailing newline
    /// when Cmd+Enter triggers this path. Trimming trailing `\r`/`\n`
    /// before the embedded-newline → `\r` conversion is what keeps
    /// the shell from receiving two (or three, after `ICRNL` echo)
    /// Enter keystrokes.
    pub(in crate::workspace) fn send_terminal_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let raw = self.input_dock.input.read(cx).value().to_string();
        let trimmed = raw.trim_end_matches(['\n', '\r']);
        if let Some(accepted) = self.send_attached_composer_if_any(trimmed.to_owned(), cx) {
            if accepted {
                if let Some(owner) = self.input_dock.owner {
                    self.input_dock.drafts.remove(&owner);
                }
                self.input_dock
                    .input
                    .update(cx, |state, cx| state.set_value("", window, cx));
            }
            return;
        }
        if trimmed.is_empty() {
            // An empty composer is not a command: through the funnel a focused
            // terminal would take the submit and receive a bare `\r`. Only an
            // agent pane reads it, as a gesture. No history entry and no
            // `set_value` — that emits `Change` even for identical text, which
            // would disarm the gesture this very Enter just armed.
            let focused = self.active_runtime().focused_pane_id;
            if self.is_agent_chat_pane(focused) {
                self.agent_empty_submit(focused, window, cx);
            }
            return;
        }
        // Push to per-lane history before routing so both agent prompts and
        // terminal commands land in the same unified buffer. `push` resets
        // the navigation cursor, so ↑ after submit starts from the entry
        // we just pushed.
        let lane_ref = self.active_ref();
        let history_text = trimmed.trim().to_owned();
        self.lane_scoped_mut(lane_ref)
            .input_history
            .push(&history_text);
        // Submitted text is no longer a draft — drop the saved entry for
        // the pane whose text is visible (`input_owner`) so returning to
        // it after send shows an empty input.
        if let Some(owner) = self.input_dock.owner {
            self.input_dock.drafts.remove(&owner);
        }
        // Route through the single pane-delivery funnel. It branches on the
        // focused pane's kind: a Terminal receives the bytes (embedded `\n` →
        // `\r`, trailing `\r` on submit — identical to the former inline
        // path), an Agent chat pane submits the text as an ACP turn. `trimmed`
        // already dropped trailing `\n`/`\r`, so the funnel adds exactly one
        // submitting `\r` for terminals.
        self.deliver_text_to_focused_pane(
            PaneTextInput {
                body: trimmed.to_string(),
                intent: PaneTextIntent::Command { submit: true },
            },
            window,
            cx,
        );
        self.input_dock
            .input
            .update(cx, |s, cx_state| s.set_value("", window, cx_state));
    }

    /// Returns `true` when history navigation in `dir` is possible for the
    /// currently active lane. Called from the `on_history_navigate` hook to
    /// decide whether to consume ↑/↓ before deferring `do_history_navigate`.
    /// Reading the lane's history is safe here (hook fires inside
    /// `terminal_input`'s update; `self` is `Workspace`, a different entity).
    pub(in crate::workspace) fn history_navigate_possible(
        &self,
        dir: crate::ui::HistoryDir,
        cx: &App,
    ) -> bool {
        // ↑ is also consumed to edit the most-recent queued prompt when the
        // focused pane is an agent chat with a non-empty queue — even if this
        // lane has no input history yet. The empty-composer gate is applied in
        // the deferred `do_history_navigate` (the composer can't be read here
        // without re-entering `terminal_input` mid-update, pitfall #5).
        if matches!(dir, crate::ui::HistoryDir::Up) && self.focused_agent_last_queued(cx).is_some()
        {
            return true;
        }
        let lane_ref = self.active_ref();
        let Some(state) = self.lane_scoped.get(&lane_ref) else {
            return false;
        };
        let buf = &state.input_history;
        match dir {
            crate::ui::HistoryDir::Up => buf.has_entries(),
            crate::ui::HistoryDir::Down => buf.is_navigating(),
        }
    }

    /// Resolve the focused Agent chat pane and the id of its most-recently
    /// queued prompt, when the focused pane is an agent chat with a non-empty
    /// queue. Backs the ↑-in-empty-composer edit affordance
    /// ([`Self::do_history_navigate`]) and its consume predicate
    /// ([`Self::history_navigate_possible`]). `None` for a terminal focus, an
    /// agent pane with an empty queue, or the Welcome state.
    ///
    /// Prefers the live queue's last entry; falls back to the parked queue's
    /// last entry so ↑ still edits the most-recent prompt after a Stop parked
    /// the queue (the strip renders parked rows as editable).
    fn focused_agent_last_queued(&self, cx: &App) -> Option<(PaneId, PromptId)> {
        let focused = self.active_runtime().focused_pane_id;
        let view = self.agent_chat_view(focused)?;
        let id = view.read(cx).queue().last_prompt_id()?;
        Some((focused, id))
    }

    /// Execute a history navigation step for the active lane. Reads the
    /// current `terminal_input` text, calls `prev` or `next` on the lane's
    /// `HistoryBuffer`, and calls `set_value` with the result. Called from a
    /// `window.defer` inside `on_history_navigate` so it runs after
    /// `terminal_input`'s current update has finished (re-entry guard).
    pub(in crate::workspace) fn do_history_navigate(
        &mut self,
        dir: crate::ui::HistoryDir,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let current = self.input_dock.input.read(cx).value().to_string();
        // ↑ in an EMPTY composer, with a non-empty queue on the focused agent
        // pane, pulls the most-recent queued prompt into the composer for
        // editing instead of recalling input history. A non-empty composer,
        // an empty queue, or a non-agent pane all fall through to the normal
        // per-lane history recall below.
        if matches!(dir, crate::ui::HistoryDir::Up)
            && current.is_empty()
            && let Some((pane_id, id)) = self.focused_agent_last_queued(cx)
        {
            self.begin_edit_queued_prompt(pane_id, id, window, cx);
            return;
        }
        let lane_ref = self.active_ref();
        let buf = &mut self.lane_scoped_mut(lane_ref).input_history;
        let new_text = match dir {
            crate::ui::HistoryDir::Up => buf.prev(&current).map(str::to_owned),
            crate::ui::HistoryDir::Down => buf.forward().map(str::to_owned),
        };
        let Some(new_text) = new_text else {
            return;
        };
        self.input_dock.input.update(cx, |s, cx_state| {
            s.set_value(&new_text, window, cx_state);
            // Move cursor to end so the user can append/edit immediately —
            // shell history convention (bash/zsh/fish). `set_value` on a
            // multi-line state clears the selection to 0..0 (cursor at start).
            s.move_cursor_to_end(cx_state);
        });
    }

    /// Finalize an accepted slash-command completion from the bottom-dock
    /// input. The menu has already inserted the command text; this decides
    /// whether to send immediately.
    ///
    /// `NoInput` commands take no argument, so the completed `/name` is sent
    /// right away by reusing [`Self::send_terminal_input`] (which reads the
    /// input, routes it to the focused agent pane's session, and clears the
    /// field — no duplication). `FreeText` commands leave `"/name "` in the
    /// input for the user to type the argument; Enter-to-send (already active
    /// for agent panes) submits it later.
    pub(in crate::workspace) fn complete_slash_command(
        &mut self,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Resolve the command kind to a plain bool first so the `&self`/`cx`
        // read borrow ends before the `&mut self` `send_terminal_input` call.
        let focused = self.active_runtime().focused_pane_id;
        let is_no_input = self
            .agent_chat_view(focused)
            .and_then(|v| {
                v.read(cx)
                    .session_config()
                    .slash_command(&name)
                    .map(|c| matches!(c.input, daruda_acp::SlashCommandInput::NoInput))
            })
            .unwrap_or(false);
        // No-op for FreeText commands (the inserted "/name " awaits the user's
        // argument) and for the rare case where the command is gone / focus changed
        // between menu build and accept — the "/name" text is already in the input,
        // so the user can still submit with Enter. Only NoInput auto-sends.
        if is_no_input {
            self.send_terminal_input(window, cx);
        }
    }
}
