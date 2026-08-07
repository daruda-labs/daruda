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
pub mod agent_icon;
pub mod agent_status_badge;
pub mod alert;
pub mod badge;
pub mod button;
pub mod button_group;
pub mod chart;
pub mod checkbox;
pub mod code_copy_button;
pub mod code_editor;
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
pub mod popover;
pub mod progress;
pub mod radio;
pub mod scrollbar;
pub mod section_header;
pub mod select;
pub mod selectable_text;
pub mod spinner;
pub mod tab_bar;
pub mod theme;
pub mod tooltip;

pub use agent_icon::{agent_icon, agent_menu_icon};
pub use agent_status_badge::{AgentStatusBadge, IndicatorSize, StatusPulseClock, color_for_status};
pub use badge::Badge;
pub use button::{
    Button, button, button_add_tile, button_bare, button_chip, button_close, button_danger,
    button_delete_glyph, button_edit_cancel_glyph, button_edit_glyph, button_header_action,
    button_primary, button_status_pill, button_status_pill_bare, button_toggle,
};
pub use button_group::{ButtonGroup, button_group};
pub use chart::BarChart;
pub use checkbox::{Checkbox, checkbox};
pub use code_copy_button::{code_copy_button, copy_button};
pub use code_editor::{
    LineDecoration, embedded_code_viewer, file_viewer_editor, make_markdown_prose_state,
    make_markdown_state, markdown_editor,
};
pub use disclosure::{Disclosure, disclosure};
pub use divider::Divider;
pub use form_helpers::{checkbox_row, field_row};
pub use group_box::{GroupBox, GroupBoxVariants, group_box};
pub use input::{
    CompletionProvider, HistoryDir, Input, InputEvent, InputGrowMode, InputState, Rope, RopeExt,
    ScrollWheelBehavior, input, input_with_action, input_with_action_grow,
};
pub use input_panel::{
    InputPanel, InputPanelEvent, InputPanelLayout, PanelAction, PanelActionVariant,
};
pub use label::Label;
pub use macro_key::{KeyDisplay, MacroKey};
pub use markdown::{Markdown, markdown};
pub use menu::{
    ContextMenuExt, DropdownMenu, PopupMenu, PopupMenuItem, menu_builder, popup_menu_deferred,
};
pub use placeholder::placeholder_text;
pub use popover::{Popover, PopoverState};
pub use progress::{Progress, progress};
pub use radio::{Radio, radio};
pub use scrollbar::{ScrollArea, scroll_area};
pub use section_header::SectionHeader;
pub use selectable_text::{SelectableText, selectable_text};
pub use spinner::{Spinner, spinner};
pub use tab_bar::{Tab, TabBar, tab, tab_bar};

pub use daruda_core::text::{CharType, char_cell_hit_x, logical_line_range, word_range};
pub use gpui_component::button::{ButtonVariant, ButtonVariants, DropdownButton};
pub use gpui_component::scroll::ScrollableElement;
pub use gpui_component::text::{
    SelectMode, TextSelectionHandle, active_text_selection, select_mode_for_click_count,
};
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
    fn word_range_cases() {
        for (text, offset, expected) in [
            ("hello world", 0, Some("hello")),
            ("hello world", 4, Some("hello")),
            ("hello world", 6, Some("world")),
            ("foo_bar baz", 2, Some("foo_bar")),
            ("foo_bar baz", 7, Some(" ")),
            ("foo_bar baz", 8, Some("baz")),
            ("a.b[c]", 1, Some(".")),
            ("a.b[c]", 3, Some("[")),
            ("a.b[c]", 5, Some("]")),
            ("中文", 0, Some("中文")),
            ("中文", 3, Some("中文")),
            ("한글 테스트", 0, Some("한글")),
            ("한글 테스트", 3, Some("한글")),
            ("한글 테스트", 7, Some("테스트")),
            ("hi", 2, None),
            ("hi", 99, None),
        ] {
            assert_eq!(word(text, offset).as_deref(), expected, "{text}:{offset}");
        }
    }

    #[test]
    fn char_type_classification() {
        for (ch, expected) in [
            ('a', CharType::Word),
            ('_', CharType::Word),
            (' ', CharType::Whitespace),
            ('\n', CharType::Newline),
            ('.', CharType::Other),
            ('中', CharType::Word),
            ('글', CharType::Word),
        ] {
            assert_eq!(CharType::from_char(ch), expected, "{ch}");
        }
    }
}

#[cfg(test)]
mod text_view_select_mode_tests {
    use super::{
        SelectMode, char_cell_hit_x, logical_line_range, select_mode_for_click_count, word_range,
    };

    fn str_char_at(s: &str, byte_offset: usize) -> Option<char> {
        if byte_offset >= s.len() || !s.is_char_boundary(byte_offset) {
            return None;
        }
        s[byte_offset..].chars().next()
    }

    #[test]
    fn select_mode_cases() {
        for (clicks, expected, expands) in [
            (1, SelectMode::Character, false),
            (2, SelectMode::Word, true),
            (3, SelectMode::Line, true),
            (4, SelectMode::All, true),
            (10, SelectMode::All, true),
        ] {
            let mode = select_mode_for_click_count(clicks);
            assert_eq!(mode, expected, "{clicks}");
            assert_eq!(!matches!(mode, SelectMode::Character), expands, "{clicks}");
        }
    }

    /// Word expansion: raw byte range is expanded to word boundaries.
    /// Calls the real `word_range` function from production.
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

    #[test]
    fn logical_line_range_cases() {
        let text = "hello\nworld\nfoo";
        let char_at = |i: usize| str_char_at(text, i);
        let len = text.len();

        for (offset, expected) in [(2, 0..5), (0, 0..5), (7, 6..11), (12, 12..15), (14, 12..15)] {
            assert_eq!(
                logical_line_range(len, char_at, offset),
                expected,
                "{offset}"
            );
        }

        let single_line = "hello world";
        let char_at = |i: usize| str_char_at(single_line, i);
        assert_eq!(logical_line_range(single_line.len(), char_at, 0), 0..11);
        assert_eq!(logical_line_range(single_line.len(), char_at, 5), 0..11);
    }

    /// Line drag union: dragging from one line into another extends the
    /// selection to cover both whole lines.
    /// Uses real `logical_line_range` calls to mirror `on_drag_move` Line arm.
    #[test]
    fn line_drag_union_covers_both_lines() {
        let text = "hello\nworld\nfoo";
        let char_at = |i: usize| str_char_at(text, i);
        let len = text.len();

        // Triple-click anchor on line 0 (offset 2) → 0..5.
        let anchor = logical_line_range(len, char_at, 2);

        // Drag cursor into line 1 (offset 7) → line1 = 6..11.
        let line_at_cursor = logical_line_range(len, char_at, 7);
        let union_start = anchor.start.min(line_at_cursor.start);
        let union_end = anchor.end.max(line_at_cursor.end);
        assert_eq!(&text[union_start..union_end], "hello\nworld");

        // Drag cursor back within line 0 (offset 3) — union stays at line 0.
        let line_at_cursor2 = logical_line_range(len, char_at, 3);
        let s2 = anchor.start.min(line_at_cursor2.start);
        let e2 = anchor.end.max(line_at_cursor2.end);
        assert_eq!(&text[s2..e2], "hello");
    }

    /// A word/line click produces a zero-width selection span (start == end),
    /// and `layout_selections` re-expands from the raw pixel-hit scan. The
    /// scan must still hit the char cell under the click — this is the bug
    /// where only quad-click (which bypasses the scan) appeared to work.
    /// Calls the real `char_cell_hit_x` used by `char_in_text_selection`.
    #[test]
    fn char_cell_hit_click_point_hits_cell_under_point() {
        // Click at x=102, off the center of any 10px cell — the old
        // center-threshold logic (center == click) would miss the containing
        // cell here, which is exactly the "only quad-click works" bug.
        let click = 102.0;
        // Cell [100, 112) contains 102 → hit (center 105 ≠ click).
        assert!(char_cell_hit_x(100.0, 10.0, click, click));
        // Cell [110, 120) is right of the click → miss.
        assert!(!char_cell_hit_x(110.0, 10.0, click, click));
        // Cell [90, 100) is left of the click → miss.
        assert!(!char_cell_hit_x(90.0, 10.0, click, click));
        // Left edge inclusive, right edge exclusive.
        assert!(char_cell_hit_x(102.0, 10.0, click, click));
        assert!(!char_cell_hit_x(92.0, 10.0, click, click));
    }

    /// A drag span keeps the pre-existing half-character (center) threshold:
    /// a char is selected when its center falls inside the span.
    #[test]
    fn char_cell_hit_drag_span_uses_center_threshold() {
        // Span [100, 150]; cells 10px wide.
        // Cell [95, 105): center 100 ≥ 100 → hit (center exactly at left edge).
        assert!(char_cell_hit_x(95.0, 10.0, 100.0, 150.0));
        // Cell [90, 100): center 95 < 100 → miss (matches "left out" convention).
        assert!(!char_cell_hit_x(90.0, 10.0, 100.0, 150.0));
        // Cell [145, 155): center 150 ≤ 150 → hit.
        assert!(char_cell_hit_x(145.0, 10.0, 100.0, 150.0));
        // Cell [150, 160): center 155 > 150 → miss.
        assert!(!char_cell_hit_x(150.0, 10.0, 100.0, 150.0));
    }
}
