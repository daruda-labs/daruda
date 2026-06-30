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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_push_ignored() {
        let mut h = HistoryBuffer::new();
        h.push("");
        assert!(h.prev("").is_none());
    }

    #[test]
    fn single_entry_prev_returns_it() {
        let mut h = HistoryBuffer::new();
        h.push("ls");
        assert_eq!(h.prev(""), Some("ls"));
    }

    #[test]
    fn dedup_consecutive() {
        let mut h = HistoryBuffer::new();
        h.push("ls");
        h.push("ls"); // duplicate — should be ignored
        // Only one entry: prev returns "ls" once, second prev stays at "ls".
        assert_eq!(h.prev(""), Some("ls"));
        // Clamped at oldest.
        assert_eq!(h.prev("ls"), Some("ls"));
    }

    #[test]
    fn prev_next_traversal() {
        let mut h = HistoryBuffer::new();
        h.push("a");
        h.push("b");
        h.push("c");

        // ↑ → c (newest)
        assert_eq!(h.prev("draft"), Some("c"));
        // ↑ → b
        assert_eq!(h.prev("c"), Some("b"));
        // ↑ → a (oldest)
        assert_eq!(h.prev("b"), Some("a"));
        // ↑ again → clamp at a
        assert_eq!(h.prev("a"), Some("a"));
        // ↓ → b
        assert_eq!(h.forward(), Some("b"));
        // ↓ → c
        assert_eq!(h.forward(), Some("c"));
        // ↓ past newest → restore draft
        assert_eq!(h.forward(), Some("draft"));
        // ↓ again → None (not navigating)
        assert!(h.forward().is_none());
    }

    #[test]
    fn draft_saved_and_restored() {
        let mut h = HistoryBuffer::new();
        h.push("cmd1");
        h.push("cmd2");

        let draft_text = "partial input";
        assert_eq!(h.prev(draft_text), Some("cmd2"));
        // Navigate all the way back, then forward past newest.
        h.prev("cmd2");
        h.forward();
        assert_eq!(h.forward(), Some(draft_text));
    }

    #[test]
    fn push_resets_cursor() {
        let mut h = HistoryBuffer::new();
        h.push("old");
        h.prev(""); // start navigating
        h.push("new"); // should reset navigation
        // next() should return None (not navigating any more)
        assert!(h.forward().is_none());
        // prev() should start fresh from "new" (newest)
        assert_eq!(h.prev(""), Some("new"));
    }

    #[test]
    fn next_when_not_navigating_returns_none() {
        let mut h = HistoryBuffer::new();
        h.push("x");
        assert!(h.forward().is_none());
    }

    #[test]
    fn per_lane_isolation() {
        let mut lane1 = HistoryBuffer::new();
        let mut lane2 = HistoryBuffer::new();
        lane1.push("from-lane-1");
        // lane2 has no entries, prev returns None.
        assert!(lane2.prev("").is_none());
        // lane1 returns its own entry.
        assert_eq!(lane1.prev(""), Some("from-lane-1"));
    }
}
