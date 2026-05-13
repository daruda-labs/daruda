use ghostty_vt::Terminal;

#[test]
fn hyperlink_at_returns_osc8_uri() {
    let mut term = Terminal::new(80, 24).unwrap();

    term.feed(b"\x1b]8;;https://example.com\x07hi\x1b]8;;\x07")
        .unwrap();

    assert_eq!(
        term.hyperlink_at(1, 1).as_deref(),
        Some("https://example.com")
    );
}
