use ghostty_vt::{KeyModeFlags, KeyModifiers, encode_key_named};
use gpui::{Context, EntityInputHandler, KeyDownEvent, Pixels, UTF16Selection, Window};
use std::ops::Range;

use super::TerminalView;

// ---------------------------------------------------------------------------
// IME helper functions
// ---------------------------------------------------------------------------

pub(crate) fn should_skip_key_down_for_ime(
    has_input: bool,
    has_marked_text: bool,
    keystroke: &gpui::Keystroke,
) -> bool {
    // IME composition in progress (key_char not yet committed)
    if keystroke.is_ime_in_progress() {
        if has_marked_text {
            return !matches!(
                keystroke.key.as_str(),
                "enter"
                    | "return"
                    | "kp_enter"
                    | "numpad_enter"
                    | "escape"
                    | "backspace"
                    | "delete"
            );
        }
        if has_input {
            return !matches!(
                keystroke.key.as_str(),
                "enter" | "return" | "kp_enter" | "numpad_enter"
            );
        }
        return false;
    }

    // key_char is set, but if it contains non-ASCII characters (e.g. Korean ㅎ
    // from the first keystroke after switching input sources), the IME pipeline
    // (replace_and_mark_text_in_range / replace_text_in_range) should handle it
    // instead of on_key_down. Without this, the first Korean character after
    // switching from English is lost because on_key_down cannot encode it.
    if let Some(key_char) = keystroke.key_char.as_deref()
        && !key_char.is_ascii()
        && !keystroke.modifiers.control
        && !keystroke.modifiers.platform
    {
        return true;
    }

    // Active marked text: skip printable keys so IME can continue
    // composition — *unless* a command modifier is held. Cmd+F /
    // Cmd+C / Cmd+T style shortcuts need to reach `on_key_down` so
    // the handler can flush the composer before letting the action
    // dispatch take over; if we bail early here the composer's
    // pending jamo survives into whatever context the shortcut
    // opens (search overlay, new tab, …).
    if has_marked_text
        && let Some(key_char) = keystroke.key_char.as_deref()
        && !key_char.is_empty()
        && !keystroke.modifiers.platform
        && !keystroke.modifiers.control
        && !matches!(
            keystroke.key.as_str(),
            "enter" | "return" | "escape" | "backspace" | "delete"
        )
    {
        return true;
    }

    false
}

pub(crate) fn ctrl_byte_for_keystroke(keystroke: &gpui::Keystroke) -> Option<u8> {
    let candidate = keystroke
        .key_char
        .as_deref()
        .or_else(|| (!keystroke.key.is_empty()).then_some(keystroke.key.as_str()))?;

    if candidate == "space" {
        return Some(0x00);
    }

    let bytes = candidate.as_bytes();
    if bytes.len() != 1 {
        return None;
    }

    let b = bytes[0];
    if (b'@'..=b'_').contains(&b) {
        Some(b & 0x1f)
    } else if b.is_ascii_lowercase() {
        Some(b - b'a' + 1)
    } else if b.is_ascii_uppercase() {
        Some(b - b'A' + 1)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// TerminalView — keyboard input
// ---------------------------------------------------------------------------

impl TerminalView {
    fn key_mode_flags(&self) -> KeyModeFlags {
        KeyModeFlags {
            cursor_key_application: self.session.decckm_enabled(),
            keypad_key_application: self.session.decnkm_enabled(),
        }
    }

    pub(super) fn on_tab(&mut self, _: &super::Tab, _window: &mut Window, cx: &mut Context<Self>) {
        self.send_tab(false, cx);
    }

    pub(super) fn on_tab_prev(
        &mut self,
        _: &super::TabPrev,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.send_tab(true, cx);
    }

    fn send_tab(&mut self, reverse: bool, cx: &mut Context<Self>) {
        if reverse {
            self.send_input_parts(&[crate::ansi::CBT], cx);
        } else {
            self.send_input_parts(&[b"\t"], cx);
        }
    }

    /// Settle the viewport after a relative scroll: drain the scroll-delta
    /// tracking, flush any pending session side-effects, then re-decide the
    /// lock and repaint. The shared tail of every relative-scroll key.
    fn settle_after_scroll(&mut self, cx: &mut Context<Self>) {
        self.sync_viewport_scroll_tracking();
        self.apply_side_effects(cx);
        self.reanchor_and_refresh(cx);
    }

    /// Handle a viewport navigation key (Home / End / PageUp / PageDown).
    /// Returns `true` when `key` was a navigation key and has been handled —
    /// the caller then stops propagation and returns. Shared by the shift
    /// and non-shift dispatch so the two can never drift out of sync.
    fn handle_scroll_key(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        let scroll_step = (self.session.rows() as i32 / 2).max(1);
        match key {
            "home" => {
                let _ = self.session.scroll_viewport_top();
                self.settle_after_scroll(cx);
            }
            "end" => {
                self.apply_side_effects(cx);
                self.snap_to_bottom_and_refresh(cx);
            }
            "pageup" | "page_up" | "page-up" => {
                let _ = self.session.scroll_viewport(-scroll_step);
                self.settle_after_scroll(cx);
            }
            "pagedown" | "page_down" | "page-down" => {
                // reanchor unlocks iff the page landed at the live edge
                // (scroll_offset clamped to 0) — no separate bottom-snap.
                let _ = self.session.scroll_viewport(scroll_step);
                self.settle_after_scroll(cx);
            }
            _ => return false,
        }
        true
    }

    /// Flush any pending Hangul, snap the viewport to the live bottom, send
    /// the key's raw bytes to the PTY, then stop event propagation. The
    /// shared body of every special-key → PTY branch in `on_key_down`.
    ///
    /// Deliberately leaner than [`Self::send_input_parts`]: it owns the
    /// `flush_hangul`-before-borrow ordering and the `stop_propagation` that
    /// protocol sends do not need, and it skips the `pending_refresh` /
    /// `notify` those add — a raw keystroke's repaint is driven by the PTY
    /// echo round-trip, not an immediate refresh.
    fn send_key_bytes(&mut self, parts: &[&[u8]], cx: &mut Context<Self>) {
        self.flush_hangul(cx);
        self.snap_to_bottom_on_pty_input();
        if let Some(input) = self.input.as_ref() {
            for bytes in parts {
                input.send(bytes);
            }
        }
        cx.stop_propagation();
    }

    pub(super) fn on_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let raw_keystroke = event.keystroke.clone();

        // Let IME handle: non-ASCII key_char, IME-in-progress, or active composition.
        // Do NOT stop propagation — event must reach [inputContext handleEvent:].
        if should_skip_key_down_for_ime(
            self.input.is_some(),
            self.state.marked_text.is_some(),
            &raw_keystroke,
        ) {
            return;
        }
        let keystroke = raw_keystroke.with_simulated_ime();

        // Natural Text Editing: remap Cmd/Opt + arrow/delete to the
        // equivalent readline bytes before the platform early-return below
        // swallows the Cmd shortcuts. Gated on an active PTY and the config
        // toggle. Runs ahead of the `alt` branch so Opt+Arrow (which has no
        // `key_char`) is caught here rather than encoded as a CSI sequence.
        if self.input.is_some()
            && !self.state.search_overlay
            && self.session.natural_text_editing()
            && let Some(bytes) = super::keybindings::natural_text_editing_bytes(
                &keystroke.key,
                keystroke.modifiers.shift,
                keystroke.modifiers.control,
                keystroke.modifiers.alt,
                keystroke.modifiers.platform,
            )
        {
            self.send_key_bytes(&[bytes], cx);
            return;
        }

        if keystroke.modifiers.platform || keystroke.modifiers.function {
            // Commit any pending Hangul composition before the global
            // action system (Cmd+A / Cmd+C / Cmd+F / Cmd+T …) routes
            // the shortcut. Without the flush, a half-composed
            // syllable lingers inside the composer and either gets
            // dropped on the next reset or surfaces in an unrelated
            // buffer (the search query, a freshly-opened tab, …).
            self.flush_hangul(cx);
            return;
        }

        // Escape while an IME preedit OR a pending Hangul jamo is
        // alive cancels the in-progress composition without forwarding
        // `\x1b` to the PTY. macOS expects this for any IME-aware text
        // target: ESC during composition discards the partial input,
        // ESC outside composition reaches the shell. Forwarding `\x1b`
        // here on top of the preedit hands the downstream TUI (Claude
        // Code etc.) a malformed `ESC + UTF-8 lead byte` sequence on
        // the very next keystroke, which Ink/blessed-style key parsers
        // render as `<ffffffff>`.
        //
        // Both signals must trip the cancel: macOS sometimes commits
        // a jamo via `insertText:` (composer holds it, `marked_text`
        // stays empty) and other times marks it via `setMarkedText:`
        // (composer stays empty, `marked_text` holds it). Either path
        // is "composition in flight" from the user's perspective.
        if keystroke.key == "escape"
            && (self.state.marked_text.is_some() || self.state.hangul_composer.is_composing())
        {
            self.state.hangul_composer.reset();
            self.clear_marked_text(cx);
            cx.stop_propagation();
            return;
        }

        if self.input.is_some() {
            // Only handle special keys / escape sequences here.
            // Printable characters MUST go through IME pipeline
            // (replace_text_in_range) so Korean/CJK composition works.
            // Every PTY-send branch routes through `send_key_bytes`, which
            // flushes pending Hangul before borrowing `self.input` and stops
            // propagation so the key does not also reach the IME.

            // Shift + Home/End/PageUp/PageDown still scrolls the viewport —
            // intercept before the generic encoder turns it into a PTY
            // escape sequence. Same handler as the unshifted keys below.
            if keystroke.modifiers.shift && self.handle_scroll_key(keystroke.key.as_str(), cx) {
                cx.stop_propagation();
                return;
            }

            if keystroke.modifiers.control
                && let Some(b) = ctrl_byte_for_keystroke(&keystroke)
            {
                self.send_key_bytes(&[&[b]], cx);
                return;
            }

            if keystroke.modifiers.alt
                && let Some(text) = keystroke.key_char.as_deref()
            {
                self.send_key_bytes(&[&[0x1b], text.as_bytes()], cx);
                return;
            }

            let modifiers = KeyModifiers {
                shift: keystroke.modifiers.shift,
                control: keystroke.modifiers.control,
                alt: keystroke.modifiers.alt,
                super_key: false,
            };
            if let Some(encoded) =
                encode_key_named(&keystroke.key, modifiers, self.key_mode_flags())
            {
                self.send_key_bytes(&[&encoded], cx);
                return;
            }

            // No match — let IME handle via replace_text_in_range.
            // Do NOT stop propagation so the event reaches
            // [inputContext handleEvent:] → insertText/setMarkedText.
            return;
        }

        if self.handle_scroll_key(keystroke.key.as_str(), cx) {
            cx.stop_propagation();
            return;
        }

        let modifiers = KeyModifiers {
            shift: keystroke.modifiers.shift,
            control: keystroke.modifiers.control,
            alt: keystroke.modifiers.alt,
            super_key: false,
        };
        if let Some(encoded) = encode_key_named(&keystroke.key, modifiers, self.key_mode_flags()) {
            self.flush_hangul(cx);
            let _ = self.session.feed(&encoded);
            self.apply_side_effects(cx);
            self.schedule_viewport_refresh(cx);
            cx.stop_propagation();
            return;
        }

        if keystroke.key == "backspace" {
            self.snap_to_bottom_on_pty_input();
            if let Some(input) = self.input.as_ref() {
                input.send(&[0x7f]);
                cx.stop_propagation();
                return;
            }
            let _ = self.session.feed(&[0x08]);
            self.apply_side_effects(cx);
            self.schedule_viewport_refresh(cx);
            cx.stop_propagation();
        }

        // Unhandled key — let IME handle it
    }
}

// ---------------------------------------------------------------------------
// EntityInputHandler — macOS IME integration
// ---------------------------------------------------------------------------

impl EntityInputHandler for TerminalView {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let text = self.state.marked_text.as_ref()?.as_str();
        let total_utf16 = Self::utf16_len(text);
        let start = range_utf16.start.min(total_utf16);
        let end = range_utf16.end.min(total_utf16);
        let range_utf16 = start..end;
        *adjusted_range = Some(range_utf16.clone());

        let range_utf8 = Self::utf16_range_to_utf8(text, range_utf16)?;
        Some(text.get(range_utf8)?.to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.state.marked_selected_range_utf16.clone(),
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        let text = self.state.marked_text.as_ref()?.as_str();
        let len = Self::utf16_len(text);
        (len > 0).then_some(0..len)
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        // macOS calls `unmarkText:` whenever the preedit ends without
        // a corresponding `insertText:` — input-source switch (Ctrl+
        // Space), candidate-window dismiss, focus loss inside the IME
        // overlay, etc. Drop the composer's pending jamo so it does
        // not leak into the *next* keystroke (typing English `a`
        // after switching out of Korean would otherwise stamp `ㅎa`
        // into the PTY because the composer would still flush its
        // held Cho on the first non-jamo input).
        self.state.hangul_composer.reset();
        self.clear_marked_text(cx);
    }

    fn replace_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.clear_marked_text(cx);

        // Search overlay: the PTY target here is the background shell,
        // but Cmd+F keystrokes belong to the search query instead.
        // Drop any in-progress composition (the partial syllable has
        // no meaning against the query) and route every commit through
        // `commit_text` which redirects to `on_search_append`.
        if self.state.search_overlay {
            self.state.hangul_composer.reset();
            self.commit_text(text, cx);
            return;
        }

        // Feed each committed char through the composer. For macOS
        // precomposed syllables or ASCII, the composer forwards them
        // verbatim and any in-flight jamo first. For Compatibility
        // Jamo commits (the Mach Port init-delay edge case), the
        // composer accumulates them internally and only releases a
        // wide precomposed syllable when the composition is complete.
        // Nothing — not even the previous cursor-block DEL rewrite —
        // reaches the PTY until the composer decides the syllable is
        // done, which is exactly the invariant Claude Code's narrow-
        // width renderer relies on.
        for ch in text.chars() {
            for out in self.state.hangul_composer.feed(ch) {
                self.commit_text(&out, cx);
            }
        }
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // While the search overlay is up the IME preedit must NOT
        // bleed into the terminal: the preedit normally renders at
        // the shell cursor position, which here belongs to a
        // background shell, not to the search input. Drop the
        // marked-text state so nothing draws under the cursor; the
        // committed text still arrives via `replace_text_in_range`
        // and is routed into the search query by `commit_text`.
        if self.state.search_overlay {
            self.clear_marked_text(cx);
            return;
        }
        // Empty marked text == cancel signal. macOS uses
        // `setMarkedText:@""` (instead of `unmarkText:`) when an
        // input-source switch (Ctrl+Space) ends the composition
        // without a corresponding insertText:, plus other implicit
        // cancellation paths. Drop the composer's pending jamo
        // here — flushing would commit the user's abandoned
        // composition into the PTY, which is the wrong call when
        // the user explicitly switched away from the IME.
        if new_text.is_empty() {
            self.state.hangul_composer.reset();
            self.clear_marked_text(cx);
            return;
        }
        // Selectively flush the composer based on what the incoming
        // preedit text is and what state the composer is in.
        //
        // Flush only when ALL of the following hold:
        //   1. The incoming text is NOT a jungseong (vowel) jamo.
        //   2. The composer is in the bare Cho state (lone consonant,
        //      no vowel yet).
        //
        // Why condition 1: after an input-source switch macOS
        // dispatches a syllable as separate jamo — insertText("ㅎ")
        // commits the choseong into the composer, then
        // setMarkedText("ㅏ") signals the vowel is coming. The
        // composer must hold the Cho so the following insertText("ㅏ")
        // can combine them into "하". Flushing on a vowel preedit
        // would produce PTY bytes "ㅎㅏ" (two narrow cells).
        //
        // Why condition 2: after the vowel the composer may be in
        // ChoJung(ㅎ,ㅏ) state and a following setMarkedText("ㄴ")
        // means "ㄴ is the jongseong of 한". Flushing ChoJung early
        // would emit "하" and leave ㄴ as a stray consonant ("하ㄴ"
        // instead of "한"). Only a bare Cho — a lone consonant with
        // no vowel attached — can never be extended by the incoming
        // consonant, so that is the only case worth flushing eagerly.
        let incoming_is_vowel = new_text
            .chars()
            .next()
            .is_some_and(super::jamo::is_compat_jungseong);

        if incoming_is_vowel {
            // If the current preedit is a lone choseong that macOS
            // never committed via insertText: (preedit was replaced
            // directly by this vowel preedit), feed it into the
            // composer now so the vowel can attach to it.
            // Scenario: setMarkedText("ㅎ") → setMarkedText("ㅏ")
            // with no insertText("ㅎ") in between. Without this,
            // composer stays Empty and the vowel emits as a standalone
            // jamo ("ㅏ" instead of combining into "하").
            if let Some(prev) = self.state.marked_text.as_ref() {
                let prev_str = prev.as_str();
                let mut prev_chars = prev_str.chars();
                if let (Some(ch), None) = (prev_chars.next(), prev_chars.next())
                    && super::jamo::is_compat_choseong(ch)
                    && !self.state.hangul_composer.is_cho_only()
                {
                    for out in self.state.hangul_composer.feed(ch) {
                        self.commit_text(&out, cx);
                    }
                }
            }
        } else if self.state.hangul_composer.is_cho_only() {
            // Consonant preedit arriving while composer holds a bare
            // Cho: flush the Cho immediately so it appears on screen.
            self.flush_hangul(cx);
        }

        // Determine the display text for the preedit overlay.
        //
        // In the post-switch mode macOS delivers each jamo of a
        // syllable as a separate insertText: call. The composer
        // accumulates them internally. When setMarkedText: arrives
        // for the next jamo, the composer already holds the preceding
        // partial syllable: we peek at the combined result and show
        // the precomposed syllable in the overlay (e.g. "한" instead
        // of just "ㄴ"). If the jamo cannot extend the current state
        // (a new choseong arriving after a complete LVT), flush the
        // composer first so the completed syllable reaches the PTY.
        let display_text = {
            let mut chars = new_text.chars();
            if let (Some(ch), None) = (chars.next(), chars.next())
                && super::jamo::is_hangul_compat_jamo(ch)
                && self.state.hangul_composer.is_composing()
            {
                if let Some(combined) = self.state.hangul_composer.peek_with(ch) {
                    combined
                } else {
                    self.flush_hangul(cx);
                    new_text.to_string()
                }
            } else {
                new_text.to_string()
            }
        };
        self.set_marked_text(display_text, new_selected_range, cx);
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        element_bounds: gpui::Bounds<Pixels>,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<gpui::Bounds<Pixels>> {
        // Refresh the last-known cursor when the session can report
        // one, otherwise fall back to whatever we cached previously
        // — and as a last resort, anchor at (1, 1) so the macOS
        // candidate window has a defined home rather than jumping
        // to the screen corner. This matters in alt-screen mode
        // where some TUIs hide the cursor while still expecting IME
        // composition (Claude Code is the practical case).
        let (col, row) = if let Some(pos) = self.session.cursor_position() {
            self.state.last_known_cursor = Some(pos);
            pos
        } else {
            self.state.last_known_cursor.unwrap_or((1, 1))
        };
        // `cell_dimensions` honours `self.state.font_size` / spacing. The
        // bare `cell_metrics` would read `window.text_style()`, which
        // during IME bounds callbacks is GPUI's root 1rem (16px) —
        // the candidate window would float above the wrong caret.
        let layout = self.cell_layout(window)?;
        let (cell_width, cell_height) = (layout.cell_width, layout.line_height);

        // Use the shaped row's actual glyph advances when available —
        // cell-width arithmetic drifts on rows containing wide (CJK /
        // emoji) glyphs before the cursor and would put the IME
        // candidate window at the wrong spot.
        let base_x = super::text_metrics::cell_left_x_for_col(
            &self.line_layouts,
            &self.state.viewport_lines,
            col,
            row,
            cell_width,
            element_bounds.left(),
        );
        let base_y = element_bounds.top() + gpui::px(cell_height * (row.saturating_sub(1)) as f32);

        // Resolve the preedit caret's pixel offset through the shaper
        // when the preedit contains wide (CJK / emoji) glyphs.
        // `element::prepaint` drops `force_width` in that case, so
        // post-wide glyphs sit at natural advances — a cell-width
        // multiplication would drift further with each wide glyph.
        // Pure-narrow preedits still get the monospaced fast path.
        use unicode_width::UnicodeWidthChar as _;
        // Use `self.state.font_size` — the inherited `window.text_style()`
        // is GPUI's root 1rem here (event callback, no style stack),
        // which would shape the preedit at the wrong size and land
        // the IME candidate window above the wrong caret.
        let font_size = gpui::px(self.state.font_size);
        let offset_px = self
            .state
            .marked_text
            .as_ref()
            .map(|text| {
                let s = text.as_str();
                let has_wide = s.chars().any(|ch| ch.width().unwrap_or(0) > 1);
                if has_wide {
                    let byte = Self::utf16_range_to_utf8(s, 0..range_utf16.start)
                        .map(|r| r.end)
                        .unwrap_or(0);
                    super::text_metrics::x_for_byte_in_text(
                        window,
                        &self.state.font,
                        font_size,
                        s,
                        byte,
                    )
                    .unwrap_or_else(|| {
                        let cells = Self::cell_offset_for_utf16(s, range_utf16.start);
                        gpui::px(cell_width * cells as f32)
                    })
                } else {
                    let cells = Self::cell_offset_for_utf16(s, range_utf16.start);
                    gpui::px(cell_width * cells as f32)
                }
            })
            .unwrap_or_else(|| gpui::px(cell_width * range_utf16.start as f32));
        let x = base_x + offset_px;
        Some(gpui::Bounds::new(
            gpui::point(x, base_y),
            gpui::size(gpui::px(cell_width), gpui::px(cell_height)),
        ))
    }

    fn character_index_for_point(
        &mut self,
        _point: gpui::Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}
