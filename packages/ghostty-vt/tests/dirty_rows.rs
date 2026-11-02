use ghostty_vt::Terminal;

#[test]
fn dirty_rows_are_cleared_after_take() {
    let mut terminal = Terminal::new(80, 24).unwrap();

    let _ = terminal.take_dirty_viewport_rows(24).unwrap();

    terminal.feed(b"hi").unwrap();
    let dirty = terminal.take_dirty_viewport_rows(24).unwrap();
    assert!(dirty.contains(&0));

    let dirty = terminal.take_dirty_viewport_rows(24).unwrap();
    assert!(dirty.is_empty());

    terminal.feed(b"x").unwrap();
    let dirty = terminal.take_dirty_viewport_rows(24).unwrap();
    assert!(dirty.contains(&0));
}

/// Some scroll paths mark a whole page dirty instead of single rows; one take
/// must consume that flag so later frames do not keep reporting every row.
#[test]
fn page_dirty_flags_are_cleared_after_take() {
    let no_history = Terminal::with_scrollback(80, 24, 0).unwrap();
    let scroll_region = Terminal::new(80, 24).unwrap();
    for (name, mut terminal, setup) in [
        ("no scrollback", no_history, &b""[..]),
        ("scroll region", scroll_region, &b"\x1b[2;20r"[..]),
    ] {
        terminal.feed(setup).unwrap();
        for _ in 0..100 {
            terminal.feed(b"line\r\n").unwrap();
        }
        assert!(!terminal.take_dirty_viewport_rows(24).unwrap().is_empty());

        let dirty = terminal.take_dirty_viewport_rows(24).unwrap();
        assert!(dirty.is_empty(), "{name}: rows stayed dirty: {dirty:?}");
    }
}
