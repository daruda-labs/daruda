#[test]
fn scrollback_jump_top_and_bottom() {
    let mut t = ghostty_vt::Terminal::new(20, 5).unwrap();

    for i in 0..50 {
        t.feed(format!("line-{i:02}\r\n").as_bytes()).unwrap();
    }

    let bottom = t.dump_viewport().unwrap();
    assert!(bottom.contains("line-49"));
    assert!(!bottom.contains("line-00"));

    t.scroll_viewport_top().unwrap();
    let top = t.dump_viewport().unwrap();
    assert!(top.contains("line-00"));

    t.scroll_viewport_bottom().unwrap();
    let bottom_again = t.dump_viewport().unwrap();
    assert!(bottom_again.contains("line-49"));
}

#[test]
fn resize_does_not_break_dump_or_feed() {
    let mut t = ghostty_vt::Terminal::new(10, 3).unwrap();
    t.feed(b"hello\r\nworld\r\n").unwrap();
    t.resize(30, 10).unwrap();
    t.feed(b"after-resize\r\n").unwrap();

    let s = t.dump_viewport().unwrap();
    assert!(s.contains("after-resize"));
}

#[test]
fn scrollback_budget_is_measured_in_rows() {
    // A wide grid makes each row costly, so a byte budget would keep fewer
    // than 1,024 rows. Line pruning is page-granular, so the exact count can
    // fall slightly below the request.
    let mut terminal = ghostty_vt::Terminal::with_scrollback(200, 3, 2048).unwrap();
    for _ in 0..4096 {
        terminal.feed(b"0123456789\r\n").unwrap();
    }
    let total = terminal.total_rows();
    assert!(
        (1536..4096).contains(&total),
        "the ring must retain about the requested 2,048 rows and still prune, got {total}"
    );
}
