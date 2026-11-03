//! The bottom dock's shared input owned by [`crate::workspace::Workspace`]:
//! one `InputState` every input-capable pane types into, and the per-pane
//! drafts swapped through it on focus changes. Rendered by
//! [`super::terminal_input`].

use std::collections::HashMap;

use gpui::{AppContext, Context, Subscription, Window};

use crate::workspace::Workspace;
use crate::workspace::main_area::pane_tree::PaneId;

pub(in crate::workspace) struct InputDock {
    /// Multi-line text input bound to the built-in "Input" bottom panel.
    /// Sends typed text verbatim to the focused terminal pane on Cmd+Enter
    /// or the inline Submit button click. The send button and drop chrome
    /// live in [`super::terminal_input`] rather than wrapping the state in an
    /// `InputPanel` composite (the commit input still uses `InputPanel` for
    /// its dropdown + floating-bar layout).
    pub(in crate::workspace) input: gpui::Entity<crate::ui::TextareaState>,
    /// Keeps the input's subscription alive with the Workspace.
    _subscription: Subscription,
    /// Cached line count of `input`. Updated on every `InputEvent::Change`
    /// so `adapt_dock_to_input_lines` can guard against redundant resizes
    /// without re-reading the entity on every render cycle.
    pub(in crate::workspace) line_count: usize,
    /// The user expanded the composer past `[agent] input_max_rows`; an
    /// emptied draft collapses it again.
    pub(in crate::workspace) expanded: bool,
    /// When true, the bottom dock shows the built-in "Input" panel instead
    /// of the active macro tab.
    pub(in crate::workspace) visible: bool,
    /// Per-pane unsent draft text, keyed by the input-capable pane
    /// (Terminal / AgentChat) the text was typed for. Swapped in/out by
    /// `set_focused_pane` on every focus change: the outgoing pane's text is
    /// saved to `owner` and the incoming pane's draft is restored. Empty
    /// strings are never stored — `remove` is used instead to avoid leaking
    /// entries for empty drafts or closed panes.
    pub(in crate::workspace) drafts: HashMap<PaneId, String>,
    /// The input-capable pane whose draft is currently visible in `input` —
    /// the last-focused Terminal / AgentChat pane. Draft saves key off this
    /// rather than the outgoing `focused_pane_id`, so text typed while a
    /// non-input pane (File / TaskEdit) held focus still saves to the pane it
    /// was meant for on the next input-pane focus. `None` until the first
    /// input-capable pane is focused.
    pub(in crate::workspace) owner: Option<PaneId>,
    /// The slash-command list open over the composer, recomputed on every
    /// edit; `None` when the cursor is not on a `/`-token or nothing matches.
    pub(in crate::workspace) slash: Option<SlashMenu>,
}

/// The open slash-command list and its highlighted row.
pub(in crate::workspace) struct SlashMenu {
    pub(in crate::workspace) matched: super::slash_command::SlashMatch,
    pub(in crate::workspace) selected: usize,
}

impl InputDock {
    /// Rows the composer may grow to: the configured cap, or the setting's
    /// ceiling while the user has expanded it.
    pub(in crate::workspace) fn max_rows(&self, configured: u8) -> usize {
        if self.expanded {
            usize::from(daruda_config::agent::INPUT_MAX_ROWS_MAX)
        } else {
            usize::from(configured)
        }
    }

    pub(in crate::workspace) fn new(
        config: &daruda_config::Config,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> Self {
        let terminal_input = cx.new(|cx_state| {
            let max_rows = usize::from(config.agent.input_max_rows);
            let mut state = crate::ui::TextareaState::new(window, cx_state).auto_grow(1, max_rows);
            state.set_placeholder(
                crate::surface::strings::bottom_dock::input_placeholder(),
                window,
                cx_state,
            );
            state
        });
        // Submit, history, mode and stop keys, and the slash-command list, are
        // the composer's own: `composer_ops` intercepts the field's actions in
        // the capture phase (see `terminal_input::render_body`).
        // The bottom-dock body wires the Submit-button click →
        // `send_terminal_input`; this subscription covers the keyboard path.
        // `PressEnter { secondary: true }` is emitted on every *submit* —
        // Cmd+Enter always, plus a plain Enter when the `submit_on_enter`
        // predicate above is active (agent pane focused, modifier-to-send off).
        // Shift+Tab mode cycling is handled directly by the `on_secondary_tab`
        // handler installed above (no event round-trip needed).
        let terminal_input_sub = cx.subscribe_in(
            &terminal_input,
            window,
            |this, _, ev: &crate::ui::InputEvent, window, cx| match ev {
                crate::ui::InputEvent::PressEnter {
                    secondary: true, ..
                } => {
                    this.send_terminal_input(window, cx);
                }
                crate::ui::InputEvent::Change => {
                    this.refresh_slash_menu(cx);
                    this.collapse_composer_if_empty(window, cx);
                    this.adapt_dock_to_input_lines(window, cx);
                    // The composer no longer holds what the resume gesture was
                    // armed against, so the next Enter sends rather than confirms.
                    this.disarm_queue_resume(cx);
                }
                _ => {}
            },
        );
        Self {
            input: terminal_input,
            _subscription: terminal_input_sub,
            line_count: 1,
            expanded: false,
            visible: false,
            drafts: HashMap::new(),
            owner: None,
            slash: None,
        }
    }
}
