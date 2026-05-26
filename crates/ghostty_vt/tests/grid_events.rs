use ghostty_vt::{GridEvent, Terminal};

/// A freshly constructed terminal has no pending grid events.
#[test]
fn empty_when_no_events() {
    let mut terminal = Terminal::new(80, 24).unwrap();
    assert!(terminal.take_grid_events().is_empty());
}

/// `take_grid_events` drains the queue: a second consecutive call after
/// an event returns an empty Vec.
#[test]
fn drain_is_one_shot() {
    let mut terminal = Terminal::new(80, 24).unwrap();
    // ESC c — hard reset (RIS).
    terminal.feed(b"\x1bc").unwrap();
    let first = terminal.take_grid_events();
    assert!(first.contains(&GridEvent::Ris));
    let second = terminal.take_grid_events();
    assert!(second.is_empty());
}

/// DECSET 1049 enters the alternate screen and DECRST 1049 leaves it.
/// Both transitions are reported as `AltScreenToggle` events.
#[test]
fn alt_screen_1049_enter_and_exit() {
    let mut terminal = Terminal::new(80, 24).unwrap();
    terminal.feed(b"\x1b[?1049h").unwrap();
    let enter = terminal.take_grid_events();
    assert_eq!(enter, vec![GridEvent::AltScreenToggle { entered: true }]);

    terminal.feed(b"\x1b[?1049l").unwrap();
    let exit = terminal.take_grid_events();
    assert_eq!(exit, vec![GridEvent::AltScreenToggle { entered: false }]);
}

/// Setting alt-screen mode to its current state must not produce a
/// duplicate event — only real transitions are reported.
#[test]
fn alt_screen_idempotent_writes_dedup() {
    let mut terminal = Terminal::new(80, 24).unwrap();
    terminal.feed(b"\x1b[?1049h").unwrap();
    let _ = terminal.take_grid_events();
    // Enable again — already on alt-screen, expect no new event.
    terminal.feed(b"\x1b[?1049h").unwrap();
    assert!(terminal.take_grid_events().is_empty());
}

/// Multiple events queued between two drains are reported in order.
#[test]
fn multiple_events_preserve_order() {
    let mut terminal = Terminal::new(80, 24).unwrap();
    terminal.feed(b"\x1b[?1049h").unwrap();
    terminal.feed(b"\x1b[?1049l").unwrap();
    let drained = terminal.take_grid_events();
    assert_eq!(
        drained,
        vec![
            GridEvent::AltScreenToggle { entered: true },
            GridEvent::AltScreenToggle { entered: false },
        ]
    );
}

/// RIS issued while on the alt-screen must report the implicit
/// alt-screen exit before the `Ris` event so consumers see the screen
/// transition first.
#[test]
fn ris_from_alt_screen_emits_exit_then_ris() {
    let mut terminal = Terminal::new(80, 24).unwrap();
    terminal.feed(b"\x1b[?1049h").unwrap();
    let _ = terminal.take_grid_events();
    terminal.feed(b"\x1bc").unwrap();
    let drained = terminal.take_grid_events();
    assert_eq!(
        drained,
        vec![
            GridEvent::AltScreenToggle { entered: false },
            GridEvent::Ris,
        ]
    );
}

/// Legacy mode 47 also triggers alt-screen events.
#[test]
fn alt_screen_47_legacy_mode() {
    let mut terminal = Terminal::new(80, 24).unwrap();
    terminal.feed(b"\x1b[?47h").unwrap();
    assert_eq!(
        terminal.take_grid_events(),
        vec![GridEvent::AltScreenToggle { entered: true }]
    );
    terminal.feed(b"\x1b[?47l").unwrap();
    assert_eq!(
        terminal.take_grid_events(),
        vec![GridEvent::AltScreenToggle { entered: false }]
    );
}

/// Mode 1047 (alt-screen with clear-on-enter) also emits the same
/// `AltScreenToggle` event as 47 / 1049.
#[test]
fn alt_screen_1047_mode() {
    let mut terminal = Terminal::new(80, 24).unwrap();
    terminal.feed(b"\x1b[?1047h").unwrap();
    assert_eq!(
        terminal.take_grid_events(),
        vec![GridEvent::AltScreenToggle { entered: true }]
    );
}
