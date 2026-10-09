use super::*;

impl SettingsView {
    pub(in crate::settings) fn subscribe_agent_input(
        state: &Entity<InputState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Subscription {
        cx.subscribe_in(
            state,
            window,
            |this, _, ev: &InputEvent, _window, cx| match ev {
                InputEvent::PressEnter { .. } | InputEvent::Blur => {
                    this.persist_agent_catalog(cx);
                }
                InputEvent::Change => {
                    if this.error.is_some() {
                        this.error = None;
                        cx.notify();
                    }
                }
                InputEvent::Focus => {}
            },
        )
    }

    /// [`Self::subscribe_agent_input`] for a row's multi-line field. Plain
    /// Enter inserts a newline and *still* emits `PressEnter` (see
    /// `gpui_component::input::state`'s `enter`), so submitting on it would
    /// rewrite `config.toml` on every line break and pop the validation
    /// banner on a line the user has not finished. Only the secondary form
    /// (Cmd/Ctrl+Enter) submits — the same split the MCP env / headers boxes
    /// make.
    pub(in crate::settings) fn subscribe_agent_multi_input(
        state: &Entity<InputState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Subscription {
        cx.subscribe_in(
            state,
            window,
            |this, _, ev: &InputEvent, _window, cx| match ev {
                InputEvent::PressEnter { secondary: true } | InputEvent::Blur => {
                    this.persist_agent_catalog(cx);
                }
                InputEvent::Change => {
                    if this.error.is_some() {
                        this.error = None;
                        cx.notify();
                    }
                }
                InputEvent::PressEnter { secondary: false } | InputEvent::Focus => {}
            },
        )
    }

    /// Wire one agent-catalog row's inputs to the standard submit /
    /// clear-error subscription plus the transport-pick repaint, pushing
    /// each into `subs`. An associated fn (not `&mut self`) so both the
    /// constructor's initial rows (loaded from config) and rows added at
    /// runtime via [`Self::add_agent_row`] go through one wiring site — two
    /// copies would let a row's inputs drift out of sync over which ones are
    /// actually subscribed.
    pub(in crate::settings) fn subscribe_agent_row(
        row: &AgentCatalogRow,
        window: &mut Window,
        cx: &mut Context<Self>,
        subs: &mut Vec<Subscription>,
    ) {
        subs.push(Self::subscribe_agent_input(&row.id_input, window, cx));
        subs.push(Self::subscribe_agent_input(&row.name_input, window, cx));
        subs.push(Self::subscribe_agent_input(&row.command_input, window, cx));
        subs.push(Self::subscribe_agent_multi_input(
            &row.env_input,
            window,
            cx,
        ));
        subs.push(Self::subscribe_agent_input(&row.host_input, window, cx));
        subs.push(Self::subscribe_agent_input(
            &row.container_input,
            window,
            cx,
        ));
        // Every picker persists on pick, like the transport one below. The
        // mode/model option lists are rebuilt in place by the id/command
        // handlers further down, which reuse these same entities so this
        // wiring stays valid.
        for state in [&row.default_mode_select, &row.default_model_select] {
            subs.push(cx.subscribe_in(
                state,
                window,
                |this, _state, ev: &select::ConfirmEvent, _window, cx| {
                    if matches!(ev, select::SelectEvent::Confirm(_)) {
                        this.persist_agent_catalog(cx);
                    }
                },
            ));
        }
        // Either tail picker may shed a "Custom (from config)" entry by
        // choosing one of the stated values. Rebuild the catalog from the
        // committed config after a successful save so the hidden preserved
        // value cannot be picked again later in the same Settings window.
        for picker in [&row.tail_window_select, &row.tail_window_calls_select] {
            subs.push(cx.subscribe_in(
                picker,
                window,
                |this, _state, ev: &select::ConfirmEvent, window, cx| {
                    if matches!(ev, select::SelectEvent::Confirm(_))
                        && this.persist_agent_catalog(cx)
                    {
                        this.reload_agent_catalog_from_live(window, cx);
                    }
                },
            ));
        }
        // The row's id keys the cached vocabulary, so retyping it switches
        // both pickers to that agent's option lists.
        subs.push(cx.subscribe_in(
            &row.id_input,
            window,
            |this, state, ev: &InputEvent, window, cx| {
                if matches!(ev, InputEvent::Change)
                    && let Some(index) = this.agent_row_index_by_id(state)
                {
                    this.refresh_agent_row_vocabulary(index, window, cx);
                }
            },
        ));
        // Re-render on transport pick so the row immediately shows/hides the
        // matching host/container field (rows are added/removed at runtime,
        // unlike the fixed global dropdowns wired in `new_with_section`), and
        // so an ssh/docker pick immediately hides a stale PATH warning — that
        // suppression is transport-dependent but needs no fresh `which` call
        // (see `agent.rs::render_agent_catalog_row`), so a plain repaint
        // suffices here.
        subs.push(cx.subscribe_in(
            &row.transport_select,
            window,
            |this, _state, ev: &select::ConfirmEvent, _window, cx| {
                if matches!(ev, select::SelectEvent::Confirm(_)) {
                    this.persist_agent_catalog(cx);
                }
            },
        ));
        // Recompute the local-PATH warning whenever the command text changes,
        // and re-source the pickers: the command names the adapter whose seed
        // fills them before any connect recorded a real vocabulary. Separate
        // from the standard submit/clear-error subscription above, which is
        // shared by every input field and doesn't know which row changed.
        subs.push(cx.subscribe_in(
            &row.command_input,
            window,
            |this, state, ev: &InputEvent, window, cx| {
                if matches!(ev, InputEvent::Change)
                    && let Some(index) = this.agent_row_index_by_command(state)
                {
                    this.recompute_agent_row_path_warning(index, cx);
                    this.refresh_agent_row_vocabulary(index, window, cx);
                }
            },
        ));
    }
}
