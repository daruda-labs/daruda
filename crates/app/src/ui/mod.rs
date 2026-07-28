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
pub use code_copy_button::code_copy_button;
pub use code_editor::{
    LineDecoration, code_diff_viewer, file_viewer_editor, make_markdown_prose_state,
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

pub use gpui_component::button::{ButtonVariant, ButtonVariants, DropdownButton};
pub use gpui_component::scroll::ScrollableElement;
pub use gpui_component::text::{
    SelectMode, TextSelectionHandle, active_text_selection, select_mode_for_click_count,
};
pub use gpui_component::text_selection::{
    CharType, ceil_char_boundary, char_cell_hit_x, floor_char_boundary, logical_line_range,
    word_range,
};
pub use gpui_component::{ActiveTheme, Disableable, Selectable, Sizable, WindowExt};
pub use gpui_component::{Icon, IconName};

#[cfg(test)]
mod word_range_tests {
    use super::{CharType, ceil_char_boundary, floor_char_boundary, word_range};

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
        // "中文" — 3 bytes each; both are letters so they form one word.
        let s = "中文";
        assert_eq!(word(s, 0).as_deref(), Some("中文"));
        assert_eq!(word(s, 3).as_deref(), Some("中文"));
    }

    #[test]
    fn word_range_hangul_double_click() {
        // The regression: double-clicking Hangul must select the whole word,
        // not one syllable and not nothing. Space-delimited so it stops there.
        let s = "한글 테스트";
        assert_eq!(word(s, 0).as_deref(), Some("한글"));
        assert_eq!(word(s, 3).as_deref(), Some("한글")); // second syllable
        assert_eq!(word(s, 7).as_deref(), Some("테스트"));
    }

    #[test]
    fn char_boundary_helpers_round_to_glyph() {
        let s = "한A"; // 한 = bytes 0..3, A = byte 3
        assert_eq!(floor_char_boundary(s, 2), 0);
        assert_eq!(floor_char_boundary(s, 3), 3);
        assert_eq!(ceil_char_boundary(s, 1), 3);
        assert_eq!(ceil_char_boundary(s, 3), 3);
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
        // CJK / Hangul are letters → Word (so double-click selects them).
        assert_eq!(CharType::from_char('中'), CharType::Word);
        assert_eq!(CharType::from_char('글'), CharType::Word);
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

    /// SelectMode variants must match their click counts.
    /// Calls the real `select_mode_for_click_count` production function.
    #[test]
    fn select_mode_from_click_count() {
        assert_eq!(select_mode_for_click_count(1), SelectMode::Character);
        assert_eq!(select_mode_for_click_count(2), SelectMode::Word);
        assert_eq!(select_mode_for_click_count(3), SelectMode::Line);
        assert_eq!(select_mode_for_click_count(4), SelectMode::All);
        assert_eq!(select_mode_for_click_count(10), SelectMode::All);
    }

    /// Word/Line/All modes signal "has selection" even without a drag.
    /// Verified via select_mode_for_click_count — same mapping used at runtime.
    #[test]
    fn word_line_all_expand_without_drag() {
        let expands = |n: usize| !matches!(select_mode_for_click_count(n), SelectMode::Character);
        assert!(!expands(1)); // Character — only expands on drag
        assert!(expands(2)); // Word
        assert!(expands(3)); // Line
        assert!(expands(4)); // All
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

    /// Logical line range: `logical_line_range` returns the full newline-
    /// delimited line containing `offset`.  Calls the real production function.
    #[test]
    fn logical_line_range_returns_correct_line() {
        let text = "hello\nworld\nfoo";
        let char_at = |i: usize| str_char_at(text, i);
        let len = text.len();

        // Offset 2 in "hello" → line 0..5.
        assert_eq!(logical_line_range(len, char_at, 2), 0..5);
        // Offset 0 (start of text) → line 0..5.
        assert_eq!(logical_line_range(len, char_at, 0), 0..5);
        // Offset 7 in "world" → line 6..11.
        assert_eq!(logical_line_range(len, char_at, 7), 6..11);
        // Offset 12 in "foo" → line 12..15.
        assert_eq!(logical_line_range(len, char_at, 12), 12..15);
        // End of text (last line, no trailing newline) → line 12..15.
        assert_eq!(logical_line_range(len, char_at, 14), 12..15);
    }

    /// On text with no newlines `logical_line_range` always returns 0..len.
    #[test]
    fn logical_line_range_single_line_text() {
        let text = "hello world";
        let char_at = |i: usize| str_char_at(text, i);
        assert_eq!(logical_line_range(text.len(), char_at, 0), 0..11);
        assert_eq!(logical_line_range(text.len(), char_at, 5), 0..11);
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
