//! The bottom dock's shared input owned by [`crate::workspace::Workspace`]:
//! one `InputState` every input-capable pane types into, and the per-pane
//! drafts swapped through it on focus changes. Rendered by
//! [`super::terminal_input`].

use std::collections::HashMap;

use gpui::{AppContext, Context, Subscription, WeakEntity, Window};

use crate::workspace::Workspace;
use crate::workspace::main_area::pane_tree::PaneId;

pub(in crate::workspace) struct InputDock {
    /// Multi-line text input bound to the built-in "Input" bottom panel.
    /// Sends typed text verbatim to the focused terminal pane on Cmd+Enter
    /// or the inline Submit button click. The send button and drop chrome
    /// live in [`super::terminal_input`] rather than wrapping the state in an
    /// `InputPanel` composite (the commit input still uses `InputPanel` for
    /// its dropdown + floating-bar layout).
    pub(in crate::workspace) input: gpui::Entity<crate::ui::InputState>,
    /// Keeps the input's subscription alive with the Workspace.
    _subscription: Subscription,
    /// Cached line count of `input`. Updated on every `InputEvent::Change`
    /// so `adapt_dock_to_input_lines` can guard against redundant resizes
    /// without re-reading the entity on every render cycle.
    pub(in crate::workspace) line_count: usize,
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
}

impl InputDock {
    pub(in crate::workspace) fn new(
        workspace: &WeakEntity<Workspace>,
        config: &daruda_config::Config,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> Self {
        let ws_for_input = workspace.clone();
        let ws_for_tab = workspace.clone();
        let ws_for_escape = workspace.clone();
        let ws_for_accept = workspace.clone();
        let ws_for_provider = workspace.clone();
        let ws_for_history = workspace.clone();
        let terminal_input = cx.new(|cx_state| {
            let max_rows = usize::from(config.agent.input_max_rows);
            let mut state = crate::ui::InputState::new(window, cx_state)
                .auto_grow(1, max_rows)
                // While an agent chat pane is focused and the user hasn't opted
                // into modifier-to-send, plain Enter submits and Shift+Enter
                // inserts a newline (Zed's agent-panel default). Evaluated live
                // on each Enter so it tracks focus + config without any sync.
                .submit_on_enter(move |app| {
                    ws_for_input.upgrade().is_some_and(|ws| {
                        let ws = ws.read(app);
                        ws.is_agent_chat_pane(ws.active_runtime().focused_pane_id)
                            && !ws.mirrors.agent.use_modifier_to_send
                    })
                })
                // Shift+Tab cycles the focused agent pane's session mode (Claude
                // Code's permission-mode cycle). `cycle_agent_mode` returns true
                // only when it actually switched (agent pane focused with ≥2
                // modes); the input then skips outdent. Otherwise it falls
                // through to the default outdent. Evaluated live on each press.
                .on_secondary_tab(move |_window, app| {
                    ws_for_tab.upgrade().is_some_and(|ws| {
                        let focused = ws.read(app).active_runtime().focused_pane_id;
                        ws.update(app, |ws, cx| ws.cycle_agent_mode(focused, cx))
                    })
                })
                // Escape cancels the focused agent pane's activity — the
                // keyboard counterpart of the bottom-dock "Stop" button. Returns
                // true only when the pane was actually busy (so Escape keeps
                // propagating normally otherwise). Fires only as a fallback after
                // the input's own Escape handling (see `InputState::on_escape`),
                // so an open completion menu still closes on the first Escape.
                .on_escape(move |_window, app| {
                    ws_for_escape.upgrade().is_some_and(|ws| {
                        let focused = ws.read(app).active_runtime().focused_pane_id;
                        ws.update(app, |ws, cx| ws.cancel_agent_turn_if_active(focused, cx))
                    })
                })
                // ↑/↓ at the boundary of the multi-line input navigates the
                // per-lane history buffer (shell-style). The hook checks whether
                // navigation is possible (entries exist / cursor is active), then
                // defers the actual `set_value` call. Deferring is the same re-
                // entry guard as `on_completion_accept`: the hook fires inside
                // `InputState`'s update, so we cannot call `terminal_input.update`
                // or `terminal_input.read` synchronously (CLAUDE.md pitfall #5).
                // Reading the workspace's lane history is fine — Workspace is a
                // different entity from terminal_input.
                .on_history_navigate(move |dir, window, app| {
                    let Some(ws) = ws_for_history.upgrade() else {
                        return false;
                    };
                    // Decide whether to consume before mutating any state.
                    // Reading ws (Workspace) is safe here: we're inside
                    // terminal_input's update, but Workspace is a different entity.
                    if !ws.read(app).history_navigate_possible(dir, app) {
                        return false;
                    }
                    let ws_deferred = ws.downgrade();
                    window.defer(app, move |window, cx| {
                        // SILENT-OK: workspace dropped between key press and defer
                        if let Some(ws) = ws_deferred.upgrade() {
                            ws.update(cx, |ws, cx| ws.do_history_navigate(dir, window, cx));
                        }
                    });
                    true
                })
                // Accepting a slash-command completion routes through the
                // workspace. The accept hook fires inside this `InputState`'s
                // update; `cx.defer_in` would re-lease this same InputState, so
                // `complete_slash_command` -> `send_terminal_input` (which
                // reads/clears `terminal_input`) would re-enter and panic. Defer
                // at window/app level — no entity re-lease (CLAUDE.md pitfall #5).
                .on_completion_accept(move |item, window, cx| {
                    let name = item.label.clone();
                    let ws = ws_for_accept.clone();
                    window.defer(cx, move |window, cx| {
                        if let Some(ws) = ws.upgrade() {
                            ws.update(cx, |ws, cx| ws.complete_slash_command(name, window, cx));
                        }
                    });
                });
            state.set_placeholder(
                crate::surface::strings::bottom_dock::input_placeholder(),
                window,
                cx_state,
            );
            // Feed the native completion menu with ACP slash commands when a
            // chat pane is focused and a `/`-token is being typed.
            state.lsp.completion_provider = Some(std::rc::Rc::new(
                crate::workspace::main_area::bottom_dock::slash_command::SlashCommandProvider {
                    workspace: ws_for_provider,
                },
            ));
            state
        });
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
                crate::ui::InputEvent::PressEnter { secondary: true } => {
                    this.send_terminal_input(window, cx);
                }
                crate::ui::InputEvent::Change => {
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
            visible: false,
            drafts: HashMap::new(),
            owner: None,
        }
    }
}
