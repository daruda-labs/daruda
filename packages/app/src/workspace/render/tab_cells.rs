//! What the tab strip draws for each tab, gathered before the element tree
//! is built so no entity is read while it is.

use gpui::{App, Hsla, SharedString};

use crate::ui::theme;
use crate::workspace::Workspace;
use crate::workspace::main_area::pane::TabEntry;
use crate::workspace::tab_indicator::TabIndicator;

/// One tab's cell in the tab strip.
pub(in crate::workspace) struct TabCell {
    pub(super) index: usize,
    /// The stable `TabEntry` id: the drag payload's identity, surviving a
    /// reorder.
    pub(super) id: u64,
    pub(in crate::workspace) is_active: bool,
    /// The tab's label, prefixed with the dirty dot when a pane in it holds
    /// unsaved edits.
    pub(super) label: SharedString,
    /// The label alone, for the tooltip.
    pub(super) title: SharedString,
    /// `Some` only for a file pane: what the tab menu's copy-path items copy.
    pub(super) file_path: Option<std::path::PathBuf>,
    pub(super) worktree_root: Option<std::path::PathBuf>,
    /// The tab the next left-dock preview replaces.
    pub(super) is_scratch: bool,
    /// The tab's status dot: its most urgent live session or unseen outcome.
    pub(in crate::workspace) indicator: Option<TabIndicator>,
}

impl Workspace {
    pub(in crate::workspace) fn tab_cells(&self, cx: &App) -> Vec<TabCell> {
        // Resolved once rather than per tab: it reads every pane's dirty
        // state, and the answer is one index either way.
        let scratch_tab = self.preview_tab_index(cx);
        let runtime = self.active_runtime();
        let acp_statuses = self.agent_chat_statuses(cx);
        runtime
            .tabs
            .iter()
            .enumerate()
            .map(|(i, tab)| {
                let pane = runtime.panes.iter().find(|p| p.id == tab.last_focused_pane);
                let base_label = self.tab_label(tab, cx).unwrap_or_else(|| "shell".into());
                // The dirty dot lets unsaved edits be spotted at a glance —
                // File panes in Raw mode and flow editors report them.
                let label = if pane.is_some_and(|p| p.tab_dirty_dot(cx)) {
                    SharedString::from(format!(
                        "{}{}",
                        crate::surface::glyphs::TAB_TITLE_DIRTY_DOT,
                        base_label
                    ))
                } else {
                    base_label.clone()
                };
                // A tab strip shows the lane on screen, which owns its panes.
                let (file_path, worktree_root) = match pane.and_then(|p| p.file_path()) {
                    Some(path) => (Some(path), self.active_lane().map(|wt| wt.path.clone())),
                    None => (None, None),
                };
                let pane_ids = tab.layout.pane_ids();
                let live = crate::workspace::claude_status_aggregate::tab_session_status(
                    &pane_ids,
                    &self.claude.pty_claude_bindings,
                    &self.claude.claude_status,
                    &acp_statuses,
                );
                let indicator =
                    TabIndicator::resolve(live, self.unseen_outcomes.for_panes(&pane_ids));
                TabCell {
                    index: i,
                    id: tab.id,
                    is_active: i == runtime.active_tab_index,
                    label,
                    title: base_label,
                    file_path,
                    worktree_root,
                    is_scratch: scratch_tab == Some(i),
                    indicator,
                }
            })
            .collect()
    }

    /// What a tab is called: a name the user gave it, else its focused
    /// pane's cwd basename, else that pane's own title — iTerm2's "Show
    /// profile name → working directory" preference. A file pane is named by
    /// its file; the parent directory is in its toolbar. `None` when the tab
    /// has nothing to be called by; each caller has its own fallback.
    pub(in crate::workspace) fn tab_label(&self, tab: &TabEntry, cx: &App) -> Option<SharedString> {
        if self.is_orchestrator_tab(tab) {
            return Some(crate::surface::strings::orchestrator::label().into());
        }
        if let Some(label) = tab.user_label.clone() {
            return Some(label);
        }
        let pane = self
            .active_runtime()
            .panes
            .iter()
            .find(|p| p.id == tab.last_focused_pane);
        pane.and_then(|p| (!p.is_file()).then(|| p.display_cwd()).flatten())
            .or_else(|| pane.map(|p| p.title(cx)))
    }
}

/// The colour of a tab's status dot. Working takes the working blue for a
/// tool call too: the badge palette's tool colour is amber in the dark theme
/// and green in the light one, which a plain dot would read as waiting or done.
pub(super) fn indicator_color(indicator: TabIndicator, cx: &App) -> Hsla {
    let t = theme::current(cx);
    match indicator {
        TabIndicator::Attention => theme::WARNING,
        TabIndicator::Failed | TabIndicator::Errored => t.status_failed_dark,
        TabIndicator::Working => t.status_working_dark,
        TabIndicator::Done => theme::SUCCESS,
    }
}
