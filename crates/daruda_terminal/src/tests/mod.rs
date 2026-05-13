use gpui::{KeyBinding, KeyContext, Keymap, Keystroke, actions};
use std::any::TypeId;

use crate::{TerminalConfig, TerminalSession};

actions!(tab_shadow_test, [RootTab, TerminalTab]);

fn osc_color_response(ps: u8, (r, g, b): (u8, u8, u8)) -> String {
    let r16 = u16::from(r) * 0x0101;
    let g16 = u16::from(g) * 0x0101;
    let b16 = u16::from(b) * 0x0101;

    format!("\x1b]{};rgb:{:04x}/{:04x}/{:04x}\x1b\\", ps, r16, g16, b16)
}

fn viewport_index_for_cell(viewport: &str, row: u16, col: u16) -> usize {
    let row = row.max(1) as usize;
    let col = col.max(1) as usize;

    use unicode_width::UnicodeWidthChar as _;

    let mut current_row = 1usize;
    let mut offset = 0usize;

    for segment in viewport.split_inclusive('\n') {
        let line = segment.strip_suffix('\n').unwrap_or(segment);

        if current_row == row {
            if col == 1 {
                return offset;
            }

            let mut current_col = 1usize;
            for (byte_index, ch) in line.char_indices() {
                let width = ch.width().unwrap_or(0);
                if width == 0 {
                    continue;
                }

                if current_col == col {
                    return offset + byte_index;
                }

                let next_col = current_col.saturating_add(width);
                if col < next_col {
                    return offset + byte_index;
                }

                current_col = next_col;
            }

            return offset + line.len();
        }

        offset = offset.saturating_add(segment.len());
        current_row += 1;
    }

    viewport.len()
}

#[test]
fn terminal_tab_binding_shadows_root_tab_binding() {
    let mut keymap = Keymap::default();
    keymap.add_bindings([
        KeyBinding::new("tab", RootTab, Some("Root")),
        KeyBinding::new("tab", TerminalTab, Some("Terminal")),
    ]);

    let mut root = KeyContext::default();
    root.add("Root");
    let mut terminal = KeyContext::default();
    terminal.add("Terminal");

    let (bindings, pending) =
        keymap.bindings_for_input(&[Keystroke::parse("tab").unwrap()], &[root, terminal]);

    assert!(!pending);
    assert_eq!(
        bindings[0].action().as_any().type_id(),
        TypeId::of::<TerminalTab>()
    );
}

#[test]
fn tracks_bracketed_paste_mode_from_output() {
    let mut session = TerminalSession::new(TerminalConfig::default()).unwrap();
    assert!(!session.bracketed_paste_enabled());

    session.feed(b"\x1b[?2004h").unwrap();
    assert!(session.bracketed_paste_enabled());

    session.feed(b"\x1b[?2004l").unwrap();
    assert!(!session.bracketed_paste_enabled());
}

#[test]
fn tracks_mouse_reporting_mode_from_output() {
    let mut session = TerminalSession::new(TerminalConfig::default()).unwrap();
    assert!(!session.mouse_reporting_enabled());
    assert!(!session.mouse_sgr_enabled());

    session.feed(b"\x1b[?1000;1006h").unwrap();
    assert!(session.mouse_reporting_enabled());
    assert!(session.mouse_sgr_enabled());

    session.feed(b"\x1b[?1000l").unwrap();
    assert!(!session.mouse_reporting_enabled());
    assert!(session.mouse_sgr_enabled());

    session.feed(b"\x1b[?1006l").unwrap();
    assert!(!session.mouse_sgr_enabled());
}

#[test]
fn viewport_index_maps_row_and_column_to_byte_index() {
    let viewport = "abc\ndef";
    assert_eq!(viewport_index_for_cell(viewport, 1, 1), 0);
    assert_eq!(viewport_index_for_cell(viewport, 1, 2), 1);
    assert_eq!(viewport_index_for_cell(viewport, 1, 4), 3);
    assert_eq!(viewport_index_for_cell(viewport, 2, 1), 4);
    assert_eq!(viewport_index_for_cell(viewport, 2, 3), 6);
}

#[test]
fn viewport_index_accounts_for_wide_characters() {
    let viewport = "Ｗa\n";
    assert_eq!(viewport_index_for_cell(viewport, 1, 1), 0);
    assert_eq!(viewport_index_for_cell(viewport, 1, 2), 0);
    assert_eq!(viewport_index_for_cell(viewport, 1, 3), "Ｗ".len());
    assert_eq!(viewport_index_for_cell(viewport, 1, 4), "Ｗ".len() + 1);
}

#[test]
fn tracks_modes_across_chunk_boundaries() {
    let mut session = TerminalSession::new(TerminalConfig::default()).unwrap();
    session.feed(b"\x1b[?1000;").unwrap();
    assert!(!session.mouse_reporting_enabled());

    session.feed(b"1006h").unwrap();
    assert!(session.mouse_reporting_enabled());
    assert!(session.mouse_sgr_enabled());
}

#[test]
fn tracks_osc_title_across_chunk_boundaries() {
    let mut session = TerminalSession::new(TerminalConfig::default()).unwrap();
    session.feed(b"\x1b]0;hi").unwrap();
    assert!(session.title().is_none());

    session.feed(b"\x07").unwrap();
    assert_eq!(session.title(), Some("hi"));
}

#[test]
fn tracks_osc_52_clipboard_across_chunk_boundaries() {
    let mut session = TerminalSession::new(TerminalConfig::default()).unwrap();
    session.feed(b"\x1b]52;c;").unwrap();
    assert!(session.take_clipboard_write().is_none());

    session.feed(b"aGk=\x07").unwrap();
    assert_eq!(session.take_clipboard_write().as_deref(), Some("hi"));
}

#[test]
fn responds_to_csi_6n_cursor_position_request() {
    let mut session = TerminalSession::new(TerminalConfig::default()).unwrap();
    let mut response = Vec::new();

    session
        .feed_with_pty_responses(b"hi\x1b[6n", |bytes| {
            response.extend_from_slice(bytes);
        })
        .unwrap();

    assert_eq!(response, b"\x1b[1;3R");
}

#[test]
fn responds_to_csi_6n_across_chunk_boundaries() {
    let mut session = TerminalSession::new(TerminalConfig::default()).unwrap();
    let mut response = Vec::new();

    session
        .feed_with_pty_responses(b"hi\x1b[", |bytes| {
            response.extend_from_slice(bytes);
        })
        .unwrap();
    assert!(response.is_empty());

    session
        .feed_with_pty_responses(b"6n", |bytes| {
            response.extend_from_slice(bytes);
        })
        .unwrap();

    assert_eq!(response, b"\x1b[1;3R");
}

#[test]
fn responds_to_csi_5n_device_status_request() {
    let mut session = TerminalSession::new(TerminalConfig::default()).unwrap();
    let mut response = Vec::new();

    session
        .feed_with_pty_responses(b"\x1b[5n", |bytes| {
            response.extend_from_slice(bytes);
        })
        .unwrap();

    assert_eq!(response, b"\x1b[0n");
}

#[test]
fn responds_to_csi_5n_across_chunk_boundaries() {
    let mut session = TerminalSession::new(TerminalConfig::default()).unwrap();
    let mut response = Vec::new();

    session
        .feed_with_pty_responses(b"\x1b[", |bytes| {
            response.extend_from_slice(bytes);
        })
        .unwrap();
    assert!(response.is_empty());

    session
        .feed_with_pty_responses(b"5n", |bytes| {
            response.extend_from_slice(bytes);
        })
        .unwrap();

    assert_eq!(response, b"\x1b[0n");
}

#[test]
fn responds_to_osc_10_default_foreground_color_query() {
    let mut session = TerminalSession::new(TerminalConfig::default()).unwrap();
    let mut response = Vec::new();

    session
        .feed_with_pty_responses(b"\x1b]10;?\x1b\\", |bytes| {
            response.extend_from_slice(bytes);
        })
        .unwrap();

    let expected = osc_color_response(10, (0xFF, 0xFF, 0xFF));
    assert_eq!(response, expected.as_bytes());
}

#[test]
fn responds_to_osc_11_default_background_color_query() {
    let mut session = TerminalSession::new(TerminalConfig::default()).unwrap();
    let mut response = Vec::new();

    session
        .feed_with_pty_responses(b"\x1b]11;?\x1b\\", |bytes| {
            response.extend_from_slice(bytes);
        })
        .unwrap();

    let expected = osc_color_response(11, (0x00, 0x00, 0x00));
    assert_eq!(response, expected.as_bytes());
}

#[test]
fn responds_to_osc_10_and_11_use_configured_defaults() {
    let config = TerminalConfig {
        default_fg: ghostty_vt::Rgb {
            r: 0x11,
            g: 0x22,
            b: 0x33,
        },
        default_bg: ghostty_vt::Rgb {
            r: 0x44,
            g: 0x55,
            b: 0x66,
        },
        ..TerminalConfig::default()
    };
    let mut session = TerminalSession::new(config).unwrap();
    let mut response = Vec::new();

    session
        .feed_with_pty_responses(b"\x1b]10;?\x1b\\\x1b]11;?\x1b\\", |bytes| {
            response.extend_from_slice(bytes);
        })
        .unwrap();

    let expected_fg = osc_color_response(10, (0x11, 0x22, 0x33));
    let expected_bg = osc_color_response(11, (0x44, 0x55, 0x66));
    let mut expected = Vec::new();
    expected.extend_from_slice(expected_fg.as_bytes());
    expected.extend_from_slice(expected_bg.as_bytes());
    assert_eq!(response, expected);
}

#[test]
fn responds_to_osc_11_across_chunk_boundaries() {
    let mut session = TerminalSession::new(TerminalConfig::default()).unwrap();
    let mut response = Vec::new();

    session
        .feed_with_pty_responses(b"\x1b]11;?\x1b", |bytes| {
            response.extend_from_slice(bytes);
        })
        .unwrap();
    assert!(response.is_empty());

    session
        .feed_with_pty_responses(b"\\", |bytes| {
            response.extend_from_slice(bytes);
        })
        .unwrap();

    let expected = osc_color_response(11, (0x00, 0x00, 0x00));
    assert_eq!(response, expected.as_bytes());
}

#[test]
fn responds_to_osc_11_query_terminated_by_bel() {
    let mut session = TerminalSession::new(TerminalConfig::default()).unwrap();
    let mut response = Vec::new();

    session
        .feed_with_pty_responses(b"\x1b]11;?\x07", |bytes| {
            response.extend_from_slice(bytes);
        })
        .unwrap();

    let expected = osc_color_response(11, (0x00, 0x00, 0x00));
    assert_eq!(response, expected.as_bytes());
}

#[test]
fn sgr_mouse_encoding_helpers_match_expected_format() {
    assert_eq!(
        crate::view::sgr_mouse_button_value(0, false, false, false, false),
        0
    );
    assert_eq!(
        crate::view::sgr_mouse_button_value(2, true, false, true, true),
        2 + 32 + 8 + 16
    );
    assert_eq!(
        crate::view::sgr_mouse_sequence(0, 1, 1, true),
        "\u{1b}[<0;1;1M"
    );
    assert_eq!(
        crate::view::sgr_mouse_sequence(0, 1, 1, false),
        "\u{1b}[<0;1;1m"
    );
}

#[test]
fn ctrl_c_encodes_to_etx_even_without_key_char() {
    let ctrl_c = Keystroke::parse("ctrl-c").unwrap();
    assert_eq!(crate::view::ctrl_byte_for_keystroke(&ctrl_c), Some(0x03));
}

#[test]
fn does_not_skip_enter_key_when_ime_in_progress() {
    let enter = Keystroke::parse("enter").unwrap();
    assert!(enter.is_ime_in_progress());
    assert!(!crate::view::should_skip_key_down_for_ime(
        true, false, &enter
    ));

    let letter = Keystroke::parse("a").unwrap();
    assert!(letter.is_ime_in_progress());
    assert!(crate::view::should_skip_key_down_for_ime(
        true, false, &letter
    ));

    let committed = Keystroke::parse("a->a").unwrap();
    assert!(!committed.is_ime_in_progress());
    assert!(!crate::view::should_skip_key_down_for_ime(
        true, false, &committed
    ));

    // With marked text, skip printable keys but not escape/backspace
    assert!(crate::view::should_skip_key_down_for_ime(
        true, true, &letter
    ));
    assert!(!crate::view::should_skip_key_down_for_ime(
        true, true, &enter
    ));

    // Non-ASCII key_char (e.g. Korean ㅎ after switching input source)
    // must be skipped so the IME pipeline handles it
    let korean = Keystroke::parse("h->ㅎ").unwrap();
    assert!(!korean.is_ime_in_progress()); // key_char is set
    assert!(crate::view::should_skip_key_down_for_ime(
        true, false, &korean
    ));
    assert!(crate::view::should_skip_key_down_for_ime(
        false, false, &korean
    ));

    // ASCII committed key must NOT be skipped
    assert!(!crate::view::should_skip_key_down_for_ime(
        true, false, &committed
    ));
}

#[test]
fn byte_index_for_column_in_line_handles_wide_characters() {
    assert_eq!(crate::view::byte_index_for_column_in_line("Ｗa", 1), 0);
    assert_eq!(crate::view::byte_index_for_column_in_line("Ｗa", 2), 0);
    assert_eq!(
        crate::view::byte_index_for_column_in_line("Ｗa", 3),
        "Ｗ".len()
    );
    assert_eq!(
        crate::view::byte_index_for_column_in_line("Ｗa", 4),
        "Ｗ".len() + 1
    );
}

#[test]
fn byte_index_for_column_handles_wide_chars() {
    // `shaped_pixel_range_for_cols` depends on
    // `byte_index_for_column_in_line` producing a byte offset that
    // can be fed to GPUI's ShapedLine. Wide chars (CJK/emoji) must
    // map to their UTF-8 start byte, not to the "middle" column.
    use crate::view::byte_index_for_column_in_line;

    // "한글" — each Hangul char has display width 2.
    let line = "한글";
    // Col 1 → byte 0 ("한" start).
    assert_eq!(byte_index_for_column_in_line(line, 1), 0);
    // Col 2 → still inside "한", so byte 0 (shaped line treats this
    // as same glyph anyway).
    assert_eq!(byte_index_for_column_in_line(line, 2), 0);
    // Col 3 → "글" start.
    assert_eq!(byte_index_for_column_in_line(line, 3), "한".len());
    // Col 4 → still inside "글", so byte offset for "글".
    assert_eq!(byte_index_for_column_in_line(line, 4), "한".len());
    // Col 5 → past the last char → line.len().
    assert_eq!(byte_index_for_column_in_line(line, 5), line.len());
}

#[test]
fn byte_index_for_column_handles_mixed_narrow_and_wide() {
    use crate::view::byte_index_for_column_in_line;

    // "a한b" — narrow, wide, narrow.
    let line = "a한b";
    assert_eq!(byte_index_for_column_in_line(line, 1), 0); // 'a'
    assert_eq!(byte_index_for_column_in_line(line, 2), 1); // '한' start
    assert_eq!(byte_index_for_column_in_line(line, 3), 1); // inside '한'
    assert_eq!(byte_index_for_column_in_line(line, 4), 1 + "한".len()); // 'b'
}

#[test]
fn maps_common_box_drawing_glyphs() {
    for ch in ['─', '│', '┌', '┐', '└', '┘', '├', '┤', '┬', '┴', '┼'] {
        assert!(
            crate::view::box_drawing_mask(ch).is_some(),
            "expected mask for {ch}"
        );
    }
    assert!(crate::view::box_drawing_mask('X').is_none());
}

#[test]
fn line_has_box_drawing_detects_glyphs() {
    assert!(crate::view::line_has_box_drawing("┌── header ──┐"));
    assert!(crate::view::line_has_box_drawing("status │ ok"));
}

#[test]
fn search_finds_matches_in_scrollback() {
    use crate::{TerminalConfig, TerminalSession};
    let mut session = TerminalSession::new(TerminalConfig::default()).unwrap();
    // Drop a keyword into the scrollback region.
    for i in 0..40 {
        let token = if i == 2 { "NEEDLE" } else { "filler" };
        session
            .feed(format!("line-{:02}-{}\r\n", i, token).as_bytes())
            .unwrap();
    }
    // Walk the screen rows and count NEEDLE occurrences — proves the
    // scrollback-aware dump works end-to-end for search input.
    let total = session.total_rows();
    let mut hits = 0;
    for y in 0..total {
        if let Ok(line) = session.dump_screen_row(y)
            && line.contains("NEEDLE")
        {
            hits += 1;
        }
    }
    assert_eq!(hits, 1, "scrollback search must find the single NEEDLE");
}

#[test]
fn dump_screen_row_returns_scrollback_content() {
    let mut session = TerminalSession::new(TerminalConfig::default()).unwrap();
    // Fill more rows than the viewport to force scrollback (default
    // rows=24). 30 lines pushes 6 of them off the top.
    for i in 0..30 {
        session
            .feed(format!("line-{:02}\r\n", i).as_bytes())
            .unwrap();
    }
    // `total_rows` should exceed the viewport capacity now.
    assert!(session.total_rows() >= 30);
    // The very first scrolled-out line must still be readable via the
    // screen dump.
    let row0 = session.dump_screen_row(0).unwrap();
    assert!(
        row0.contains("line-00"),
        "expected first scrollback row to contain 'line-00'; got {row0:?}"
    );
}

#[test]
fn line_has_box_drawing_skips_plain_text() {
    assert!(!crate::view::line_has_box_drawing(""));
    assert!(!crate::view::line_has_box_drawing(
        "$ cargo build --release"
    ));
    assert!(!crate::view::line_has_box_drawing(
        "error[E0499]: cannot borrow 'x' as mutable"
    ));
    // Pipe character (ASCII 0x7C) is NOT a box drawing glyph.
    assert!(!crate::view::line_has_box_drawing("foo | bar"));
    // Korean / CJK characters must not trigger the box path.
    assert!(!crate::view::line_has_box_drawing("한글 테스트 line"));
}

// ---------------------------------------------------------------------------
// vi editing reconciliation — documents ghostty_vt semantics that the
// `reconcile_dirty_viewport_after_output` fallback relies on. If these break,
// the Alacritty/iTerm2-style conservative refresh in view/mod.rs may need to
// be revisited.
// ---------------------------------------------------------------------------

fn place_cursor_at(session: &mut TerminalSession, row: u16, col: u16) {
    let seq = format!("\x1b[{};{}H", row, col);
    session.feed(seq.as_bytes()).unwrap();
}

fn drain_scroll_and_dirty(session: &mut TerminalSession) {
    let _ = session.take_viewport_scroll_delta();
    let _ = session.take_dirty_viewport_rows();
}

// These tests codify ghostty_vt's *observed* dirty-row / scroll-delta
// semantics that the reconcile fallback in view/mod.rs depends on. If a
// ghostty upgrade changes any of these, the Alacritty/iTerm2-style
// conservative refresh path may need to be revisited.

#[test]
fn linefeed_at_bottom_margin_reports_scroll_delta_and_dirty_rows() {
    // This is the real-world "vi `o` on last line" path: vi does not emit
    // IND/SU itself — it just writes `\n` and relies on the terminal to
    // scroll. The reconcile path must react via `take_viewport_scroll_delta`.
    let mut session = TerminalSession::new(TerminalConfig::default()).unwrap();
    let rows = session.rows();

    place_cursor_at(&mut session, rows, 1);
    drain_scroll_and_dirty(&mut session);

    session.feed(b"\n").unwrap();

    let delta = session.take_viewport_scroll_delta();
    assert!(
        delta > 0,
        "LF at bottom margin must produce positive scroll delta; got {delta}"
    );

    let dirty = session.take_dirty_viewport_rows();
    assert!(
        dirty.iter().any(|&r| r == rows - 1),
        "new bottom row must be dirty after LF scroll; got {dirty:?}"
    );
}

#[test]
fn plain_character_output_keeps_dirty_set_small() {
    // Sanity check: single-character writes must stay under the rows/2
    // threshold so normal typing uses the partial-update fast path.
    let mut session = TerminalSession::new(TerminalConfig::default()).unwrap();
    let rows = session.rows();

    place_cursor_at(&mut session, 5, 1);
    drain_scroll_and_dirty(&mut session);

    session.feed(b"x").unwrap();

    assert_eq!(session.take_viewport_scroll_delta(), 0);
    let dirty = session.take_dirty_viewport_rows();
    assert!(
        dirty.len() * 2 < rows as usize,
        "plain output must stay under rows/2 dirty; got {} / {rows}",
        dirty.len()
    );
}

#[test]
fn erase_display_marks_every_viewport_row_dirty() {
    // ED (CSI 2 J) must mark the whole viewport dirty, which saturates the
    // `dirty.len() * 2 >= rows` heuristic and forces a full refresh.
    let mut session = TerminalSession::new(TerminalConfig::default()).unwrap();
    let rows = session.rows();

    session.feed(b"seed\r\n").unwrap();
    drain_scroll_and_dirty(&mut session);

    session.feed(b"\x1b[2J").unwrap();

    let dirty = session.take_dirty_viewport_rows();
    assert_eq!(
        dirty.len(),
        rows as usize,
        "ED must dirty every row; got {dirty:?}"
    );
    assert!(dirty.len() * 2 >= rows as usize);
}

#[test]
fn insert_line_marks_cursor_through_bottom_margin_dirty() {
    // Covers vi's shift-down path via IL (CSI L). With the Handler fix in
    // ghostty_vt_sys, markDirty now propagates to `take_dirty_viewport_rows`.
    // Since `rem = bottom - cursor.y + 1` rows are marked, the
    // `dirty.len() * 2 >= rows` heuristic will trigger a full refresh.
    let mut session = TerminalSession::new(TerminalConfig::default()).unwrap();
    let rows = session.rows();

    session.feed(b"a\r\nb\r\nc\r\n").unwrap();
    place_cursor_at(&mut session, 2, 1);
    drain_scroll_and_dirty(&mut session);

    session.feed(b"\x1b[1L").unwrap();

    let delta = session.take_viewport_scroll_delta();
    assert_eq!(delta, 0, "IL does not move viewport pin");

    let dirty = session.take_dirty_viewport_rows();
    let cursor_row0 = 1u16;
    let bottom0 = rows - 1;
    assert!(
        dirty.contains(&cursor_row0),
        "cursor row must be dirty after IL; got {dirty:?}"
    );
    assert!(
        dirty.contains(&bottom0),
        "bottom margin must be dirty after IL (shift-down propagates); got {dirty:?}"
    );
    assert!(
        dirty.len() * 2 >= rows as usize,
        "IL must saturate the rows/2 refresh heuristic; got {} / {rows}",
        dirty.len()
    );
}

#[test]
fn delete_line_marks_cursor_through_bottom_margin_dirty() {
    let mut session = TerminalSession::new(TerminalConfig::default()).unwrap();
    let rows = session.rows();

    session.feed(b"a\r\nb\r\nc\r\n").unwrap();
    place_cursor_at(&mut session, 2, 1);
    drain_scroll_and_dirty(&mut session);

    session.feed(b"\x1b[1M").unwrap();

    assert_eq!(session.take_viewport_scroll_delta(), 0);
    let dirty = session.take_dirty_viewport_rows();
    assert!(
        dirty.len() * 2 >= rows as usize,
        "DL must saturate the rows/2 refresh heuristic; got {} / {rows}",
        dirty.len()
    );
}

#[test]
fn reverse_index_at_top_margin_reports_scroll_or_dirty() {
    // RI (ESC M) at top row should either produce a scroll delta or at
    // minimum mark rows dirty. Guards against regressions in the reverseIndex
    // FFI handler.
    let mut session = TerminalSession::new(TerminalConfig::default()).unwrap();
    let rows = session.rows();

    session.feed(b"content\r\n").unwrap();
    place_cursor_at(&mut session, 1, 1);
    drain_scroll_and_dirty(&mut session);

    session.feed(b"\x1bM").unwrap();

    let delta = session.take_viewport_scroll_delta();
    let dirty = session.take_dirty_viewport_rows();
    assert!(
        delta != 0 || !dirty.is_empty(),
        "RI at top must be observable; delta={delta} dirty={dirty:?} rows={rows}"
    );
}

mod ffi_handlers;
