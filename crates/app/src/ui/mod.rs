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
pub mod button_group;
pub mod chart;
pub mod checkbox;
pub mod code_editor;
pub mod context_menu;
pub mod dialog;
pub mod disclosure;
pub mod divider;
pub mod form_helpers;
pub mod group_box;
pub mod highlighter;
pub mod input;
pub mod input_panel;
pub mod label;
pub mod list;
pub mod macro_key;
pub mod markdown;
pub mod menu;
pub mod placeholder;
pub mod progress;
pub mod radio;
pub mod scrollbar;
pub mod section_header;
pub mod select;
pub mod tab_bar;
pub mod theme;
pub mod tooltip;

pub use agent_status_badge::{AgentStatusBadge, IndicatorSize, StatusPulseClock, color_for_status};
pub use badge::Badge;
pub use button::{
    Button, button, button_add_tile, button_bare, button_chip, button_close, button_danger,
    button_delete_glyph, button_header_action, button_primary, button_toggle,
};
pub use button_group::{ButtonGroup, button_group};
pub use chart::BarChart;
pub use checkbox::{Checkbox, checkbox};
pub use code_editor::{
    LineDecoration, file_viewer_editor, make_markdown_prose_state, make_markdown_state,
    markdown_editor,
};
pub use context_menu::{ContextMenu, ContextMenuCorner, ContextMenuItem};
pub use disclosure::{Disclosure, disclosure};
pub use divider::Divider;
pub use form_helpers::{checkbox_row, field_row};
pub use group_box::{GroupBox, GroupBoxVariants, group_box};
pub use input::{
    CompletionProvider, HistoryDir, Input, InputEvent, InputGrowMode, InputState, Rope, RopeExt,
    input, input_with_action, input_with_action_grow,
};
pub use input_panel::{
    InputPanel, InputPanelEvent, InputPanelLayout, PanelAction, PanelActionVariant,
};
pub use label::Label;
pub use macro_key::{KeyDisplay, MacroKey};
pub use markdown::{Markdown, markdown};
pub use menu::{ContextMenuExt, DropdownMenu, PopupMenu, PopupMenuItem, menu_builder};
pub use placeholder::placeholder_text;
pub use progress::{Progress, progress};
pub use radio::{Radio, radio};
pub use section_header::SectionHeader;
pub use tab_bar::{Tab, TabBar, tab, tab_bar};

pub use gpui_component::button::{ButtonVariant, ButtonVariants, DropdownButton};
pub use gpui_component::scroll::ScrollableElement;
pub use gpui_component::text_selection::{CharType, word_range};
pub use gpui_component::{ActiveTheme, Disableable, Selectable, Sizable, WindowExt};
pub use gpui_component::{Icon, IconName};

#[cfg(test)]
mod word_range_tests {
    use super::{CharType, word_range};

    /// Byte-offset `char_at` for a plain &str.
    fn str_char_at(s: &str, byte_offset: usize) -> Option<char> {
        if byte_offset >= s.len() || !s.is_char_boundary(byte_offset) {
            return None;
        }
        s[byte_offset..].chars().next()
    }

    fn word(s: &str, offset: usize) -> Option<String> {
        let range = word_range(s.len(), |i| str_char_at(s, i), offset)?;
        Some(s[range].to_string())
    }

    #[test]
    fn word_range_ascii_word() {
        let s = "hello world";
        assert_eq!(word(s, 0).as_deref(), Some("hello"));
        assert_eq!(word(s, 4).as_deref(), Some("hello"));
        assert_eq!(word(s, 6).as_deref(), Some("world"));
    }

    #[test]
    fn word_range_underscore_connects() {
        let s = "foo_bar baz";
        assert_eq!(word(s, 2).as_deref(), Some("foo_bar"));
        assert_eq!(word(s, 7).as_deref(), Some(" "));
        assert_eq!(word(s, 8).as_deref(), Some("baz"));
    }

    #[test]
    fn word_range_punctuation_is_single() {
        let s = "a.b[c]";
        assert_eq!(word(s, 1).as_deref(), Some("."));
        assert_eq!(word(s, 3).as_deref(), Some("["));
        assert_eq!(word(s, 5).as_deref(), Some("]"));
    }

    #[test]
    fn word_range_multibyte_cjk() {
        // "中文" — each char is 3 bytes.
        let s = "中文";
        assert_eq!(word(s, 0).as_deref(), Some("中"));
        assert_eq!(word(s, 3).as_deref(), Some("文"));
    }

    #[test]
    fn word_range_out_of_bounds_is_none() {
        let s = "hi";
        assert_eq!(word(s, 2), None); // offset == len
        assert_eq!(word(s, 99), None);
    }

    #[test]
    fn char_type_classification() {
        assert_eq!(CharType::from_char('a'), CharType::Word);
        assert_eq!(CharType::from_char('_'), CharType::Word);
        assert_eq!(CharType::from_char(' '), CharType::Whitespace);
        assert_eq!(CharType::from_char('\n'), CharType::Newline);
        assert_eq!(CharType::from_char('.'), CharType::Other);
        assert_eq!(CharType::from_char('中'), CharType::Other);
    }
}
