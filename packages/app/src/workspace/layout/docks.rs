//! The three docks a window lays out, and which view each side dock shows.

use gpui::{AppContext, Context, Entity, WeakEntity};

use super::{Dock, DockPosition};
use crate::workspace::Workspace;

pub(in crate::workspace) struct Docks {
    /// Left dock (lanes, git changes, files — the active view is `left_view`).
    pub(in crate::workspace) left: Entity<Dock>,
    pub(in crate::workspace) right: Entity<Dock>,
    pub(in crate::workspace) bottom: Entity<Dock>,
    /// Active view inside the left dock. Persisted via `ProjectState`.
    pub(in crate::workspace) left_view: daruda_store::project::LeftDockView,
    /// Active utility tab (Usage / Skills / Tools), independent of pages.
    pub(in crate::workspace) right_view: daruda_store::project::RightDockView,
    /// Dock resize drag — active while the user is pulling on the right edge
    /// of the left dock, the left edge of the right dock, or the top edge of
    /// the bottom dock.
    pub(in crate::workspace) drag: Option<super::ops::DockDrag>,
}

impl Docks {
    pub(in crate::workspace) fn new(
        ws_weak: &WeakEntity<Workspace>,
        config: &daruda_config::Config,
        cx: &mut Context<Workspace>,
    ) -> Self {
        let left = {
            let ws = ws_weak.clone();
            cx.new(|_| {
                let mut d = Dock::new(DockPosition::Left, ws);
                d.resize(config.left_dock.left_default_width);
                d.is_open = !config.left_dock.left_collapsed_by_default;
                // Register the three left-dock views. Only the count is
                // read (by the layout pass); the tab strip owns selection.
                d.add_panel(super::LanesPanel);
                d.add_panel(super::GitChangesPanel);
                d.add_panel(super::FilesPanel);
                d
            })
        };
        let bottom = {
            let ws = ws_weak.clone();
            cx.new(|_| {
                let mut d = Dock::new(DockPosition::Bottom, ws);
                d.add_panel(super::MacrosPanel);
                d
            })
        };
        let right = {
            let ws = ws_weak.clone();
            cx.new(|_| {
                let mut d = Dock::new(DockPosition::Right, ws);
                d.add_panel(super::AgentChatPanel);
                d
            })
        };
        Self {
            left,
            right,
            bottom,
            left_view: daruda_store::project::LeftDockView::default(),
            right_view: daruda_store::project::RightDockView::default(),
            drag: None,
        }
    }
}
