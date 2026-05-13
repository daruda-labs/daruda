//! Desktop notifications via `osascript display notification`.
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
//! The call shells out and detaches; we never wait for the user to
//! dismiss. Concurrency is unbounded by design (one notification per
//! pane event), but `osascript` is cheap (~50 ms cold, fork+exec) and
//! macOS itself coalesces duplicates in Notification Center.

use std::process::{Command, Stdio};

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;

/// Fire one desktop notification. Both `title` and `body` may contain
/// arbitrary UTF-8; double-quotes and backslashes are escaped so the
/// generated AppleScript stays well-formed regardless of payload.
pub fn show(title: &str, body: &str) {
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

/// Escape `\` and `"` so the AppleScript string literal stays valid.
/// Newlines pass through unchanged — AppleScript renders them as
/// real line breaks in the notification body, which matches what the
/// shell intends.
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
}
