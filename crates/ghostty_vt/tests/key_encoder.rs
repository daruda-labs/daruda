use ghostty_vt::{KeyModeFlags, KeyModifiers, encode_key_named};

#[test]
fn encodes_common_special_keys() {
    assert_eq!(
        encode_key_named("up", KeyModifiers::default(), KeyModeFlags::default()).as_deref(),
        Some(&b"\x1b[A"[..])
    );
    assert_eq!(
        encode_key_named("f1", KeyModifiers::default(), KeyModeFlags::default()).as_deref(),
        Some(&b"\x1bOP"[..])
    );
    assert_eq!(
        encode_key_named("pageup", KeyModifiers::default(), KeyModeFlags::default()).as_deref(),
        Some(&b"\x1b[5~"[..])
    );
}

#[test]
fn encoding_changes_with_modifiers_for_special_keys() {
    let no_mods = encode_key_named("up", KeyModifiers::default(), KeyModeFlags::default()).unwrap();
    let ctrl = encode_key_named(
        "up",
        KeyModifiers {
            control: true,
            ..Default::default()
        },
        KeyModeFlags::default(),
    )
    .unwrap();

    assert_ne!(no_mods, ctrl);
}

#[test]
fn decckm_changes_arrow_key_encoding() {
    let normal = encode_key_named("up", KeyModifiers::default(), KeyModeFlags::default()).unwrap();
    let app = encode_key_named(
        "up",
        KeyModifiers::default(),
        KeyModeFlags {
            cursor_key_application: true,
            ..Default::default()
        },
    )
    .unwrap();
    // Normal mode: CSI A. Application mode (DECCKM): SS3 A.
    assert_eq!(normal, b"\x1b[A");
    assert_eq!(app, b"\x1bOA");
}

#[test]
fn decnkm_does_not_affect_non_keypad_keys() {
    // The FFI maps standard navigation/function keys only; DECNKM
    // (keypad_key_application) only changes dedicated KP keys (kp_0..kp_9,
    // kp_enter, etc.) which are not in the FFI key name table.
    // Arrow keys are governed by DECCKM (cursor_key_application), not DECNKM.
    let normal = encode_key_named("up", KeyModifiers::default(), KeyModeFlags::default()).unwrap();
    let with_decnkm = encode_key_named(
        "up",
        KeyModifiers::default(),
        KeyModeFlags {
            keypad_key_application: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        normal, with_decnkm,
        "DECNKM must not change arrow key encoding"
    );
}
