//! What the tab strip draws for each tab, gathered before the element tree
//! is built so no entity is read while it is.

use gpui::{App, SharedString};

use crate::workspace::Workspace;
use crate::workspace::main_area::pane::TabEntry;

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
    /// `Some` only for a file pane: what the tab menu's copy-path items copy.
    pub(super) file_path: Option<std::path::PathBuf>,
    pub(super) worktree_root: Option<std::path::PathBuf>,
    /// The tab the next left-dock preview replaces.
    pub(super) is_scratch: bool,
    /// The most urgent agent session among the tab's panes.
    pub(in crate::workspace) status: Option<daruda_agent::SessionStatus>,
}

impl Workspace {
    pub(in crate::workspace) fn tab_cells(&self, cx: &App) -> Vec<TabCell> {
        // Resolved once rather than per tab: it reads every pane's dirty
        // state, and the answer is one index either way.
        let scratch_tab = self.preview_tab_index(cx);
        let runtime = self.active_runtime();
        let acp_statuses: Vec<_> = runtime
            .panes
            .iter()
            .filter_map(|p| {
                let status = p.agent_chat_view()?.read(cx).to_session_status()?;
                Some((p.id, status))
            })
            .collect();
        runtime
            .tabs
            .iter()
            .enumerate()
            .map(|(i, tab)| {
                let pane = runtime.panes.iter().find(|p| p.id == tab.last_focused_pane);
                let base_label = self.tab_label(tab, cx);
                // The dirty dot lets unsaved edits be spotted at a glance —
                // File panes in Raw mode and TaskEdit panes both report them.
                let label = if pane.is_some_and(|p| p.tab_dirty_dot(cx)) {
                    SharedString::from(format!(
                        "{}{}",
                        crate::surface::strings::TAB_TITLE_DIRTY_DOT,
                        base_label
                    ))
                } else {
                    base_label
                };
                let (file_path, worktree_root) = match pane.and_then(|p| p.file_identity()) {
                    Some((path, wt_id)) => {
                        let root = self
                            .active_lanes()
                            .iter()
                            .find(|wt| wt.id == wt_id)
                            .map(|wt| wt.path.clone());
                        (Some(path), root)
                    }
                    None => (None, None),
                };
                let status = crate::workspace::claude_status_aggregate::tab_session_status(
                    &tab.layout.pane_ids(),
                    &self.claude.pty_claude_bindings,
                    &self.claude.claude_status,
                    &acp_statuses,
                );
                TabCell {
                    index: i,
                    id: tab.id,
                    is_active: i == runtime.active_tab_index,
                    label,
                    file_path,
                    worktree_root,
                    is_scratch: scratch_tab == Some(i),
                    status,
                }
            })
            .collect()
    }

    /// What a tab is called: a name the user gave it, else its focused
    /// pane's cwd basename, else that pane's own title — iTerm2's "Show
    /// profile name → working directory" preference. A file pane is named by
    /// its file; the parent directory is in its toolbar.
    pub(in crate::workspace) fn tab_label(&self, tab: &TabEntry, cx: &App) -> SharedString {
        if self.is_orchestrator_tab(tab) {
            return crate::surface::strings::orchestrator_label().into();
        }
        if let Some(label) = tab.user_label.clone() {
            return label;
        }
        let pane = self
            .active_runtime()
            .panes
            .iter()
            .find(|p| p.id == tab.last_focused_pane);
        pane.and_then(|p| (!p.is_file()).then(|| p.display_cwd()).flatten())
            .or_else(|| pane.map(|p| p.title(cx)))
            .unwrap_or_else(|| "shell".into())
    }
}
