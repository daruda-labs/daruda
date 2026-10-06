use ghostty_vt::Terminal;

#[test]
fn dec_special_graphics_maps_acs_line_drawing() {
    let mut t = Terminal::new(4, 1).unwrap();

    // Designate G0 as DEC Special Graphics, then print "q" which maps to "─".
    t.feed(b"\x1b(0q").unwrap();
    let row = t.dump_viewport_row(0).unwrap();
    assert!(row.contains('─'), "row={row:?}");

    // Switch back to ASCII and ensure "q" prints literally.
    t.feed(b"\x1b(Bq").unwrap();
    let row = t.dump_viewport_row(0).unwrap();
    assert!(row.contains('q'), "row={row:?}");
}
