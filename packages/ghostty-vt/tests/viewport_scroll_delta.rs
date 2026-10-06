use ghostty_vt::Terminal;

#[test]
fn viewport_scroll_delta_tracks_output_scrolling() {
    let mut terminal = Terminal::new(5, 2).unwrap();

    assert_eq!(terminal.take_viewport_scroll_delta(), 0);

    terminal.feed(b"1\n2\n3\n").unwrap();
    let delta = terminal.take_viewport_scroll_delta();
    assert!(delta > 0);

    assert_eq!(terminal.take_viewport_scroll_delta(), 0);
}
