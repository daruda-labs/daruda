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
