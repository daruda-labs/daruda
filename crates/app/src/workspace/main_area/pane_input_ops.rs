//! The single funnel for delivering user text to a pane, branching on
//! pane type in ONE place. Macro buttons, Skills, and the unified
//! bottom-dock input all route through
//! [`Workspace::deliver_text_to_focused_pane`] so "place text in a pane"
//! is not reimplemented per call site.
//!
//! Per-type logic lives outside the dispatch: terminal byte conversion in
//! the pure [`to_terminal_bytes`] helper, agent submit in the existing
//! `send_agent_prompt_text` shim. The funnel only matches and delegates.

use gpui::{Context, Window};

use super::pane::PaneContent;
use super::pane_tree::PaneId;
use crate::workspace::Workspace;

/// A request to deliver text to a pane.
///
/// Invariant driven by `submit`:
/// - `submit = false` — "place the text where the user would type it, but
///   don't execute it". Terminal: type the characters as-is. AgentChat:
///   insert into the shared bottom-dock input at the cursor (preserving any
///   existing draft) without running a turn.
/// - `submit = true` — "execute". Terminal: append a trailing CR so the
///   shell runs the line. AgentChat: send the text as an ACP turn.
pub(in crate::workspace) struct PaneTextInput {
    pub body: String,
    pub submit: bool,
}

/// Convert a [`PaneTextInput`] into the raw bytes a terminal PTY expects.
///
/// Embedded `\n` are normalized to `\r` (terminal line-discipline
/// convention — mirrors `Workspace::send_terminal_input`, so each line is
/// treated as a separate command). When `submit` is set the payload ends
/// with exactly one trailing `\r` so the shell runs the final line as if
/// Enter was pressed; a body that already ends in `\r` is left alone so we
/// never send a double Enter.
fn to_terminal_bytes(input: &PaneTextInput) -> Vec<u8> {
    let mut payload: String = input
        .body
        .chars()
        .map(|c| if c == '\n' { '\r' } else { c })
        .collect();
    if input.submit && !payload.ends_with('\r') {
        payload.push('\r');
    }
    payload.into_bytes()
}

impl Workspace {
    /// Deliver `input` to the pane identified by `pane_id`, branching on
    /// the pane's content kind. Returns `true` when the text was accepted,
    /// `false` when the pane is gone or its kind cannot receive text.
    ///
    /// The single place "deliver text to a pane" is decided; the per-type
    /// work is delegated (terminal bytes via [`to_terminal_bytes`], agent
    /// submit via `send_agent_prompt_text`).
    pub(in crate::workspace) fn deliver_text_to_pane(
        &mut self,
        pane_id: PaneId,
        input: PaneTextInput,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(pane) = self.active_runtime().panes.iter().find(|p| p.id == pane_id) else {
            return false;
        };
        match &pane.content {
            PaneContent::Terminal(_) => {
                let Some(view) = pane.terminal_view().cloned() else {
                    return false;
                };
                // `view` is now owned, so the immutable `pane` borrow can end
                // before the `&mut self` calls below.
                let bytes = to_terminal_bytes(&input);
                view.update(cx, |v, _| v.send_input(&bytes));
                self.bump_activity(pane_id);
                cx.notify();
                true
            }
            PaneContent::AgentChat(_) => {
                if input.submit {
                    // Trim at the single dispatch point: an empty / whitespace-
                    // only submit (e.g. an "Enter-only" macro — empty `send` +
                    // `auto_enter`) must NOT fire a blank ACP turn on the focused
                    // agent chat. An accepted no-op returns `true` (the funnel
                    // handled it) without touching the session. A terminal, by
                    // contrast, may legitimately receive whitespace, so that arm
                    // is left untrimmed.
                    let trimmed = input.body.trim();
                    if trimmed.is_empty() {
                        // A whitespace-only submit while a queued prompt is being
                        // edited would strand the "Editing…" strip row (the flag
                        // stays set against an empty body). Cancel the edit so the
                        // row reverts and the composer clears.
                        let editing = self
                            .agent_chat_view(pane_id)
                            .is_some_and(|v| v.read(cx).queue.editing_prompt.is_some());
                        if editing {
                            self.cancel_edit_queued_prompt(pane_id, window, cx);
                        }
                        return true;
                    }
                    self.send_agent_prompt_text(pane_id, trimmed.to_string(), cx);
                    self.bump_activity(pane_id);
                    true
                } else {
                    // Non-submit: place the text in the shared bottom-dock
                    // input (where an agent-chat pane routes typing) without
                    // running a turn, so the user can review/edit then press
                    // Enter — the agent-pane analog of typing at a shell
                    // prompt.
                    self.insert_into_focused_input(&input.body, window, cx);
                    self.bump_activity(pane_id);
                    true
                }
            }
            PaneContent::File(_) | PaneContent::TaskEditPane(_) => false,
        }
    }

    /// Deliver `input` to the currently focused pane. Thin wrapper over
    /// [`Self::deliver_text_to_pane`].
    pub(in crate::workspace) fn deliver_text_to_focused_pane(
        &mut self,
        input: PaneTextInput,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let id = self.active_runtime().focused_pane_id;
        self.deliver_text_to_pane(id, input, window, cx)
    }

    /// Insert `text` into the shared bottom-dock input at the cursor without
    /// submitting. The AgentChat non-submit delivery path routes here: a
    /// macro with `auto_enter = false` places its text where the user types,
    /// preserving any existing draft (insert-at-cursor, not clobber). Empty
    /// `text` is a no-op. The `InputState::insert` update notifies for us.
    fn insert_into_focused_input(
        &mut self,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if text.is_empty() {
            return;
        }
        self.terminal_input
            .update(cx, |state, cx_state| state.insert(text, window, cx_state));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn submit_appends_single_cr() {
        let bytes = to_terminal_bytes(&PaneTextInput {
            body: "claude".to_string(),
            submit: true,
        });
        assert_eq!(bytes, b"claude\r");
    }

    #[test]
    fn submit_does_not_double_trailing_cr() {
        let bytes = to_terminal_bytes(&PaneTextInput {
            body: "claude\r".to_string(),
            submit: true,
        });
        assert_eq!(bytes, b"claude\r");
    }

    #[test]
    fn no_submit_sends_body_verbatim() {
        let bytes = to_terminal_bytes(&PaneTextInput {
            body: "git checkout ".to_string(),
            submit: false,
        });
        assert_eq!(bytes, b"git checkout ");
    }

    #[test]
    fn interior_newline_becomes_cr() {
        let bytes = to_terminal_bytes(&PaneTextInput {
            body: "a\nb".to_string(),
            submit: false,
        });
        assert_eq!(bytes, b"a\rb");
    }

    #[test]
    fn submit_with_interior_newline_normalizes_and_appends_cr() {
        let bytes = to_terminal_bytes(&PaneTextInput {
            body: "line1\nline2".to_string(),
            submit: true,
        });
        assert_eq!(bytes, b"line1\rline2\r");
    }
}
