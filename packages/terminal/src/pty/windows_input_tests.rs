//! Byte-level native ConPTY regression fixture, isolated from desktop input.

use super::*;
use std::time::{Duration, Instant};

fn payload() -> String {
    "한글🙂\rsecond line\r".repeat(8192)
}

#[test]
fn native_large_multiline_input_preserves_utf8() {
    let config = PtyConfig {
        shell: std::env::current_exe()
            .unwrap()
            .to_string_lossy()
            .into_owned(),
        ..Default::default()
    };
    let args = [
        "--exact",
        "pty::windows_input_tests::input_collector",
        "--ignored",
        "--nocapture",
    ]
    .map(std::ffi::OsString::from);
    let handle = spawn_pty_with_args(&config, &args).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut output = Vec::new();
    let mut answered = false;
    loop {
        let chunk = handle
            .stdout_rx
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap_or_else(|error| panic!("{error:?}: {}", String::from_utf8_lossy(&output)));
        output.extend_from_slice(&chunk);
        if !answered && output.windows(4).any(|bytes| bytes == b"\x1b[6n") {
            handle.write(b"\x1b[1;1R").unwrap();
            answered = true;
        }
        if String::from_utf8_lossy(&output).contains("INPUT_PROBE_READY") {
            break;
        }
    }
    // Console VT input does not advertise bracketed paste; use the same
    // carriage-return encoding as the bottom-dock command submission path.
    let mut bytes = payload().into_bytes();
    bytes.extend_from_slice(b"INPUT_PROBE_END");
    handle.write(&bytes).unwrap();
    loop {
        let chunk = handle
            .stdout_rx
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap_or_else(|error| panic!("{error:?}: {}", String::from_utf8_lossy(&output)));
        output.extend_from_slice(&chunk);
        let text = String::from_utf8_lossy(&output);
        assert!(!text.contains("INPUT_PROBE_MISMATCH"), "{text}");
        if text.contains("INPUT_PROBE_OK") {
            break;
        }
    }
}

#[test]
#[ignore = "Child fixture invoked only by the isolated ConPTY parent test"]
fn input_collector() {
    use std::io::{Read as _, Write as _};
    use windows_sys::Win32::System::Console::{
        ENABLE_ECHO_INPUT, ENABLE_LINE_INPUT, ENABLE_PROCESSED_INPUT,
        ENABLE_VIRTUAL_TERMINAL_INPUT, GetConsoleMode, GetStdHandle, STD_INPUT_HANDLE,
        SetConsoleCP, SetConsoleMode,
    };
    // SAFETY: only this fixture's private pseudoconsole is changed.
    unsafe {
        let input = GetStdHandle(STD_INPUT_HANDLE);
        let mut mode = 0;
        assert_ne!(GetConsoleMode(input, &mut mode), 0);
        assert_ne!(SetConsoleCP(65001), 0);
        assert_ne!(
            SetConsoleMode(
                input,
                (mode & !(ENABLE_LINE_INPUT | ENABLE_ECHO_INPUT | ENABLE_PROCESSED_INPUT))
                    | ENABLE_VIRTUAL_TERMINAL_INPUT
            ),
            0
        );
    }
    println!("INPUT_PROBE_READY");
    std::io::stdout().flush().unwrap();
    let mut input = std::io::stdin();
    let mut bytes = Vec::new();
    let mut chunk = [0; 8192];
    while !bytes.ends_with(b"INPUT_PROBE_END") {
        let count = input.read(&mut chunk).unwrap();
        assert_ne!(count, 0);
        bytes.extend_from_slice(&chunk[..count]);
        assert!(bytes.len() <= payload().len() + 15);
    }
    let mut expected = payload().into_bytes();
    expected.extend_from_slice(b"INPUT_PROBE_END");
    if bytes == expected {
        println!("INPUT_PROBE_OK");
    } else {
        println!("INPUT_PROBE_MISMATCH");
        panic!("Native input differed from the submitted payload");
    }
}
