#[test]
fn viewport_dump_contains_text() {
    let mut t = ghostty_vt::Terminal::new(80, 24).unwrap();
    t.feed(b"hello\r\nworld\r\n").unwrap();
    t.feed(b"\x1b[31mred\x1b[0m\r\n").unwrap();

    let s = t.dump_viewport().unwrap();
    assert!(s.contains("hello"));
    assert!(s.contains("world"));
    assert!(s.contains("red"));
}

#[test]
fn viewport_row_dump_contains_text() {
    let mut t = ghostty_vt::Terminal::new(80, 24).unwrap();
    t.feed(b"hello\r\n").unwrap();

    let row0 = t.dump_viewport_row(0).unwrap();
    assert!(row0.contains("hello"));
}
