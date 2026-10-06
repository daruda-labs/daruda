use ghostty_vt::{Rgb, Terminal};

#[test]
fn osc_4_updates_palette_for_standard_and_bright_colors() {
    let mut t = Terminal::new(2, 1).unwrap();
    t.feed(b"\x1b]4;1;#010203\x07").unwrap();
    t.feed(b"\x1b]4;9;#040506\x07").unwrap();

    t.feed(b"\x1b[31mX").unwrap();
    let styles = t.dump_viewport_row_cell_styles(0).unwrap();
    assert_eq!(
        styles[0].fg,
        Rgb {
            r: 0x01,
            g: 0x02,
            b: 0x03
        }
    );

    t.feed(b"\x1b[0m\x1b[91mX").unwrap();
    let styles = t.dump_viewport_row_cell_styles(0).unwrap();
    assert_eq!(
        styles[1].fg,
        Rgb {
            r: 0x04,
            g: 0x05,
            b: 0x06
        }
    );
}

#[test]
fn osc_4_updates_palette_for_background_colors() {
    let mut t = Terminal::new(2, 1).unwrap();
    t.feed(b"\x1b]4;1;#070809\x07").unwrap();
    t.feed(b"\x1b]4;9;#0a0b0c\x07").unwrap();

    t.feed(b"\x1b[41mX").unwrap();
    let styles = t.dump_viewport_row_cell_styles(0).unwrap();
    assert_eq!(
        styles[0].bg,
        Rgb {
            r: 0x07,
            g: 0x08,
            b: 0x09
        }
    );

    t.feed(b"\x1b[0m\x1b[101mX").unwrap();
    let styles = t.dump_viewport_row_cell_styles(0).unwrap();
    assert_eq!(
        styles[1].bg,
        Rgb {
            r: 0x0A,
            g: 0x0B,
            b: 0x0C
        }
    );
}

#[test]
fn osc_104_resets_palette_entries() {
    let mut t = Terminal::new(1, 1).unwrap();
    t.feed(b"\x1b]4;1;#010203\x07").unwrap();

    t.feed(b"\x1b[31mX").unwrap();
    let styles = t.dump_viewport_row_cell_styles(0).unwrap();
    assert_eq!(
        styles[0].fg,
        Rgb {
            r: 0x01,
            g: 0x02,
            b: 0x03
        }
    );

    t.feed(b"\x1b]104;1\x07").unwrap();
    t.feed(b"\x1b[0m\x1b[31mX").unwrap();
    let styles = t.dump_viewport_row_cell_styles(0).unwrap();
    assert_eq!(
        styles[0].fg,
        Rgb {
            r: 0xCC,
            g: 0x66,
            b: 0x66
        }
    );
}
