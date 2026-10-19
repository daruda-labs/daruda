//! Public API tests for the bounded final-answer reader.

use daruda_agent::jsonl::answer::last_assistant_text;

const BUDGET: u64 = 1024 * 1024;

fn assistant(text: &str, sidechain: bool) -> String {
    serde_json::json!({
        "type": "assistant", "uuid": "a", "timestamp": "2026-10-19T00:00:00Z", "isSidechain": sidechain,
        "message": { "model": "claude", "id": "m", "role": "assistant", "content": [{"type": "text", "text": text}] }
    })
    .to_string()
}

fn tool_result(bytes: usize) -> String {
    serde_json::json!({
        "type": "user", "uuid": "t", "timestamp": "now",
        "message": { "role": "user", "content": [{"type": "tool_result", "tool_use_id": "c", "content": "x".repeat(bytes)}] }
    })
    .to_string()
}

fn read(lines: &[&[u8]]) -> Option<String> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.jsonl");
    std::fs::write(&path, lines.join(&b'\n')).unwrap();
    last_assistant_text(&path, BUDGET).unwrap()
}

#[test]
fn a_huge_tool_result_inside_the_budget_does_not_hide_the_answer() {
    let huge = tool_result(BUDGET as usize / 2);
    let answer = assistant("final", false);
    assert_eq!(
        read(&[huge.as_bytes(), answer.as_bytes(), huge.as_bytes(), b""]).as_deref(),
        Some("final")
    );
}

#[test]
fn an_answer_beyond_the_budget_is_not_read() {
    let huge = tool_result(BUDGET as usize);
    let answer = assistant("too far", false);
    assert_eq!(read(&[answer.as_bytes(), huge.as_bytes(), b""]), None);
}

#[test]
fn an_answer_straddling_the_budget_is_not_read() {
    let answer = assistant(&"y".repeat(BUDGET as usize), false);
    assert_eq!(read(&[answer.as_bytes(), b""]), None);
}

#[test]
fn a_torn_final_line_keeps_the_earlier_answer() {
    let answer = assistant("kept", false);
    let torn = &assistant("torn", false).into_bytes()[..40];
    assert_eq!(read(&[answer.as_bytes(), torn]).as_deref(), Some("kept"));
}

#[test]
fn an_answer_without_a_trailing_newline_is_read() {
    let answer = assistant("no newline", false);
    assert_eq!(read(&[answer.as_bytes()]).as_deref(), Some("no newline"));
}

#[test]
fn unknown_entries_and_unparsable_lines_are_skipped() {
    let answer = assistant("kept", false);
    let unknown = br#"{"type":"progress","data":{"nested":[1,2,3]}}"#;
    let string_content = br#"{"type":"user","message":{"role":"user","content":"plain"}}"#;
    assert_eq!(
        read(&[
            answer.as_bytes(),
            unknown,
            string_content,
            b"not json",
            b"",
            b"   ",
            b""
        ])
        .as_deref(),
        Some("kept")
    );
}

#[test]
fn an_invalid_utf8_line_is_skipped() {
    let answer = assistant("kept", false);
    assert_eq!(
        read(&[answer.as_bytes(), b"{\"type\":\"assistant\xff\"}", b""]).as_deref(),
        Some("kept")
    );
}

#[test]
fn sidechain_and_text_free_messages_are_skipped() {
    let answer = assistant("main", false);
    let side = assistant("side", true);
    let mut tools: serde_json::Value = serde_json::from_str(&assistant("", false)).unwrap();
    tools["message"]["content"] = serde_json::json!([
        {"type": "thinking", "thinking": "private", "signature": "s"},
        {"type": "tool_use", "id": "c", "name": "Read", "input": {"path": "/x"}}
    ]);
    let tools = tools.to_string();
    assert_eq!(
        read(&[answer.as_bytes(), side.as_bytes(), tools.as_bytes(), b""]).as_deref(),
        Some("main")
    );
}

#[test]
fn text_blocks_of_one_message_are_joined_by_lines() {
    let mut message: serde_json::Value = serde_json::from_str(&assistant("", false)).unwrap();
    message["message"]["content"] = serde_json::json!([
        {"type": "text", "text": "one"},
        {"type": "tool_use", "id": "c", "name": "Read", "input": {}},
        {"type": "text", "text": "two"}
    ]);
    let message = message.to_string();
    assert_eq!(read(&[message.as_bytes()]).as_deref(), Some("one\ntwo"));
}

#[test]
fn missing_and_empty_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("missing.jsonl");
    assert_eq!(
        last_assistant_text(&path, BUDGET).unwrap_err().kind(),
        std::io::ErrorKind::NotFound
    );
    std::fs::write(&path, "").unwrap();
    assert_eq!(last_assistant_text(&path, BUDGET).unwrap(), None);
}
