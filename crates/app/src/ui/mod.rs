//! Wrapper home for `gpui_component::*`.
//!
//! Application code under `crates/app/src/` must never import
//! `gpui_component::*` directly. Always go through `crate::ui::*`.
//! Trait imports needed to chain widget modifiers are re-exported
//! here (`Disableable`, `ButtonVariants`, `WindowExt`, ...).
//!
//! `small()` is the project-wide default and is auto-applied by the
//! factory functions in each submodule (`button`, `checkbox`, ...).
//! `xsmall()` is only used when a call site explicitly needs a tighter
//! size (icon-only chrome buttons, badges with pixel-level sizing).
//! Variants (`primary`, `danger`) ship as separate factories so the
//! call site reads as one line: `crate::ui::button_primary("save", "Save")`.

pub mod accordion;
pub mod agent_status_badge;
pub mod alert;
pub mod badge;
pub mod button;
pub mod checkbox;
pub mod code_editor;
pub mod context_menu;
pub mod dialog;
pub mod divider;
pub mod form_helpers;
pub mod input;
pub mod input_panel;
pub mod label;
pub mod list;
pub mod macro_key;
pub mod menu;
pub mod radio;
pub mod section_header;
pub mod select;
pub mod tab_bar;
pub mod theme;
pub mod tooltip;

pub use agent_status_badge::{AgentStatusBadge, IndicatorSize, color_for_status};
pub use badge::Badge;
pub use button::{
    Button, button, button_add_tile, button_bare, button_chip, button_close, button_danger,
    button_header_action, button_primary, button_toggle,
};
pub use checkbox::{Checkbox, checkbox};
pub use code_editor::{
    file_viewer_editor, make_markdown_prose_state, make_markdown_state, markdown_editor,
};
pub use context_menu::{ContextMenu, ContextMenuCorner, ContextMenuItem};
pub use divider::Divider;
pub use form_helpers::{checkbox_row, field_row};
pub use input::{
    Input, InputEvent, InputState, input, input_with_action, input_with_action_inline,
};
pub use input_panel::{
    InputPanel, InputPanelEvent, InputPanelLayout, PanelAction, PanelActionVariant,
};
pub use label::Label;
pub use macro_key::{KeyDisplay, MacroKey};
pub use menu::{ContextMenuExt, DropdownMenu, PopupMenu, PopupMenuItem, menu_builder};
pub use radio::{Radio, radio};
pub use section_header::SectionHeader;
pub use tab_bar::{Tab, TabBar, tab, tab_bar};

pub use gpui_component::button::{ButtonVariant, ButtonVariants, DropdownButton};
pub use gpui_component::scroll::ScrollableElement;
pub use gpui_component::{ActiveTheme, Disableable, Sizable, WindowExt};
pub use gpui_component::{Icon, IconName};
