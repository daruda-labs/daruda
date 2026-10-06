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
