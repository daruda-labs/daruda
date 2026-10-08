//! Desktop notifications behind an OS-specific delivery boundary.
//!
//! Why osascript and not `UNUserNotificationCenter`: the latter
//! requires the running binary to be code-signed and notarized into
//! its own Notification Center entry. For the in-development
//! `cargo run -p daruda` flow that signing is not present, so the
//! framework call silently fails. AppleScript's `display notification`
//! is available on every macOS install since 10.9 and surfaces in
//! Notification Center under the "Script Editor" identity — visible
//! and dismissible by the user, which is the goal.
//!
//! Tests replace the OS boundary with a thread-local recording fake. Besides
//! preventing external side effects, thread-local storage keeps parallel test
//! cases from consuming each other's notifications.
//!
//! Windows uses native toast notifications with a profile-specific identity.
//! Linux delegates delivery to notify-send. No backend waits for dismissal.

#[cfg(test)]
use std::cell::RefCell;
#[cfg(all(not(test), target_os = "macos"))]
use std::process::{Command, Stdio};

#[cfg(windows)]
#[path = "notifications_windows.rs"]
mod windows;

/// A live pane to reveal when a notification is selected.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Target {
    pub workspace: daruda_store::project::WorkspaceUuid,
    pub pane: u64,
}

/// Keep notification activation on the GPUI foreground thread.
pub(crate) fn install(cx: &mut gpui::App) {
    #[cfg(windows)]
    windows::install(cx);
    #[cfg(not(windows))]
    let _ = cx;
}

/// Remove generated shortcuts belonging to the executable being uninstalled.
pub(crate) fn unregister_desktop() -> anyhow::Result<()> {
    #[cfg(windows)]
    return windows::unregister();
    #[cfg(not(windows))]
    Ok(())
}

pub(crate) fn show_for(title: &str, body: &str, target: Target) {
    #[cfg(all(windows, not(test)))]
    report(windows::deliver(title, body, Some(target)));
    #[cfg(any(not(windows), test))]
    {
        let _ = (target.workspace, target.pane);
        deliver(title, body);
    }
}

#[cfg(any(not(test), windows))]
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
#[cfg(any(not(test), windows))]
use daruda_store::observability::log_writer::LogWriter;

#[cfg(test)]
thread_local! {
    static RECORDED_NOTIFICATIONS: RefCell<Vec<(String, String)>> = const {
        RefCell::new(Vec::new())
    };
}

/// Fire one desktop notification. Both `title` and `body` may contain
/// arbitrary UTF-8; double-quotes and backslashes are escaped so the
/// generated AppleScript stays well-formed regardless of payload.
pub fn show(title: &str, body: &str) {
    deliver(title, body);
}

#[cfg(all(not(test), target_os = "macos"))]
fn deliver(title: &str, body: &str) {
    let script = format!(
        r#"display notification "{}" with title "{}""#,
        escape_applescript(body),
        escape_applescript(title),
    );
    // Detach: spawn and forget. stdout/stderr suppressed because a
    // failed notification is not actionable from the daruda UI — the
    // spawn error itself is still logged so we don't lose visibility
    // when (e.g.) `osascript` goes missing on a stripped system.
    if let Err(e) = Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        LogWriter::log(
            ErrorReport::new("Desktop notification spawn failed")
                .severity(ErrorSeverity::Info)
                .from_error(&e)
                .at(file!(), line!())
                .dedup("platform.notification.spawn")
                .build(),
        );
    }
}

#[cfg(all(not(test), windows))]
fn deliver(title: &str, body: &str) {
    report(windows::deliver(title, body, None));
}

#[cfg(all(not(test), target_os = "linux"))]
fn deliver(title: &str, body: &str) {
    report(
        daruda_core::process::command("notify-send")
            .args(["--app-name=daruda", "--", title, body])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map(|_| ())
            .map_err(Into::into),
    );
}

#[cfg(any(windows, all(not(test), target_os = "linux")))]
fn report(result: anyhow::Result<()>) {
    if let Err(error) = result {
        LogWriter::log(
            ErrorReport::new("Desktop notification delivery failed")
                .severity(ErrorSeverity::Info)
                .from_error(error.root_cause())
                .at(file!(), line!())
                .dedup("platform.notification.delivery")
                .build(),
        );
    }
}

#[cfg(test)]
fn deliver(title: &str, body: &str) {
    RECORDED_NOTIFICATIONS.with(|notifications| {
        notifications
            .borrow_mut()
            .push((title.to_string(), body.to_string()));
    });
}

#[cfg(test)]
pub(crate) fn take_recorded_for_test() -> Vec<(String, String)> {
    RECORDED_NOTIFICATIONS.with(|notifications| std::mem::take(&mut *notifications.borrow_mut()))
}

/// Escape `\` and `"` so the AppleScript string literal stays valid.
/// Newlines pass through unchanged — AppleScript renders them as
/// real line breaks in the notification body, which matches what the
/// shell intends.
#[cfg(any(test, target_os = "macos"))]
fn escape_applescript(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_handles_quote_and_backslash() {
        assert_eq!(escape_applescript(r#"a "b" c"#), r#"a \"b\" c"#);
        assert_eq!(escape_applescript(r"path\to\file"), r"path\\to\\file");
        assert_eq!(escape_applescript("plain text"), "plain text");
    }

    #[test]
    fn escape_passes_through_newlines() {
        assert_eq!(escape_applescript("line1\nline2"), "line1\nline2");
    }

    #[test]
    fn test_backend_records_without_cross_call_residue() {
        let _ = take_recorded_for_test();

        show("Agent", "Agent finished responding");

        assert_eq!(
            take_recorded_for_test(),
            vec![("Agent".to_string(), "Agent finished responding".to_string())]
        );
        assert!(take_recorded_for_test().is_empty());
    }
}
