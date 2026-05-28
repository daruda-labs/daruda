use gpui::{Context, IntoElement, KeyContext, MouseButton, Render, Window, div, prelude::*};

use super::{
    KEY_CONTEXT, SEARCH_KEY_CONTEXT, TerminalView, element::TerminalTextElement,
    ensure_key_bindings, state::PendingRefresh,
};

impl TerminalView {
    /// Lazily register focus-in / blur subscriptions for DECSET 1004
    /// reporting. Called once from `render()`; the subscriptions live
    /// as long as the view.
    pub(super) fn ensure_focus_subscriptions(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self._focus_in_sub.is_some() {
            return;
        }

        self._focus_in_sub =
            Some(
                cx.on_focus_in(&self.focus_handle, window, |this, _window, cx| {
                    if this.session.focus_event_enabled() {
                        this.send_protocol_parts(&[crate::ansi::FOCUS_IN], cx);
                    }
                }),
            );

        self._focus_out_sub = Some(cx.on_blur(&self.focus_handle, window, |this, _window, cx| {
            // Commit any in-flight Hangul before focus moves away. macOS IME
            // never delivers a final `insertText:` after focus is lost, so a
            // partial syllable would be silently dropped when switching panes.
            this.flush_hangul(cx);
            if this.session.focus_event_enabled() {
                this.send_protocol_parts(&[crate::ansi::FOCUS_OUT], cx);
            }
        }));

        // Reset the Hangul composer when the user switches the macOS input
        // source (default `⌃Space`). macOS swallows that shortcut at the
        // system level — no IME callback fires — so without this hook the
        // composer's pending jamo leaks into the next keystroke.
        let weak = cx.entity().downgrade();
        self._keyboard_layout_sub = Some(cx.on_keyboard_layout_change(move |cx| {
            if let Some(view) = weak.upgrade() {
                view.update(cx, |this, cx| {
                    this.state.hangul_composer.reset();
                    this.clear_marked_text(cx);
                });
            }
        }));
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        ensure_key_bindings(cx);
        self.ensure_focus_subscriptions(window, cx);

        if !self.state.pending_output.is_empty() {
            let bytes = std::mem::take(&mut self.state.pending_output);
            self.feed_output_bytes_to_session(&bytes);
            self.check_prompt_arrived();
            self.maybe_scroll_to_bottom_on_output();
            self.apply_side_effects(cx);
            self.reconcile_dirty_viewport_after_output();
        }

        match self.state.pending_refresh {
            PendingRefresh::Preserve => self.refresh_viewport_preserving_selection(),
            PendingRefresh::Clear => self.refresh_viewport(),
            PendingRefresh::No => {}
        }
        self.state.pending_refresh = PendingRefresh::No;

        if self.session.window_title_updates_enabled() {
            let title = self
                .session
                .title()
                .unwrap_or(crate::ux::strings::FALLBACK_TITLE);

            if self.state.last_window_title.as_deref() != Some(title) {
                self.state.last_window_title = Some(title.to_string());
            }
        }

        // Bell detection — drain the pending flag even when disabled so it
        // doesn't accumulate.
        if self.session.take_bell() && self.session.visual_bell_enabled() {
            let bell = crate::ux::strings::BELL_FLASH;
            self.state.flash.bell = Some(std::time::Instant::now() + bell);
            cx.notify();
            let entity = cx.entity().downgrade();
            window
                .spawn(cx, async move |cx| {
                    cx.background_executor().timer(bell).await;
                    cx.update(|_, cx| {
                        if let Some(view) = entity.upgrade() {
                            view.update(cx, |_, cx| cx.notify());
                        }
                    })
                    .ok();
                })
                .detach();
        }

        let mut key_context = KeyContext::default();
        key_context.add(KEY_CONTEXT);
        if self.state.search_overlay {
            key_context.add(SEARCH_KEY_CONTEXT);
        }

        let font_size_px = gpui::px(self.state.font_size);
        let base_line_height =
            crate::view::text_metrics::cell_metrics_at(window, &self.state.font, font_size_px)
                .map(|(_, h)| h)
                .unwrap_or(self.state.font_size * 1.25);
        let line_height_px = gpui::px(base_line_height * self.state.vertical_spacing);

        div()
            .size_full()
            .flex()
            .text_size(font_size_px)
            .line_height(line_height_px)
            .track_focus(&self.focus_handle)
            .key_context(key_context)
            .on_action(cx.listener(Self::on_copy))
            .on_action(cx.listener(Self::on_select_all))
            .on_action(cx.listener(Self::on_paste))
            .on_action(cx.listener(Self::on_tab))
            .on_action(cx.listener(Self::on_tab_prev))
            .on_action(cx.listener(Self::on_zoom_in))
            .on_action(cx.listener(Self::on_zoom_out))
            .on_action(cx.listener(Self::on_reset_zoom))
            .on_action(cx.listener(Self::on_toggle_fullscreen))
            .on_action(cx.listener(Self::on_clear_buffer))
            .on_action(cx.listener(Self::on_clear_scrollback))
            .on_action(cx.listener(Self::on_scroll_to_bottom))
            .on_action(cx.listener(Self::on_search_open))
            .on_action(cx.listener(Self::on_search_close))
            .on_action(cx.listener(Self::on_search_next))
            .on_action(cx.listener(Self::on_search_prev))
            .on_action(cx.listener(Self::on_search_backspace))
            .on_action(cx.listener(Self::on_prompt_jump_prev))
            .on_action(cx.listener(Self::on_prompt_jump_next))
            .on_action(cx.listener(Self::on_command_jump_prev))
            .on_action(cx.listener(Self::on_command_jump_next))
            .on_action(cx.listener(Self::on_copy_last_command_output))
            .on_action(cx.listener(Self::on_search_cursor_left))
            .on_action(cx.listener(Self::on_search_cursor_right))
            .on_action(cx.listener(Self::on_search_cursor_home))
            .on_action(cx.listener(Self::on_search_cursor_end))
            .on_action(cx.listener(Self::on_search_delete_forward))
            .on_action(cx.listener(Self::on_search_clear_query))
            .on_action(cx.listener(Self::on_search_toggle_regex))
            .on_key_down(cx.listener(Self::on_key_down))
            .on_scroll_wheel(cx.listener(Self::on_scroll_wheel))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_down(MouseButton::Middle, cx.listener(Self::on_mouse_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up(MouseButton::Middle, cx.listener(Self::on_mouse_up))
            .on_mouse_up(MouseButton::Right, cx.listener(Self::on_mouse_up))
            .text_color(crate::ux::theme::TERMINAL_FG)
            .font(self.state.font.clone())
            .cursor_text()
            .whitespace_nowrap()
            .child(TerminalTextElement { view: cx.entity() })
            .when(self.state.search_overlay, |el| {
                el.child(self.render_search_bar(cx))
            })
    }
}
