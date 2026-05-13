use super::*;

// ---------------------------------------------------------------------------
// FFI handler dispatch — each test below targets one handler in
// crates/ghostty_vt_sys/zig/lib.zig and proves the sequence actually reaches
// the terminal. `ghostty.Stream` uses `@hasDecl` checks so a missing handler
// fails silently; these tests guard against that regression.
// ---------------------------------------------------------------------------

fn row_text(session: &TerminalSession, row0: u16) -> String {
    session
        .dump_viewport_row(row0)
        .unwrap()
        .trim_end_matches('\n')
        .trim_end()
        .to_string()
}

#[test]
fn ffi_handler_index_advances_cursor_and_scrolls_at_bottom() {
    // IND (ESC D) — advances cursor, scrolls when at bottom margin.
    let mut session = TerminalSession::new(TerminalConfig::default()).unwrap();
    let rows = session.rows();

    place_cursor_at(&mut session, 2, 1);
    let (_, y_before) = session.cursor_position().unwrap();
    session.feed(b"\x1bD").unwrap();
    let (_, y_after) = session.cursor_position().unwrap();
    assert_eq!(y_after, y_before + 1, "IND mid-screen advances cursor");

    place_cursor_at(&mut session, rows, 1);
    session.feed(b"content").unwrap();
    drain_scroll_and_dirty(&mut session);
    place_cursor_at(&mut session, rows, 1);
    session.feed(b"\x1bD").unwrap();
    assert!(
        session.take_viewport_scroll_delta() > 0,
        "IND at bottom margin must scroll viewport"
    );
}

#[test]
fn ffi_handler_scroll_up_cs_s_shifts_content() {
    // SU (CSI S) — shift viewport up by 1.
    let mut session = TerminalSession::new(TerminalConfig::default()).unwrap();

    session.feed(b"row1\r\nrow2\r\nrow3\r\n").unwrap();
    drain_scroll_and_dirty(&mut session);

    session.feed(b"\x1b[S").unwrap();

    let delta = session.take_viewport_scroll_delta();
    let dirty = session.take_dirty_viewport_rows();
    assert!(
        delta != 0 || !dirty.is_empty(),
        "SU must produce observable change; delta={delta} dirty={dirty:?}"
    );
    assert_eq!(
        row_text(&session, 0),
        "row2",
        "row1 should have scrolled out"
    );
}

#[test]
fn ffi_handler_scroll_down_cs_t_shifts_content() {
    // SD (CSI T) — shift viewport down by 1.
    let mut session = TerminalSession::new(TerminalConfig::default()).unwrap();

    session.feed(b"row1\r\nrow2\r\n").unwrap();
    drain_scroll_and_dirty(&mut session);

    session.feed(b"\x1b[T").unwrap();

    assert_eq!(
        row_text(&session, 0),
        "",
        "row 0 must be blank after scroll down"
    );
    assert_eq!(
        row_text(&session, 1),
        "row1",
        "row1 should have shifted down one line"
    );
}

#[test]
fn ffi_handler_insert_blanks_cs_at() {
    // ICH (CSI @) — insert N blank cells at cursor, shifting right.
    let mut session = TerminalSession::new(TerminalConfig::default()).unwrap();

    session.feed(b"abcdef").unwrap();
    place_cursor_at(&mut session, 1, 2); // cursor on 'b'
    session.feed(b"\x1b[2@").unwrap();

    assert_eq!(row_text(&session, 0), "a  bcdef", "ICH inserted 2 blanks");
}

#[test]
fn ffi_handler_delete_chars_cs_p() {
    // DCH (CSI P) — delete N cells at cursor, shifting left.
    let mut session = TerminalSession::new(TerminalConfig::default()).unwrap();

    session.feed(b"abcdef").unwrap();
    place_cursor_at(&mut session, 1, 2); // cursor on 'b'
    session.feed(b"\x1b[2P").unwrap();

    assert_eq!(row_text(&session, 0), "adef", "DCH removed 2 chars");
}

#[test]
fn ffi_handler_erase_chars_cs_x() {
    // ECH (CSI X) — replace N cells with blanks, no shift.
    let mut session = TerminalSession::new(TerminalConfig::default()).unwrap();

    session.feed(b"abcdef").unwrap();
    place_cursor_at(&mut session, 1, 2);
    session.feed(b"\x1b[2X").unwrap();

    assert_eq!(
        row_text(&session, 0),
        "a  def",
        "ECH erased 2 cells in place"
    );
}

#[test]
fn ffi_handler_decstbm_confines_il_to_scroll_region() {
    // DECSTBM (CSI r) — set scroll region. IL outside the region must no-op,
    // inside the region must shift. Proves setTopAndBottomMargin dispatches.
    let mut session = TerminalSession::new(TerminalConfig::default()).unwrap();

    session.feed(b"l1\r\nl2\r\nl3\r\nl4\r\nl5\r\n").unwrap();
    session.feed(b"\x1b[2;4r").unwrap(); // scroll region: rows 2..=4

    // IL outside region (row 5) → no-op.
    place_cursor_at(&mut session, 5, 1);
    session.feed(b"\x1b[L").unwrap();
    assert_eq!(row_text(&session, 4), "l5", "IL outside STBM must no-op");

    // IL at top of region (row 2) → shifts l2/l3 down, row 4 drops out of region.
    place_cursor_at(&mut session, 2, 1);
    session.feed(b"\x1b[L").unwrap();
    assert_eq!(row_text(&session, 0), "l1", "row 1 unchanged");
    assert_eq!(row_text(&session, 1), "", "row 2 became blank");
    assert_eq!(row_text(&session, 2), "l2", "l2 shifted to row 3");
    assert_eq!(row_text(&session, 3), "l3", "l3 shifted to row 4");
}

#[test]
fn ffi_handler_save_restore_cursor() {
    // DECSC / DECRC (ESC 7 / ESC 8) — round-trip cursor position.
    let mut session = TerminalSession::new(TerminalConfig::default()).unwrap();

    place_cursor_at(&mut session, 5, 10);
    session.feed(b"\x1b7").unwrap(); // save
    place_cursor_at(&mut session, 1, 1);
    session.feed(b"\x1b8").unwrap(); // restore

    let (x, y) = session.cursor_position().unwrap();
    assert_eq!((x, y), (10, 5), "cursor restored to saved (col=10, row=5)");
}

#[test]
fn ffi_handler_next_line_esc_e() {
    // NEL (ESC E) — CR + LF combined.
    let mut session = TerminalSession::new(TerminalConfig::default()).unwrap();

    place_cursor_at(&mut session, 2, 5);
    session.feed(b"\x1bE").unwrap();

    let (x, y) = session.cursor_position().unwrap();
    assert_eq!(x, 1, "NEL resets column to 1 (1-indexed)");
    assert_eq!(y, 3, "NEL advances row to 3 (1-indexed)");
}

#[test]
fn ffi_handler_print_repeat_esc_b() {
    // REP (CSI b) — repeat last printed character N times.
    let mut session = TerminalSession::new(TerminalConfig::default()).unwrap();

    session.feed(b"a\x1b[3b").unwrap();
    assert_eq!(
        row_text(&session, 0),
        "aaaa",
        "REP repeats 'a' 3 more times"
    );
}

#[test]
fn ffi_handler_full_reset_esc_c() {
    // RIS (ESC c) — full reset. Must clear screen and reset cursor.
    let mut session = TerminalSession::new(TerminalConfig::default()).unwrap();

    session.feed(b"some text\r\nmore text\r\n").unwrap();
    session.feed(b"\x1bc").unwrap();

    assert_eq!(row_text(&session, 0), "", "row 0 must be empty after RIS");
    assert_eq!(row_text(&session, 1), "", "row 1 must be empty after RIS");
    let (x, y) = session.cursor_position().unwrap();
    assert_eq!(
        (x, y),
        (1, 1),
        "cursor must be at origin (1-indexed) after RIS"
    );
}
