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
pub use gpui_component::text::SelectMode;
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

#[cfg(test)]
mod text_view_select_mode_tests {
    use super::{SelectMode, word_range};
    use gpui::{point, px};

    fn str_char_at(s: &str, byte_offset: usize) -> Option<char> {
        if byte_offset >= s.len() || !s.is_char_boundary(byte_offset) {
            return None;
        }
        s[byte_offset..].chars().next()
    }

    /// SelectMode variants must match their click counts.
    #[test]
    fn select_mode_from_click_count() {
        let p = point(px(10.0_f32), px(20.0_f32));

        let mode_for = |click_count: usize| -> SelectMode {
            match click_count {
                2 => SelectMode::Word(p),
                3 => SelectMode::Line(p),
                c if c >= 4 => SelectMode::All,
                _ => SelectMode::Character,
            }
        };

        assert_eq!(mode_for(1), SelectMode::Character);
        assert_eq!(mode_for(2), SelectMode::Word(p));
        assert_eq!(mode_for(3), SelectMode::Line(p));
        assert_eq!(mode_for(4), SelectMode::All);
        assert_eq!(mode_for(10), SelectMode::All);
    }

    /// has_selection logic: Word/Line/All return true even with same start==end.
    #[test]
    fn has_selection_word_mode_without_drag() {
        let p = point(px(10.0_f32), px(20.0_f32));

        let has_selection = |mode: SelectMode, has_start: bool| -> bool {
            match mode {
                SelectMode::Word(_) | SelectMode::Line(_) | SelectMode::All => has_start,
                SelectMode::Character => false,
            }
        };

        assert!(!has_selection(SelectMode::Character, true));
        assert!(has_selection(SelectMode::Word(p), true));
        assert!(has_selection(SelectMode::Line(p), true));
        assert!(has_selection(SelectMode::All, true));
        assert!(!has_selection(SelectMode::All, false));
    }

    /// Word expansion: raw byte range is expanded to word boundaries.
    #[test]
    fn word_expansion_expands_to_boundaries() {
        let text = "hello world foo";
        let char_at = |i: usize| str_char_at(text, i);

        // Click on 'w' (offset 6): word = "world" (6..11).
        let start = word_range(text.len(), char_at, 6)
            .map(|r| r.start)
            .unwrap_or(6);
        let end = word_range(text.len(), char_at, 10)
            .map(|r| r.end)
            .unwrap_or(11);
        assert_eq!(&text[start..end], "world");

        // Click on middle of "hello" (offset 2): word = 0..5.
        let r = word_range(text.len(), char_at, 2).unwrap();
        assert_eq!(&text[r], "hello");

        // Click on space (offset 5): word = space run.
        let r = word_range(text.len(), char_at, 5).unwrap();
        assert_eq!(&text[r], " ");
    }

    /// Visual line expansion: characters on the same Y row are selected.
    ///
    /// Simulates a 2-line layout: "hello " on row y=0, "world" on row y=20.
    #[test]
    fn visual_line_expansion_covers_full_row() {
        use gpui::Pixels;

        let text = "hello world";
        let line_height = px(20.0_f32);

        // Fake position_for_index: first 6 bytes (0..6) on y=0, rest on y=20.
        let position_for_index = |offset: usize| -> Option<gpui::Point<Pixels>> {
            if offset > text.len() {
                return None;
            }
            let y = if offset < 6 { 0.0_f32 } else { 20.0_f32 };
            Some(point(px(offset as f32 * 8.0), px(y)))
        };

        // Simulate a narrow pixel selection inside line 1 (y=20): offsets 6..8.
        let raw_start = 6usize;
        let raw_end = 8usize;
        let raw_top = position_for_index(raw_start)
            .map(|p| p.y)
            .unwrap_or_default();
        let raw_bottom = position_for_index(raw_end.saturating_sub(1))
            .map(|p| p.y + line_height)
            .unwrap_or(raw_top + line_height);

        let (line_start, line_end) = {
            let mut ls: Option<usize> = None;
            let mut le: Option<usize> = None;
            let mut off = 0;
            for c in text.chars() {
                if let Some(pos) = position_for_index(off)
                    && pos.y < raw_bottom
                    && pos.y + line_height > raw_top
                {
                    if ls.is_none() {
                        ls = Some(off);
                    }
                    le = Some(off + c.len_utf8());
                }
                off += c.len_utf8();
            }
            (ls, le)
        };

        // Entire "world" (6..11) should be selected.
        assert_eq!(line_start, Some(6));
        assert_eq!(line_end, Some(11));
        assert_eq!(&text[line_start.unwrap()..line_end.unwrap()], "world");

        // Simulate a narrow selection inside line 0 (y=0): offsets 1..3.
        let raw_start0 = 1usize;
        let raw_end0 = 3usize;
        let raw_top0 = position_for_index(raw_start0)
            .map(|p| p.y)
            .unwrap_or_default();
        let raw_bottom0 = position_for_index(raw_end0.saturating_sub(1))
            .map(|p| p.y + line_height)
            .unwrap_or(raw_top0 + line_height);

        let (ls0, le0) = {
            let mut ls: Option<usize> = None;
            let mut le: Option<usize> = None;
            let mut off = 0;
            for c in text.chars() {
                if let Some(pos) = position_for_index(off)
                    && pos.y < raw_bottom0
                    && pos.y + line_height > raw_top0
                {
                    if ls.is_none() {
                        ls = Some(off);
                    }
                    le = Some(off + c.len_utf8());
                }
                off += c.len_utf8();
            }
            (ls, le)
        };

        // Entire "hello " (0..6) should be selected.
        assert_eq!(ls0, Some(0));
        assert_eq!(le0, Some(6));
        assert_eq!(&text[ls0.unwrap()..le0.unwrap()], "hello ");
    }
}
