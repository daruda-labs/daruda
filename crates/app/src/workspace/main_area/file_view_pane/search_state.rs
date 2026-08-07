//! Find-panel state for the file viewer (GPUI-free).
//!
//! `FileViewerSearch` holds the live query + match state, and the `PaneFileView`
//! `search_*` methods that drive it live here. This is the `PaneFileView`-side
//! counterpart to the workspace-side `search_ops.rs` (`impl Workspace`).

use super::{FileViewMode, PaneFileContent, PaneFileView, VisualRow};

/// Live state of the find panel inside the file viewer.
pub(in crate::workspace) struct FileViewerSearch {
    /// The current query string typed by the user.
    pub query: String,
    /// Row indices (into `active_rows()`) that contain the query string.
    pub matches: Vec<usize>,
    /// Index into `matches` that is currently highlighted.
    /// `None` when the query is empty or there are no matches.
    pub focused: Option<usize>,
}

impl PaneFileView {
    /// Open the search panel. Resets the query to empty if the panel was already open.
    pub(in crate::workspace) fn search_open(&mut self) {
        if self.search.is_none() {
            self.search = Some(FileViewerSearch {
                query: String::new(),
                matches: Vec::new(),
                focused: None,
            });
        }
    }

    /// Close the search panel.
    pub(in crate::workspace) fn search_close(&mut self) {
        self.search = None;
    }

    /// Update the search query from the TextInput widget and recompute matches.
    pub(in crate::workspace) fn search_update_query(&mut self, query: &str) {
        if let Some(s) = &mut self.search {
            s.query = query.to_string();
        }
        self.search_recompute();
    }

    /// Append a character to the search query and recompute matches.
    #[allow(dead_code)]
    pub(in crate::workspace) fn search_insert_char(&mut self, ch: char) {
        if let Some(s) = &mut self.search {
            s.query.push(ch);
        }
        self.search_recompute();
    }

    /// Remove the last character from the search query and recompute matches.
    #[allow(dead_code)]
    pub(in crate::workspace) fn search_backspace(&mut self) {
        if let Some(s) = &mut self.search {
            s.query.pop();
        }
        self.search_recompute();
    }

    /// Advance the focused match to the next one (wraps around).
    pub(in crate::workspace) fn search_next_match(&mut self) {
        if let Some(s) = &mut self.search {
            if s.matches.is_empty() {
                return;
            }
            s.focused = Some(match s.focused {
                None => 0,
                Some(i) => (i + 1) % s.matches.len(),
            });
        }
    }

    /// Move the focused match to the previous one (wraps around).
    pub(in crate::workspace) fn search_prev_match(&mut self) {
        if let Some(s) = &mut self.search {
            if s.matches.is_empty() {
                return;
            }
            s.focused = Some(match s.focused {
                None => s.matches.len().saturating_sub(1),
                Some(0) => s.matches.len() - 1,
                Some(i) => i - 1,
            });
        }
    }

    /// Clear the search query and all match state without closing the panel.
    #[allow(dead_code)]
    pub(in crate::workspace) fn search_clear(&mut self) {
        if let Some(s) = &mut self.search {
            s.query.clear();
            s.matches.clear();
            s.focused = None;
        }
    }

    /// Row index of the currently focused match, or `None`.
    pub(in crate::workspace) fn search_focused_row(&self) -> Option<usize> {
        let s = self.search.as_ref()?;
        let fi = s.focused?;
        s.matches.get(fi).copied()
    }

    /// Recompute which rows match the current query.
    fn search_recompute(&mut self) {
        let Some(mut s) = self.search.take() else {
            return;
        };
        s.matches.clear();
        s.focused = None;
        if !s.query.is_empty() {
            let query_lower = s.query.to_lowercase();

            // Preview mode: search block plain text (one match index = one block index).
            if let PaneFileContent::LoadedMarkdown { blocks, .. } = &self.content
                && self.view_mode == FileViewMode::Preview
            {
                for (i, block) in blocks.iter().enumerate() {
                    let text = super::markdown_viewer::md_block_plain_text(block);
                    if text.to_lowercase().contains(&query_lower) {
                        s.matches.push(i);
                    }
                }
            } else {
                let rows: &[VisualRow] = match &self.content {
                    PaneFileContent::LoadedRaw => &[],
                    PaneFileContent::LoadedDiff {
                        rows_all,
                        rows_no_ctx,
                        ..
                    } => {
                        if self.hide_unchanged {
                            rows_no_ctx
                        } else {
                            rows_all
                        }
                    }
                    PaneFileContent::LoadedMarkdown { raw_rows, .. } => raw_rows,
                    _ => &[],
                };
                for (i, row) in rows.iter().enumerate() {
                    if row.content.to_lowercase().contains(&query_lower) {
                        s.matches.push(i);
                    }
                }
            }

            if !s.matches.is_empty() {
                s.focused = Some(0);
            }
        }
        self.search = Some(s);
    }
}

#[cfg(test)]
mod tests {
    use crate::workspace::main_area::file_view_pane::{
        FileViewMode, PaneFileContent, PaneFileView, SelectionDrag, VisualRow, VisualRowKind,
    };

    fn raw_viewer(_contents: &[&str]) -> PaneFileView {
        PaneFileView {
            lane_id: 0,
            path: "test.txt".into(),
            staged: false,
            file_status: None,
            content: PaneFileContent::LoadedRaw,
            view_mode: FileViewMode::Raw,
            hide_unchanged: false,
            selection_drag: SelectionDrag::None,
            search: None,
            pending_scroll_line: None,
        }
    }

    fn diff_viewer(contents: &[&str]) -> PaneFileView {
        let rows_all: Vec<VisualRow> = contents
            .iter()
            .enumerate()
            .map(|(i, s)| VisualRow {
                kind: VisualRowKind::Context,
                line_no_left: (i + 1).to_string(),
                line_no_right: (i + 1).to_string(),
                content: s.to_string(),
                header_context: String::new(),
                spans: Vec::new(),
                word_changes: Vec::new(),
            })
            .collect();
        PaneFileView {
            lane_id: 0,
            path: "test.diff".into(),
            staged: false,
            file_status: None,
            content: PaneFileContent::LoadedDiff {
                rows_all,
                rows_no_ctx: Vec::new(),
                added: 0,
                removed: 0,
            },
            view_mode: FileViewMode::Changes,
            hide_unchanged: false,
            selection_drag: SelectionDrag::None,
            search: None,
            pending_scroll_line: None,
        }
    }

    #[test]
    fn search_lifecycle_and_query_editing() {
        let mut fv = raw_viewer(&["alpha", "beta"]);
        assert!(fv.search.is_none());
        fv.search_open();
        assert!(fv.search.is_some());
        fv.search_close();
        assert!(fv.search.is_none());

        let mut fv = diff_viewer(&["hello world", "foo bar"]);
        fv.search_open();
        "hello".chars().for_each(|c| fv.search_insert_char(c));
        {
            let s = fv.search.as_ref().unwrap();
            assert_eq!(s.query, "hello");
            assert!(!s.matches.is_empty());
        }
        fv.search_clear();
        let s = fv.search.as_ref().unwrap();
        assert_eq!(s.query, "");
        assert!(s.matches.is_empty());
        assert!(s.focused.is_none());
        // Panel remains open after clear.
        assert!(fv.search.is_some());

        let mut fv = raw_viewer(&["hello world", "foo bar"]);
        fv.search_open();
        fv.search_insert_char('h');
        fv.search_insert_char('e');
        assert_eq!(fv.search.as_ref().unwrap().query, "he");
        fv.search_backspace();
        assert_eq!(fv.search.as_ref().unwrap().query, "h");
        fv.search_backspace();
        assert_eq!(fv.search.as_ref().unwrap().query, "");
        fv.search_backspace(); // no-op on empty
        assert_eq!(fv.search.as_ref().unwrap().query, "");
    }

    #[test]
    fn search_matching_cases() {
        let mut fv = diff_viewer(&["hello world", "nothing here", "hello again"]);
        fv.search_open();
        "hello".chars().for_each(|c| fv.search_insert_char(c));
        let s = fv.search.as_ref().unwrap();
        // rows 0 and 2 contain "hello"; row 1 does not
        assert_eq!(s.matches, vec![0, 2]);
        assert_eq!(s.focused, Some(0));

        let mut fv = raw_viewer(&["alpha", "beta"]);
        fv.search_open();
        "zzz".chars().for_each(|c| fv.search_insert_char(c));
        let s = fv.search.as_ref().unwrap();
        assert!(s.matches.is_empty());
        assert!(s.focused.is_none());

        let mut fv = diff_viewer(&["Hello", "world", "HELLO"]);
        fv.search_open();
        "hello".chars().for_each(|c| fv.search_insert_char(c));
        let s = fv.search.as_ref().unwrap();
        assert_eq!(s.matches, vec![0, 2]);

        let mut fv = diff_viewer(&["hello", "world"]);
        fv.search_open();
        "hello".chars().for_each(|c| fv.search_insert_char(c));
        assert_eq!(fv.search.as_ref().unwrap().matches, vec![0]);
        for _ in 0.."hello".len() {
            fv.search_backspace();
        }
        assert!(fv.search.as_ref().unwrap().matches.is_empty());
        assert!(fv.search.as_ref().unwrap().focused.is_none());
    }

    #[test]
    fn search_focus_navigation_cases() {
        let mut fv = diff_viewer(&["aaa", "bbb", "aaa", "aaa"]);
        fv.search_open();
        fv.search_insert_char('a');
        // matches: [0, 2, 3], focused = Some(0)
        fv.search_next_match();
        assert_eq!(fv.search.as_ref().unwrap().focused, Some(1));
        fv.search_next_match();
        assert_eq!(fv.search.as_ref().unwrap().focused, Some(2));
        fv.search_next_match(); // wraps
        assert_eq!(fv.search.as_ref().unwrap().focused, Some(0));
        fv.search_prev_match();
        assert_eq!(fv.search.as_ref().unwrap().focused, Some(2));

        let mut fv = diff_viewer(&["aaa", "bbb", "aaa"]);
        fv.search_open();
        fv.search_insert_char('a');
        // matches = [0, 2], focused = Some(0) → row 0
        assert_eq!(fv.search_focused_row(), Some(0));
        fv.search_next_match();
        // focused = Some(1) → row 2
        assert_eq!(fv.search_focused_row(), Some(2));
    }
}
