//! Dock system — manages toggle-able side and bottom panels.
//!
//! Each dock occupies a fixed position (Left, Bottom, Right) and
//! contains a list of panel entries. Only one panel per dock is
//! active at a time. Docks are independent from the center pane
//! split tree — they resize via drag handles but do not split.

pub(in crate::workspace) mod ops;
pub(in crate::workspace) mod snap;

// Re-export snapshot types so callers use `crate::workspace::layout::*Snapshot`
// without reaching into the `snap` submodule directly.
pub(in crate::workspace) use self::snap::{
    BottomDockSnapshot, DockSnapshot, GroupSnapshot, LeftDockSnapshot, ProjectSnapshot,
    RightDockSnapshot,
};

use crate::ui::theme;
use gpui::{IntoElement, Render, WeakEntity, div, prelude::*, px};

use crate::surface::strings;

use crate::workspace::Workspace;

/// Where a dock sits relative to the center panes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DockPosition {
    Left,
    Bottom,
    Right,
}

// ----------------------------------------------------------------
// Panel trait
// ----------------------------------------------------------------

/// Identity contract for a dock panel. Implement this on a unit
/// struct (or a struct with config data) and register it via
/// `Dock::add_panel` — no enum variant required.
pub(super) trait Panel {
    fn panel_name(&self) -> &'static str;
    #[allow(dead_code)]
    fn panel_icon(&self) -> &'static str;
}

/// Object-safe version of `Panel`. `Dock` stores `Vec<Box<dyn PanelHandle>>`
/// so heterogeneous panel types can coexist without an enum.
pub(super) trait PanelHandle: Send + Sync {
    fn name(&self) -> &'static str;
    #[allow(dead_code)]
    fn icon(&self) -> &'static str;
}

impl<T: Panel + Send + Sync> PanelHandle for T {
    fn name(&self) -> &'static str {
        self.panel_name()
    }
    fn icon(&self) -> &'static str {
        self.panel_icon()
    }
}

// ----------------------------------------------------------------
// Built-in panel types
// ----------------------------------------------------------------

/// Left-dock lanes view panel.
pub(super) struct LanesPanel;
/// Left-dock git-changes view panel.
pub(super) struct GitChangesPanel;
/// Left-dock files view panel.
pub(super) struct FilesPanel;
/// Bottom-dock macro buttons panel.
pub(super) struct MacrosPanel;
/// Right-dock agent chat panel.
pub(super) struct AgentChatPanel;

impl Panel for LanesPanel {
    fn panel_name(&self) -> &'static str {
        strings::DOCK_PANEL_WORKTREES
    }
    fn panel_icon(&self) -> &'static str {
        "⊞"
    }
}

impl Panel for GitChangesPanel {
    fn panel_name(&self) -> &'static str {
        strings::DOCK_PANEL_GIT
    }
    fn panel_icon(&self) -> &'static str {
        "⎇"
    }
}

impl Panel for FilesPanel {
    fn panel_name(&self) -> &'static str {
        strings::DOCK_PANEL_FILES
    }
    fn panel_icon(&self) -> &'static str {
        "◧"
    }
}

impl Panel for MacrosPanel {
    fn panel_name(&self) -> &'static str {
        strings::DOCK_PANEL_MACROS
    }
    fn panel_icon(&self) -> &'static str {
        "⌨"
    }
}

impl Panel for AgentChatPanel {
    fn panel_name(&self) -> &'static str {
        strings::DOCK_PANEL_AGENT_TASKS
    }
    fn panel_icon(&self) -> &'static str {
        "◨"
    }
}

// ----------------------------------------------------------------
// Dock
// ----------------------------------------------------------------

/// State of a single dock.
pub(super) struct Dock {
    pub position: DockPosition,
    pub is_open: bool,
    pub size: f32,
    pub min_size: f32,
    pub max_size: f32,
    /// Registered panels. Heterogeneous via `PanelHandle` — adding a new
    /// panel type requires no enum change, only `add_panel(MyPanel)`.
    pub panels: Vec<Box<dyn PanelHandle>>,
    /// Index into `panels`. Tab-based docks (left = left_dock_view,
    /// right = right_dock_view) drive selection from `Workspace`
    /// state instead, so this index is currently exercised by the
    /// panel-registration tests only.
    #[allow(dead_code)]
    pub active_panel: usize,
    /// Back-reference to the owning `Workspace`. Read by `Workspace::render`
    /// when staging each `DockSnapshot` so event handlers in left/right dock and bottom
    /// renderers can route calls back to `Workspace` without going through
    /// `cx.entity()` (which would give `Entity<Dock>` in that context).
    pub(in crate::workspace) workspace: WeakEntity<Workspace>,
    /// Staged snapshot written by `Workspace::render` before GPUI
    /// descends into this dock's element tree.
    pub(in crate::workspace) snap: DockSnapshot,
}

impl Dock {
    pub fn new(position: DockPosition, workspace: WeakEntity<Workspace>) -> Self {
        let (size, min_size, max_size) = match position {
            DockPosition::Left => (
                theme::DOCK_LEFT_DEFAULT_W,
                theme::DOCK_LEFT_MIN_W,
                theme::DOCK_LEFT_MAX_W,
            ),
            DockPosition::Right => (
                theme::DOCK_RIGHT_DEFAULT_W,
                theme::DOCK_RIGHT_MIN_W,
                theme::DOCK_RIGHT_MAX_W,
            ),
            DockPosition::Bottom => (
                theme::DOCK_BOTTOM_DEFAULT_H,
                theme::DOCK_BOTTOM_MIN_H,
                theme::DOCK_BOTTOM_MAX_H,
            ),
        };
        Self {
            position,
            is_open: false,
            size,
            min_size,
            max_size,
            panels: Vec::new(),
            active_panel: 0,
            workspace,
            snap: DockSnapshot::None,
        }
    }

    /// Register a panel. Panels are displayed in registration order;
    /// `active_panel` indexes into this list.
    pub fn add_panel<P: Panel + Send + Sync + 'static>(&mut self, panel: P) {
        self.panels.push(Box::new(panel));
    }

    pub fn toggle(&mut self) {
        self.is_open = !self.is_open;
    }

    #[allow(dead_code)]
    pub fn resize(&mut self, new_size: f32) {
        self.size = new_size.clamp(self.min_size, self.max_size);
    }

    /// Name of the currently-active panel, or `""` when the panel list is empty.
    #[allow(dead_code)]
    pub fn active_panel_name(&self) -> &'static str {
        self.panels
            .get(self.active_panel)
            .map(|p| p.name())
            .unwrap_or("")
    }
}

// ----------------------------------------------------------------
// GPUI Render
// ----------------------------------------------------------------

impl Render for Dock {
    fn render(
        &mut self,
        _window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl IntoElement {
        if !self.is_open {
            return div().into_any_element();
        }

        let t = theme::current(cx);
        let dock_border = t.border;
        let dock_bg = t.dock_bg;

        let (header_el, content_el) = match &self.snap {
            DockSnapshot::Left(snap) => {
                let header = crate::workspace::left_dock::view_tabs::render(snap, cx);
                let content: gpui::AnyElement = match snap.left_dock_view {
                    daruda_store::project::LeftDockView::Lanes => {
                        crate::workspace::left_dock::projects::render(snap, cx)
                    }
                    daruda_store::project::LeftDockView::GitChanges => {
                        crate::workspace::left_dock::git_changes::render(snap, cx)
                    }
                    daruda_store::project::LeftDockView::Files => {
                        crate::workspace::left_dock::files::render(snap, cx)
                    }
                };
                (header, content)
            }
            DockSnapshot::Bottom(snap) => {
                let header = crate::workspace::main_area::bottom_dock::tab_strip::render(snap, cx);
                let body = crate::workspace::main_area::bottom_dock::render_body(snap, cx);
                (header, body)
            }
            DockSnapshot::Right(snap) => {
                let header = crate::workspace::right_dock::view_tabs::render(snap, cx);
                let content = crate::workspace::right_dock::render(snap, cx);
                (header, content)
            }
            DockSnapshot::None => {
                return div().into_any_element();
            }
        };

        let border = match self.position {
            DockPosition::Left => div().border_r_1(),
            DockPosition::Right => div().border_l_1(),
            DockPosition::Bottom => div().border_t_1(),
        };

        let container = border
            .border_color(dock_border)
            .bg(dock_bg)
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(header_el)
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .child(content_el),
            );

        let sized = match self.position {
            DockPosition::Left | DockPosition::Right => container.w(px(self.size)).h_full(),
            DockPosition::Bottom => container.h(px(self.size)).w_full(),
        };

        sized.into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_weak() -> WeakEntity<Workspace> {
        gpui::WeakEntity::new_invalid()
    }

    struct TestPanel {
        name: &'static str,
        icon: &'static str,
    }

    impl Panel for TestPanel {
        fn panel_name(&self) -> &'static str {
            self.name
        }
        fn panel_icon(&self) -> &'static str {
            self.icon
        }
    }

    // SAFETY: unit struct with only &'static str fields — safe to share across threads.
    unsafe impl Send for TestPanel {}
    unsafe impl Sync for TestPanel {}

    #[test]
    fn dock_starts_closed() {
        let dock = Dock::new(DockPosition::Left, dummy_weak());
        assert!(!dock.is_open);
    }

    #[test]
    fn dock_toggle_flips_state() {
        let mut dock = Dock::new(DockPosition::Bottom, dummy_weak());
        assert!(!dock.is_open);
        dock.toggle();
        assert!(dock.is_open);
        dock.toggle();
        assert!(!dock.is_open);
    }

    #[test]
    fn dock_resize_clamps_to_range() {
        let mut dock = Dock::new(DockPosition::Left, dummy_weak());
        dock.resize(10.0);
        assert_eq!(dock.size, dock.min_size);
        dock.resize(9999.0);
        assert_eq!(dock.size, dock.max_size);
    }

    #[test]
    fn default_sizes_are_within_range() {
        for pos in [
            DockPosition::Left,
            DockPosition::Bottom,
            DockPosition::Right,
        ] {
            let dock = Dock::new(pos, dummy_weak());
            assert!(dock.size >= dock.min_size);
            assert!(dock.size <= dock.max_size);
        }
    }

    #[test]
    fn add_panel_appends_in_order() {
        let mut dock = Dock::new(DockPosition::Left, dummy_weak());
        dock.add_panel(TestPanel {
            name: "A",
            icon: "a",
        });
        dock.add_panel(TestPanel {
            name: "B",
            icon: "b",
        });
        dock.add_panel(TestPanel {
            name: "C",
            icon: "c",
        });
        assert_eq!(dock.panels.len(), 3);
        assert_eq!(dock.panels[0].name(), "A");
        assert_eq!(dock.panels[1].name(), "B");
        assert_eq!(dock.panels[2].name(), "C");
    }

    #[test]
    fn active_panel_name_follows_active_panel_index() {
        let mut dock = Dock::new(DockPosition::Left, dummy_weak());
        dock.add_panel(TestPanel {
            name: "First",
            icon: "1",
        });
        dock.add_panel(TestPanel {
            name: "Second",
            icon: "2",
        });
        assert_eq!(dock.active_panel_name(), "First");
        dock.active_panel = 1;
        assert_eq!(dock.active_panel_name(), "Second");
    }
}
