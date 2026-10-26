use super::*;

#[test]
fn release_preserves_modifiers_but_uses_protocol_specific_button() {
    // Right button + Alt + Control: SGR keeps button 2; normal uses 3.
    assert_eq!(
        encode_report(MouseReport::Release(26), 4, 7, true, false),
        b"\x1b[<26;4;7m"
    );
    assert_eq!(
        encode_report(MouseReport::Release(26), 4, 7, false, false),
        b"\x1b[M;$'"
    );
}

#[test]
fn press_motion_and_wheel_keep_their_button_codes() {
    for (button, sgr, normal) in [
        (0, &b"\x1b[<0;1;1M"[..], &b"\x1b[M !!"[..]),
        (58, &b"\x1b[<58;1;1M"[..], &b"\x1b[MZ!!"[..]),
        (72, &b"\x1b[<72;1;1M"[..], &b"\x1b[Mh!!"[..]),
        (73, &b"\x1b[<73;1;1M"[..], &b"\x1b[Mi!!"[..]),
    ] {
        assert_eq!(
            encode_report(MouseReport::Press(button), 1, 1, true, false),
            sgr
        );
        assert_eq!(
            encode_report(MouseReport::Press(button), 1, 1, false, false),
            normal
        );
    }
}

#[test]
fn coordinate_limit_depends_on_the_negotiated_encoding() {
    assert!(encode_report(MouseReport::Press(0), 224, 1, false, false).is_empty());
    assert_eq!(
        encode_report(MouseReport::Press(0), 224, 1, false, true),
        b"\x1b[M \xc4\x80!"
    );
    assert_eq!(
        encode_report(MouseReport::Press(0), 224, 1, true, false),
        b"\x1b[<0;224;1M"
    );
    assert!(encode_report(MouseReport::Press(0), 2016, 1, false, true).is_empty());
}
