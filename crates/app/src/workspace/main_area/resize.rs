//! Workspace resize — propagate window viewport changes (and dock
//! open/close) to every PTY's grid + every `TerminalView`'s render.
//!
//! Called from:
//! - `Workspace::new_with_project` (initial bounds capture).
//! - GPUI window-bounds observer (each viewport change).
//! - Dock toggle / dock-resize end (`dock_ops.rs`) when dock width
//!   or height changed.
//! - Pane close + new pane (`mod.rs`) so the surviving panes redraw
//!   to fill freed space.

use gpui::{Context, Window};

use crate::workspace::Workspace;
use super::pane_tree::collect_pane_sizes;
use super::pane::FontMetricsKey;
use crate::workspace::status_bar;
use crate::workspace::{TAB_BAR_HEIGHT, TITLE_BAR_HEIGHT, render};
use daruda_terminal::view::TerminalLayout;

impl Workspace {
    pub fn resize_all_tabs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let size = window.viewport_size();
        let mut width = f32::from(size.width).max(1.0);
        let mut height = (f32::from(size.height)
            - TITLE_BAR_HEIGHT
            - TAB_BAR_HEIGHT
            - status_bar::STATUS_BAR_HEIGHT)
            .max(1.0);

        // Subtract open dock sizes from the available content area.
        // Dock resize handles are absolute overlays (render.rs), so
        // they don't eat flex space — no handle subtraction needed.
        // Left dock always renders via sidebar content/header overrides
        // regardless of panels, so check is_open only.
        let left_open = self.left_dock.read(cx).is_open;
        let left_size = self.left_dock.read(cx).size;
        let right_open = self.right_dock.read(cx).is_open;
        let right_size = self.right_dock.read(cx).size;
        let right_panels_empty = self.right_dock.read(cx).panels.is_empty();
        let bottom_open = self.bottom_dock.read(cx).is_open;
        let bottom_size = self.bottom_dock.read(cx).size;
        if left_open {
            width = (width - left_size).max(1.0);
        }
        if right_open && !right_panels_empty {
            width = (width - right_size).max(1.0);
        }
        if bottom_open {
            height = (height - bottom_size).max(1.0);
        }

        let mut metrics_cache: std::collections::HashMap<FontMetricsKey, TerminalLayout> =
            std::collections::HashMap::new();

        self.main_area.last_viewport = Some((width, height));
        let mut any_measured = false;

        for tab in &self.main_area.tabs {
            let mut sizes = Vec::new();
            collect_pane_sizes(&tab.layout, width, height, &mut sizes);
            // Single-pane tabs have no pane header; split tabs reserve
            // PANE_HEADER_HEIGHT per pane (see render::pane_header).
            let pane_header_h = if tab.layout.leaf_count() > 1 {
                render::PANE_HEADER_HEIGHT
            } else {
                0.0
            };
            for (pane_id, w, h) in sizes {
                let Some(pane) = self.main_area.panes.iter().find(|p| p.id == pane_id) else {
                    continue;
                };
                if pane.resize(w, h, pane_header_h, &mut metrics_cache, window, cx) {
                    any_measured = true;
                }
            }
        }

        if any_measured {
            self.main_area.pending_resize = false;
        } else {
            // No pane could be measured (e.g. text system not ready
            // yet). Defer until the next window bounds notification
            // so we don't leave panes at their default 80×24.
            self.main_area.pending_resize = true;
            cx.notify();
        }
    }
}
