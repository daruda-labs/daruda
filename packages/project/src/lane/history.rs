//! Per-lane prompt/command history buffer.
//!
//! A single buffer holds both agent prompts and terminal commands in
//! submit order, providing shell-style ↑/↓ history navigation.
//!
//! GPUI-free — all types and logic here are pure Rust.

/// Direction for history navigation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryDir {
    /// Move toward older entries (↑).
    Up,
    /// Move toward more recent entries (↓).
    Down,
}

/// Shell-style history buffer for one lane.
///
/// `entries` grows indefinitely (no eviction — the intent is to match
/// shell history; a sensible cap can be added later).
///
/// The `cursor` is an index into `entries`: `None` = "no navigation in
/// progress", `Some(i)` = currently showing `entries[i]`.
/// Indices count from the *end* of the vec (0 = newest, 1 = second-newest…)
/// so pushes do not invalidate an in-flight cursor.
///
/// `draft` saves the live input text as of the first ↑ press so ↓ past
/// the newest entry restores whatever the user had typed.
pub struct HistoryBuffer {
    entries: Vec<String>,
    /// Distance from the end: `None` = not navigating; `Some(0)` = newest.
    cursor: Option<usize>,
    draft: Option<String>,
}

impl HistoryBuffer {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            cursor: None,
            draft: None,
        }
    }

    /// Push `text` as the newest history entry.
    ///
    /// - Empty strings are ignored.
    /// - Consecutive duplicates are collapsed (like bash's `HISTCONTROL=ignoredups`).
    /// - Always resets the navigation cursor and clears the draft.
    pub fn push(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if self.entries.last().map(String::as_str) == Some(text) {
            // Deduplicate: same text as most-recent entry — just reset cursor.
            self.cursor = None;
            self.draft = None;
            return;
        }
        self.entries.push(text.to_owned());
        self.cursor = None;
        self.draft = None;
    }

    /// Navigate to the previous (older) entry.
    ///
    /// On the first call, `current_text` is saved as the draft so it can
    /// be restored when the user navigates back past the newest entry.
    ///
    /// Returns `Some(&str)` with the text to display, or `None` if there
    /// are no entries to navigate to (caller should do nothing / leave
    /// text unchanged).
    pub fn prev(&mut self, current_text: &str) -> Option<&str> {
        if self.entries.is_empty() {
            return None;
        }

        let next_cursor = match self.cursor {
            None => {
                // First ↑ press: save the current draft and go to newest.
                self.draft = Some(current_text.to_owned());
                0
            }
            Some(c) => {
                if c + 1 >= self.entries.len() {
                    // Already at oldest — clamp.
                    return Some(self.entries[0].as_str());
                }
                c + 1
            }
        };

        self.cursor = Some(next_cursor);
        let idx = self.entries.len() - 1 - next_cursor;
        Some(self.entries[idx].as_str())
    }

    /// Navigate to the next (more recent) entry.
    ///
    /// Returns `Some(&str)` with the text to display. When moving past the
    /// newest entry, returns the saved draft and clears the navigation cursor.
    /// Returns `None` when not currently navigating (nothing to do).
    pub fn forward(&mut self) -> Option<&str> {
        let c = self.cursor?;

        if c == 0 {
            // Already at the newest entry — move back to draft.
            self.cursor = None;
            // Return the saved draft (may be empty string).
            Some(self.draft.as_deref().unwrap_or(""))
        } else {
            let next_cursor = c - 1;
            self.cursor = Some(next_cursor);
            let idx = self.entries.len() - 1 - next_cursor;
            Some(self.entries[idx].as_str())
        }
    }

    /// Reset navigation state without adding an entry. Called when the
    /// input is cleared programmatically (e.g. after submit).
    pub fn reset_cursor(&mut self) {
        self.cursor = None;
        self.draft = None;
    }

    /// Returns `true` when the buffer contains at least one entry.
    /// Used to decide whether ↑ should be consumed.
    pub fn has_entries(&self) -> bool {
        !self.entries.is_empty()
    }

    /// Returns `true` when the user is currently navigating the history
    /// (cursor is active). Used to decide whether ↓ should be consumed.
    pub fn is_navigating(&self) -> bool {
        self.cursor.is_some()
    }
}

impl Default for HistoryBuffer {
    fn default() -> Self {
        Self::new()
    }
}
