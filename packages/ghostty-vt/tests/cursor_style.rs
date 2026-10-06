use ghostty_vt::Terminal;

#[test]
fn full_reset_restores_the_default_cursor_style() {
    let mut term = Terminal::new(10, 3).expect("terminal");
    term.feed(b"\x1b[5 q").expect("feed DECSCUSR");
    assert_eq!(term.cursor_style(), 5);
    term.feed(b"\x1bc").expect("feed RIS");
    assert_eq!(term.cursor_style(), 0);
}
