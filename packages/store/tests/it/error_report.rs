//! `ErrorReport` construction and rendering.

use std::error::Error;
use std::time::Duration;

use daruda_store::observability::error_report::*;

#[derive(Debug)]
struct InnerErr;
impl std::fmt::Display for InnerErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "inner I/O failure")
    }
}
impl Error for InnerErr {}

#[derive(Debug)]
struct OuterErr;
impl std::fmt::Display for OuterErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "outer wrap")
    }
}
impl Error for OuterErr {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        static SRC: InnerErr = InnerErr;
        Some(&SRC)
    }
}

#[test]
fn severity_dismiss_timing_matches_d4() {
    assert_eq!(
        ErrorSeverity::Info.auto_dismiss_after(),
        Duration::from_secs(5)
    );
    assert_eq!(
        ErrorSeverity::Warning.auto_dismiss_after(),
        Duration::from_secs(8)
    );
    assert_eq!(
        ErrorSeverity::Error.auto_dismiss_after(),
        Duration::from_secs(30)
    );
}

#[test]
fn from_error_flattens_source_chain() {
    let r = ErrorReport::from_error("Outer failed", &OuterErr);
    assert_eq!(r.title, "Outer failed");
    assert_eq!(r.message, "outer wrap");
    assert_eq!(r.source_chain, vec!["inner I/O failure".to_string()]);
    assert_eq!(r.severity, ErrorSeverity::Error);
}

#[test]
fn builder_overrides_apply_in_order() {
    let r = ErrorReport::new("PTY died")
        .severity(ErrorSeverity::Error)
        .message("Broken pipe (os error 32)")
        .at("packages/app/src/pty.rs", 229)
        .with_context("session", "tab-3-pane-1")
        .with_context("cwd", "~/git/daruda")
        .dedup("pty.writer")
        .build();
    assert_eq!(r.title, "PTY died");
    assert_eq!(r.message, "Broken pipe (os error 32)");
    assert_eq!(r.location.as_deref(), Some("packages/app/src/pty.rs:229"));
    assert_eq!(
        r.context.get("session").map(String::as_str),
        Some("tab-3-pane-1")
    );
    assert_eq!(r.dedup_key.as_deref(), Some("pty.writer"));
}

#[test]
fn plain_text_includes_all_sections_in_order() {
    let r = ErrorReport::new("PTY died")
        .from_error(&OuterErr)
        .at("packages/app/src/pty.rs", 229)
        .with_context("session", "tab-3-pane-1")
        .with_backtrace("frame 0\nframe 1")
        .dedup("pty.writer")
        .build();
    let txt = r.to_plain_text();

    let pos_title = txt.find("[daruda] PTY died").expect("title present");
    let pos_severity = txt.find("severity: error").expect("severity present");
    let pos_time = txt.find("time:").expect("timestamp present");
    let pos_location = txt.find("location:").expect("location present");
    let pos_message = txt.find("\nmessage:\n").expect("message present");
    let pos_source = txt.find("\nsource chain:\n").expect("source chain present");
    let pos_context = txt.find("\ncontext:\n").expect("context present");
    let pos_backtrace = txt.find("\nbacktrace:\n").expect("backtrace present");

    assert!(pos_title < pos_severity);
    assert!(pos_severity < pos_time);
    assert!(pos_time < pos_location);
    assert!(pos_location < pos_message);
    assert!(pos_message < pos_source);
    assert!(pos_source < pos_context);
    assert!(pos_context < pos_backtrace);

    assert!(txt.contains("  outer wrap"));
    assert!(txt.contains("  └─ inner I/O failure"));
    assert!(txt.contains("session"));
    assert!(txt.contains("frame 0"));
}

#[test]
fn ndjson_line_round_trips() {
    let r = ErrorReport::new("MCP reload failed")
        .severity(ErrorSeverity::Warning)
        .message("connection refused")
        .with_context("scope", "personal")
        .dedup("mcp.reload.personal")
        .build();
    let line = r.to_ndjson_line();
    assert!(line.ends_with('\n'));

    let trimmed = line.trim_end_matches('\n');
    let value: serde_json::Value = serde_json::from_str(trimmed).expect("valid JSON");
    assert_eq!(value["title"], "MCP reload failed");
    assert_eq!(value["severity"], "warning");
    assert_eq!(value["message"], "connection refused");
    assert_eq!(value["dedup_key"], "mcp.reload.personal");
    assert!(
        value["text"]
            .as_str()
            .unwrap()
            .contains("MCP reload failed")
    );

    let parsed: ErrorReport = serde_json::from_str(trimmed).expect("ErrorReport deserializes");
    assert_eq!(parsed.title, r.title);
    assert_eq!(parsed.severity, r.severity);
    assert_eq!(parsed.message, r.message);
    assert_eq!(parsed.dedup_key, r.dedup_key);
}

#[test]
fn info_ndjson_line_omits_text_block() {
    let r = ErrorReport::new("Claude session status changed")
        .severity(ErrorSeverity::Info)
        .message("ExecutingTool -> Working")
        .with_context("cwd", "~/git/x")
        .build();
    let line = r.to_ndjson_line();
    assert!(line.ends_with('\n'));
    let value: serde_json::Value =
        serde_json::from_str(line.trim_end_matches('\n')).expect("valid JSON");

    // Structured fields stay — they carry the full signal.
    assert_eq!(value["message"], "ExecutingTool -> Working");
    assert_eq!(value["context"]["cwd"], "~/git/x");
    assert_eq!(value["severity"], "info");
    // The duplicated human-text block is dropped for Info.
    assert!(
        value.get("text").is_none(),
        "Info entries must omit the text block, got: {line}"
    );

    // Still deserializes back into an ErrorReport (text was extra).
    let parsed: ErrorReport =
        serde_json::from_str(line.trim_end_matches('\n')).expect("ErrorReport deserializes");
    assert_eq!(parsed.severity, ErrorSeverity::Info);
    assert_eq!(parsed.message, "ExecutingTool -> Working");
}

#[test]
fn empty_optional_sections_are_omitted_in_plain_text() {
    let r = ErrorReport::new("Bare").build();
    let txt = r.to_plain_text();
    assert!(txt.contains("[daruda] Bare"));
    assert!(!txt.contains("source chain:"));
    assert!(!txt.contains("context:"));
    assert!(!txt.contains("backtrace:"));
    assert!(!txt.contains("location:"));
}

#[test]
fn project_id_is_attached_via_builder_and_round_trips() {
    let r = ErrorReport::new("Lane probe failed")
        .severity(ErrorSeverity::Warning)
        .message("permission denied")
        .project_id(7)
        .build();
    assert_eq!(r.project_id, Some(7));

    let line = r.to_ndjson_line();
    let trimmed = line.trim_end_matches('\n');
    let value: serde_json::Value = serde_json::from_str(trimmed).expect("valid JSON");
    assert_eq!(value["project_id"], 7);

    let parsed: ErrorReport = serde_json::from_str(trimmed).expect("deserializes");
    assert_eq!(parsed.project_id, Some(7));
}

#[test]
fn project_id_absent_is_omitted_from_json() {
    let r = ErrorReport::new("Bare").build();
    assert!(r.project_id.is_none());
    let line = r.to_ndjson_line();
    assert!(
        !line.contains("\"project_id\""),
        "absent project_id must not be serialized, got: {line}"
    );
}

#[test]
fn from_panic_str_payload_captured() {
    let result = std::panic::catch_unwind(|| {
        std::panic::set_hook(Box::new(|info| {
            let r = ErrorReport::from_panic(info);
            assert_eq!(r.title, "daruda panicked");
            assert!(r.message.contains("test panic message"));
            assert_eq!(r.severity, ErrorSeverity::Error);
            assert_eq!(r.dedup_key.as_deref(), Some("panic"));
            assert!(r.backtrace.is_some());
            assert!(r.location.is_some());
        }));
        panic!("test panic message");
    });
    let _ = std::panic::take_hook();
    assert!(result.is_err());
}
