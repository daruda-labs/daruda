//! Popover — re-export. Anchored panel opened from a trigger element, for
//! browsing surfaces that are not command menus (the status-bar Ports chip):
//! clicks inside keep it open; outside click / Escape dismiss it. For a list
//! of commands use `.dropdown_menu` + [`crate::ui::menu_builder`] instead —
//! menu items are expected to dismiss on click, panels are not.

pub use gpui_component::popover::{Popover, PopoverState};
