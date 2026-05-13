use gpui::Context;

use super::TerminalView;

impl TerminalView {
    pub fn feed_output_bytes(&mut self, bytes: &[u8], cx: &mut Context<Self>) {
        self.feed_output_bytes_to_session(bytes);
        self.check_prompt_arrived();
        self.maybe_scroll_to_bottom_on_output();
        self.refresh_viewport();
        // Consume dirty rows that ghostty accumulated during the feed —
        // refresh_viewport already rebuilt the full viewport, so these
        // would otherwise be double-applied by the next
        // reconcile_dirty_viewport_after_output call.
        let _ = self.session.take_dirty_viewport_rows();
        self.sync_viewport_scroll_tracking();
        self.apply_side_effects(cx);
        cx.notify();
    }

    pub fn queue_output_bytes(&mut self, bytes: &[u8], cx: &mut Context<Self>) {
        const MAX_PENDING_OUTPUT_BYTES: usize = 256 * 1024;

        if self.state.pending_output.len().saturating_add(bytes.len()) <= MAX_PENDING_OUTPUT_BYTES {
            self.state.pending_output.extend_from_slice(bytes);
            cx.notify();
            return;
        }

        if !self.state.pending_output.is_empty() {
            let pending = std::mem::take(&mut self.state.pending_output);
            self.feed_output_bytes_to_session(&pending);
            self.check_prompt_arrived();
            self.maybe_scroll_to_bottom_on_output();
            self.apply_side_effects(cx);
            self.reconcile_dirty_viewport_after_output();
        }

        if bytes.len() > MAX_PENDING_OUTPUT_BYTES {
            let mut offset = 0usize;
            while offset < bytes.len() {
                let end = (offset + MAX_PENDING_OUTPUT_BYTES).min(bytes.len());
                self.feed_output_bytes_to_session(&bytes[offset..end]);
                offset = end;
            }
            self.check_prompt_arrived();
            self.maybe_scroll_to_bottom_on_output();
            self.apply_side_effects(cx);
            self.reconcile_dirty_viewport_after_output();
            cx.notify();
            return;
        }

        self.state.pending_output.extend_from_slice(bytes);
        cx.notify();
    }

    /// Drain bytes accumulated by `queue_output_bytes` immediately, so callers
    /// that read state derived from terminal sequences see fresh values without
    /// waiting for the next `render()`.
    pub fn flush_pending_output(&mut self, cx: &mut Context<Self>) {
        if self.state.pending_output.is_empty() {
            return;
        }
        let bytes = std::mem::take(&mut self.state.pending_output);
        self.feed_output_bytes_to_session(&bytes);
        self.check_prompt_arrived();
        self.maybe_scroll_to_bottom_on_output();
        self.apply_side_effects(cx);
        self.reconcile_dirty_viewport_after_output();
    }

    pub fn resize_terminal(&mut self, cols: u16, rows: u16, cx: &mut Context<Self>) {
        let _ = self.session.resize(cols, rows);
        self.sync_viewport_scroll_tracking();
        // Do not immediately refresh the viewport here. ghostty_vt reflows
        // the alt-screen on resize, which garbles TUI content before the TUI
        // has a chance to respond to SIGWINCH. Instead keep the pre-resize
        // viewport_lines visible until output arrives.
        cx.notify();
    }

    /// Apply updated terminal colors to the running session. Flushes the
    /// viewport style-run cache so every cell renders with the new palette.
    pub fn apply_colors(
        &mut self,
        fg: ghostty_vt::Rgb,
        bg: ghostty_vt::Rgb,
        palette: &[[u8; 3]; 16],
    ) {
        self.session.apply_colors(fg, bg, palette);
        self.refresh_viewport_preserving_selection();
        let _ = self.session.take_dirty_viewport_rows();
    }

    /// Returns whether `merge_bg_runs_transparent` would produce quads that
    /// cover every column (1..=cols) for the given viewport row.
    ///
    /// Used by visual tests to verify that transparent mode never leaves
    /// trailing empty columns without a background quad.
    #[cfg(any(test, feature = "test-support"))]
    pub fn transparent_bg_covers_row(&self, row: usize) -> bool {
        let total_cols = self.session.cols();
        if total_cols == 0 {
            return true;
        }
        let runs = self
            .state
            .viewport_style_runs
            .get(row)
            .map(|v| v.as_slice())
            .unwrap_or(&[]);
        let spans = crate::view::bg_merge::merge_bg_runs_transparent(
            runs,
            total_cols,
            self.session.default_background(),
        );
        let covered: u16 = spans.iter().map(|s| s.end_col - s.start_col + 1).sum();
        covered == total_cols
    }
}
