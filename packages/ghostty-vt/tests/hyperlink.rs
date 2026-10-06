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

#[test]
fn url_ids_align_per_char_with_text_dump() {
    let mut term = Terminal::new(80, 24).unwrap();
    // A wide CJK char (2 cells) followed by an ASCII char that alone
    // carries the OSC 8 link. The id array must align 1:1 with the text
    // dump's chars — not the physical cells — so the link lands on 'x'
    // and not on the CJK char or its spacer.
    term.feed("\u{AC00}".as_bytes()).unwrap();
    term.feed(b"\x1b]8;;https://example.com\x07x\x1b]8;;\x07")
        .unwrap();

    let text = term.dump_screen_row(0).unwrap();
    let ids = term.dump_screen_row_url_ids(0).unwrap();
    let chars: Vec<char> = text.chars().collect();
    assert_eq!(chars, vec!['\u{AC00}', 'x']);
    assert_eq!(
        ids.len(),
        chars.len(),
        "one id per emitted char (not per physical cell)"
    );
    assert_eq!(ids[0], 0, "the CJK char carries no link");
    assert_ne!(ids[1], 0, "the link must land on the ASCII char");
}

#[test]
fn url_ids_trim_trailing_blanks_like_text() {
    let mut term = Terminal::new(80, 24).unwrap();
    // Link on the first char, then trailing blanks fill the row. The id
    // array must trim trailing blanks exactly as the text dump does.
    term.feed(b"\x1b]8;;https://example.com\x07a\x1b]8;;\x07b")
        .unwrap();

    let text = term.dump_screen_row(0).unwrap();
    let ids = term.dump_screen_row_url_ids(0).unwrap();
    assert_eq!(text, "ab");
    assert_eq!(ids.len(), 2, "trailing blanks must not produce ids");
    assert_ne!(ids[0], 0, "the link must land on 'a'");
    assert_eq!(ids[1], 0, "'b' is outside the link");
}
