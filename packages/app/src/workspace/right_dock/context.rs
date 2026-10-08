//! Right-dock view state owned by [`crate::workspace::Workspace`]: what the
//! Tasks and Skills tabs remember between frames, and the folds and scroll
//! every tab shares. Which tab is showing is layout (`Docks::right_view`).

use std::collections::HashSet;

use gpui::{AppContext, Context, ScrollHandle, Window};

use super::section::DockSections;
use super::tasks::TaskBrowser;
use crate::workspace::Workspace;

pub(in crate::workspace) struct RightDockViews {
    /// The Tasks list's scope, status, folds and search.
    pub(in crate::workspace) tasks: TaskBrowser,
    /// Search query input rendered atop the Skills tab. Cleared on `Esc`;
    /// substring-filters Project / Personal / Plugin scopes simultaneously.
    /// The renderer reads the current text via
    /// `RightDockSnapshot::skill_search_query` (captured per frame) so the
    /// panel render closure never re-enters the workspace.
    pub(in crate::workspace) skill_search_input: gpui::Entity<crate::ui::InputState>,
    /// Plugin ids (`<plugin>@<marketplace>`) whose accordion section in the
    /// Skills tab is expanded. Empty means every plugin group renders
    /// collapsed; the user toggles groups via the accordion chevron.
    pub(in crate::workspace) skill_plugin_expanded: HashSet<String>,
    /// Sections the user folded or unfolded against their default.
    pub(in crate::workspace) sections: DockSections,
    /// Scroll handle for the panel body — shared with the scrollbar overlay
    /// by every tab that wraps its body in `overflow_y_scroll`.
    pub(in crate::workspace) scroll_handle: ScrollHandle,
}

impl RightDockViews {
    pub(in crate::workspace) fn new(window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        Self {
            // Task data lives in the app-wide `GlobalTasks`; the browser's
            // subscription rebroadcasts mutations into this workspace's
            // render path and re-evaluates whether the live tick (pulse +
            // duration) needs to be running.
            tasks: TaskBrowser::new(window, cx),
            skill_search_input: cx.new(|cx| {
                crate::ui::InputState::new(window, cx)
                    .placeholder(crate::surface::strings::skills::search_placeholder())
            }),
            skill_plugin_expanded: HashSet::new(),
            sections: DockSections::default(),
            scroll_handle: ScrollHandle::new(),
        }
    }
}
