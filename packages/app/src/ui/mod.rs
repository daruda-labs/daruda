//! The app's facade over `daruda_ui` — the one path app code reaches widgets,
//! `gpui_component`, and `ferrum_flow` through. On top of the re-export it
//! owns what needs the app: localized wrappers and agent-domain widgets.

pub use daruda_ui::*;

pub mod agent_icon;
pub mod agent_status_badge;
mod localized;

pub use agent_icon::{agent_icon, agent_menu_icon};
pub use agent_status_badge::{AgentStatusBadge, IndicatorSize, StatusPulseClock, color_for_status};
pub use localized::{button_close, button_delete_glyph, code_copy_button, markdown};
