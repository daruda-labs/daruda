//! OSC 133 prompt-mark recording and the command history / output ranges
//! derived from it.

use super::*;

impl TerminalSession {
    /// Recently observed FinalTerm / OSC 133 boundaries, oldest first,
    /// bounded to ~1024 entries.
    pub fn prompt_marks(&self) -> &VecDeque<PromptMark> {
        &self.prompt_marks
    }

    /// Assign a monotonic `seq` to the init payload, run the FTCS state
    /// transitions, and push the resulting [`PromptMark`] into the FIFO.
    /// The only place that constructs a `PromptMark` literal with a seq,
    /// so the uniqueness invariant lives in one site.
    pub(super) fn push_prompt_mark(&mut self, init: PromptMarkInit) {
        // `+= 1` (debug-panics on u64 overflow, idiomatic) is preferred
        // over `saturating_add` for the seq uniqueness invariant — silent
        // saturation at `u64::MAX` would alias every subsequent mark to
        // the same seq and break identity-based focus tracking. Overflow
        // is unreachable in practice (~2^64 marks), but a loud failure
        // mode beats silent corruption if it ever happens.
        self.prompt_mark_seq += 1;
        match init.kind {
            PromptMarkKind::PromptStart => {
                self.prompt_arrived = true;
                // A new prompt without an intervening CommandFinished
                // means the previous command was aborted (Ctrl-C, kill,
                // shell exit). Drop the orphaned start so the next
                // command's elapsed measurement starts fresh.
                self.command_started_at = None;
            }
            PromptMarkKind::CommandStart => {
                self.command_started_at = Some(Instant::now());
            }
            PromptMarkKind::CommandFinished => {
                if let Some(start) = self.command_started_at.take() {
                    self.pending_finished_command_elapsed = Some(start.elapsed());
                }
            }
            _ => {}
        }
        if self.prompt_marks.len() >= PROMPT_MARKS_CAP {
            self.prompt_marks.pop_front();
        }
        self.prompt_marks.push_back(PromptMark {
            kind: init.kind,
            seq: self.prompt_mark_seq,
            abs_y: init.abs_y,
            screen_col: init.screen_col,
            exit_code: init.exit_code,
            timestamp: init.timestamp,
        });
    }

    /// Consume the pending prompt-arrival signal set when a PromptStart
    /// (OSC 133 A) mark is received.  Returns `true` once per new prompt.
    pub fn take_prompt_arrived(&mut self) -> bool {
        std::mem::take(&mut self.prompt_arrived)
    }

    /// Whether shell integration reports an executing command (OSC 133 C).
    /// Typing at the prompt (B) is idle; a new prompt also closes an orphaned C.
    pub fn command_is_running(&self) -> bool {
        self.prompt_marks
            .iter()
            .rev()
            .find_map(|mark| match mark.kind {
                PromptMarkKind::CommandExecuted => Some(true),
                PromptMarkKind::PromptStart
                | PromptMarkKind::CommandStart
                | PromptMarkKind::CommandFinished => Some(false),
                _ => None,
            })
            .unwrap_or(false)
    }

    /// Return the row range of the most recent complete command output,
    /// derived from the last FTCS `E`…`F` pair (or `C`…`D` fallback).
    /// Returns `None` if no closed block has been observed, or if
    /// either bound has been evicted from `LineBuffer`. Rows are in
    /// the current screen-frame coordinate space — pass directly to
    /// [`Self::dump_screen_row`].
    pub fn last_command_output_rows(&self) -> Option<std::ops::Range<u32>> {
        // Walk backwards so we always pick the most recent complete
        // pair. Prefer the semantic E/F pair (output body only) over
        // the C/D pair (which includes the command echo itself).
        let mut end_abs: Option<LogicalLineAbs> = None;
        for mark in self.prompt_marks.iter().rev() {
            match mark.kind {
                PromptMarkKind::SemanticTextEnd if end_abs.is_none() => {
                    end_abs = Some(mark.abs_y);
                }
                PromptMarkKind::SemanticTextStart => {
                    if let Some(end) = end_abs {
                        return self.abs_range_to_screen(mark.abs_y, end);
                    }
                }
                _ => {}
            }
        }
        // Fallback: the C/D pair captures the whole command-through-
        // output region when the shell doesn't emit E/F.
        let mut end_abs: Option<LogicalLineAbs> = None;
        for mark in self.prompt_marks.iter().rev() {
            match mark.kind {
                PromptMarkKind::CommandFinished if end_abs.is_none() => {
                    end_abs = Some(mark.abs_y);
                }
                PromptMarkKind::CommandExecuted => {
                    if let Some(end) = end_abs {
                        return self.abs_range_to_screen(mark.abs_y, end);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// Translate an `[start_abs, end_abs)` pair of absolute Y
    /// coordinates to a current-frame `Range<u32>`. Returns `None` if
    /// either bound has been evicted from `LineBuffer`, or if the range
    /// would be empty.
    fn abs_range_to_screen(
        &self,
        start_abs: LogicalLineAbs,
        end_abs: LogicalLineAbs,
    ) -> Option<std::ops::Range<u32>> {
        let start = self.abs_to_screen_row(start_abs)?;
        let end = self.abs_to_screen_row(end_abs)?;
        if start >= end {
            return None;
        }
        Some(start..end)
    }

    /// Walk `prompt_marks` for completed shell commands. Each
    /// `(CommandStart, CommandExecuted)` pair becomes one entry; a
    /// following `CommandFinished` populates the exit code. Entries
    /// are returned oldest-first so a UI consumer can render the
    /// natural top-down or reverse it for "most recent first".
    /// Pairs whose `B` mark has been evicted from `LineBuffer` are
    /// dropped (we can no longer slice the typed command text).
    pub fn command_history(&self) -> Vec<CommandHistoryEntry> {
        let mut entries: Vec<CommandHistoryEntry> = Vec::new();
        let mut last_b: Option<PromptMark> = None;
        // Carry the C-mark timestamp of the most recently opened entry so
        // a following D mark can compute the C→D elapsed duration. Reset
        // when a D mark consumes it (or when a fresh C opens a new entry).
        let mut last_c_ts: Option<std::time::SystemTime> = None;
        for mark in &self.prompt_marks {
            match mark.kind {
                PromptMarkKind::CommandStart => {
                    last_b = Some(*mark);
                }
                PromptMarkKind::CommandExecuted => {
                    let Some(b) = last_b.take() else {
                        continue;
                    };
                    let Some(b_row) = self.abs_to_screen_row(b.abs_y) else {
                        continue;
                    };
                    let Some(text) = self.extract_command_text(&b, mark, b_row) else {
                        continue;
                    };
                    if text.trim().is_empty() {
                        continue;
                    }
                    entries.push(CommandHistoryEntry {
                        command_text: text,
                        start_row: b_row,
                        exit_code: None,
                        duration: None,
                    });
                    last_c_ts = Some(mark.timestamp);
                }
                PromptMarkKind::CommandFinished => {
                    if let Some(last) = entries.last_mut()
                        && last.exit_code.is_none()
                    {
                        last.exit_code = mark.exit_code;
                        if let Some(c_ts) = last_c_ts.take() {
                            last.duration = mark.timestamp.duration_since(c_ts).ok();
                        }
                    }
                }
                _ => {}
            }
        }
        entries
    }

    /// Slice the typed command out of the grid using the captured
    /// `(abs_y, col)` of the `B` and `C` marks. Spans multiple rows
    /// when the shell's prompt + command wrapped. Wide characters
    /// (CJK / emoji) widen the column span by 2 each but are sliced
    /// by char count in the dumped row string — close enough for
    /// the picker label, which is truncated visually anyway.
    ///
    /// Returns `None` when either mark has been evicted from
    /// `LineBuffer` — the typed command is no longer reachable.
    ///
    /// `b_row` is the pre-translated screen row of `b.abs_y` — callers
    /// already need it for `CommandHistoryEntry::start_row`, so pass it
    /// in to avoid a redundant `abs_to_screen_row` call.
    fn extract_command_text(&self, b: &PromptMark, c: &PromptMark, b_row: u32) -> Option<String> {
        let start_col = (b.screen_col as usize).saturating_sub(1);
        let end_col = (c.screen_col as usize).saturating_sub(1);
        let c_row = self.abs_to_screen_row(c.abs_y)?;
        if b_row > c_row {
            // Defensive: monotonic abs_y + monotonic translation makes this
            // unreachable in practice, but eviction of just one of the two
            // marks could in principle invert the range.
            return None;
        }

        if b_row == c_row {
            let row = self.dump_screen_row(b_row).unwrap_or_default();
            return Some(slice_chars(&row, start_col, end_col).trim_end().to_string());
        }

        let mut out = String::new();
        if let Ok(row) = self.dump_screen_row(b_row) {
            out.push_str(slice_chars(&row, start_col, usize::MAX).trim_end());
        }
        let mut r = b_row + 1;
        while r < c_row {
            if let Ok(row) = self.dump_screen_row(r) {
                out.push('\n');
                out.push_str(row.trim_end());
            }
            r += 1;
        }
        if end_col > 0
            && let Ok(row) = self.dump_screen_row(c_row)
        {
            out.push('\n');
            out.push_str(slice_chars(&row, 0, end_col).trim_end());
        }
        Some(out.trim_end().to_string())
    }
}
