//! What a mutation did to the row list, and the one place that tells gpui.
//!
//! Callers name the change; they do not pick a scroll anchor. That choice was
//! being re-argued in prose at every mutation site, and the two gpui entry
//! points differ in a way no call site makes visible: both keep the scroll-top
//! *item*, but [`ListState::remeasure_items`] keeps the offset inside it in
//! pixels while [`ListState::remeasure`] keeps it as a fraction of that item's
//! height. The fraction is what a width or font change wants, since every row's
//! height moved; pixels are what a content change wants, since the row under
//! the reader may not have moved at all.
//!
//! [`ListState::remeasure_items`]: gpui::ListState::remeasure_items
//! [`ListState::remeasure`]: gpui::ListState::remeasure

use std::ops::Range;

use super::AgentChatView;

/// The change a mutation made, in the terms the list needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::workspace) enum ListSync {
    /// Row identity or count changed from `from` on. Scroll above `from` is
    /// preserved; nothing below it is assumed to still exist.
    Structure { from: usize, old_len: usize },
    /// These rows' rendered heights changed and nothing else did. The reader
    /// keeps the exact pixel they were on.
    Rows(Range<usize>),
    /// Heights changed everywhere; the reader keeps their *relative* place
    /// inside the top row.
    ///
    /// Right when the layout itself moved — a width or font change puts every
    /// row at a new height, the top row included. Also what the content-change
    /// paths that cannot name an index still use: they differ from
    /// `Rows(0..n)` only when the top row is one of the rows that changed, so
    /// moving them is a judgement about which anchor reads better, not a fix.
    EveryRow,
}

impl AgentChatView {
    /// Apply one [`ListSync`] and trace it under the calling path's own name.
    /// The single place `list_state` is told anything, so a new mutation path
    /// cannot invent a fourth rule — and `reason` keeps the trace answering the
    /// question it exists for, which path caused this sync.
    pub(in crate::workspace) fn apply_list_sync(&mut self, sync: ListSync, reason: &str) {
        match sync {
            ListSync::Structure { from, old_len } => {
                self.list_state
                    .splice(from..old_len, self.rows.len().saturating_sub(from));
                self.trace_list_sync(reason, from, self.rows.len(), old_len);
            }
            ListSync::Rows(range) => {
                let (from, to) = (range.start, range.end);
                self.list_state.remeasure_items(range);
                self.trace_list_sync(reason, from, to, self.rows.len());
            }
            ListSync::EveryRow => {
                let n = self.rows.len();
                self.list_state.remeasure();
                self.trace_list_sync(reason, 0, n, n);
            }
        }
    }

    /// [`ListSync::Rows`] over every row — "heights changed, I cannot say
    /// which, and the reader must not move". Distinct from
    /// [`ListSync::EveryRow`] in the anchor it keeps, which is the whole reason
    /// this is spelled out rather than left to each caller.
    pub(super) fn resync_all_row_heights(&mut self, reason: &str) {
        let n = self.rows.len();
        if n > 0 {
            self.apply_list_sync(ListSync::Rows(0..n), reason);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three cases are distinct values, so a caller cannot express "rows
    /// changed" and get the layout anchor by accident.
    #[test]
    fn the_three_changes_are_not_interchangeable() {
        assert_ne!(ListSync::Rows(0..3), ListSync::EveryRow);
        assert_ne!(
            ListSync::Structure {
                from: 0,
                old_len: 3
            },
            ListSync::Rows(0..3)
        );
    }
}
