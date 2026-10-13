use super::*;

/// One directory as this host spells it, and the path a shell writes for it
/// in a `file://` URL — Windows puts the drive after the leading slash.
fn this_host_dir() -> (&'static str, &'static str) {
    if cfg!(windows) {
        ("/C:/Users/user", "C:\\Users\\user")
    } else {
        ("/home/user", "/home/user")
    }
}

fn osc7_on_mymac(payload: &[u8]) -> Option<String> {
    parse_osc7_path(payload, PathStyle::Posix, Some("mymac"))
}

#[test]
fn parses_osc7_with_this_hostname() {
    let path = osc7_on_mymac(b"file://mymac/home/user/projects/daruda").unwrap();
    assert_eq!(path, "/home/user/projects/daruda");
}

#[test]
fn parses_osc7_with_empty_hostname() {
    assert_eq!(osc7_on_mymac(b"file:///tmp/x").as_deref(), Some("/tmp/x"));
    assert_eq!(
        osc7_on_mymac(b"file://localhost/tmp/x").as_deref(),
        Some("/tmp/x")
    );
}

/// A shell on another machine (`ssh box`) reports a path that means nothing
/// here; taking it would point new tabs and the MCP scan at a local
/// directory that merely shares the name.
#[test]
fn refuses_osc7_from_another_host() {
    assert_eq!(osc7_on_mymac(b"file://box/home/user"), None);
}

#[test]
fn percent_decodes_spaces() {
    assert_eq!(osc7_on_mymac(b"file:///a%20b/c").as_deref(), Some("/a b/c"));
}

/// The Windows reading, asserted from any host: a drive letter keeps no
/// leading slash, and a path with no drive has no Windows form at all.
#[test]
fn osc7_follows_the_windows_style() {
    let windows = |payload: &[u8]| parse_osc7_path(payload, PathStyle::Windows, Some("mymac"));
    assert_eq!(
        windows(b"file:///C:/Users/x").as_deref(),
        Some("C:\\Users\\x")
    );
    assert_eq!(windows(b"file:///home/user"), None);
}

#[test]
fn rejects_non_file_scheme() {
    assert!(osc7_on_mymac(b"http://x/y").is_none());
}

#[test]
fn feed_sets_cwd_via_osc7_bel() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    let (url_path, dir) = this_host_dir();
    session
        .feed(format!("\x1b]7;file://localhost{url_path}\x07").as_bytes())
        .unwrap();
    assert_eq!(session.cwd(), Some(dir));
}

#[test]
fn feed_sets_cwd_via_osc7_st() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    let (url_path, dir) = this_host_dir();
    session
        .feed(format!("\x1b]7;file://localhost{url_path}\x1b\\").as_bytes())
        .unwrap();
    assert_eq!(session.cwd(), Some(dir));
}

/// What a shell really sends: its own `gethostname`, not `localhost`.
#[test]
fn feed_takes_osc7_from_this_machine_and_not_another() {
    let host = daruda_core::host::name().expect("this machine has a name");
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    let (url_path, dir) = this_host_dir();
    session
        .feed(format!("\x1b]7;file://{host}{url_path}\x07").as_bytes())
        .unwrap();
    assert_eq!(session.cwd(), Some(dir));
    session
        .feed(format!("\x1b]7;file://not-this-machine.invalid{url_path}\x07").as_bytes())
        .unwrap();
    assert_eq!(session.cwd(), Some(dir));
}

#[test]
fn track_cwd_disabled_skips_osc7() {
    let cfg = TerminalConfig {
        track_cwd: false,
        ..TerminalConfig::default()
    };
    let mut session = TerminalSession::new(TerminalDims::default(), cfg).unwrap();
    session
        .feed(b"\x1b]7;file://localhost/home/user\x07")
        .unwrap();
    assert_eq!(session.cwd(), None);
}

/// A settings change reaches a session that is already running: the next
/// keystroke sees the new natural-editing switch.
#[test]
fn input_settings_apply_to_a_running_session() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    assert!(session.natural_text_editing());
    assert!(!session.copy_on_select(), "copy-on-select is opt-in");
    session.apply_input_settings(crate::InputSettings {
        natural_text_editing: false,
        osc1337_max_bytes: 4096,
        copy_on_select: true,
    });
    assert!(!session.natural_text_editing());
    assert!(session.copy_on_select());
}
