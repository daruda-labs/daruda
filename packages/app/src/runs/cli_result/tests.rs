use super::*;

pub(in crate::runs) fn assistant(text: &str, sidechain: bool) -> String {
    serde_json::json!({
        "type": "assistant", "uuid": "a", "timestamp": "2026-10-19T00:00:00Z", "isSidechain": sidechain,
        "message": { "model": "claude", "id": "m", "role": "assistant", "content": [{"type":"text", "text":text}] }
    }).to_string()
}

#[test]
fn result_reader_ignores_sidechains_thinking_and_tool_only_messages() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("result.jsonl");
    let tool = serde_json::json!({ "type":"user", "uuid":"tool", "timestamp":"now", "message":{"role":"user", "content":[{"type":"tool_result","content":"x".repeat(1_000_000)}]} }).to_string();
    let mut thinking: serde_json::Value =
        serde_json::from_str(&assistant("unused", false)).unwrap();
    thinking["message"]["content"] = serde_json::json!([{"type":"thinking","thinking":"private"},{"type":"tool_use","id":"call","name":"Read","input":{}}]);
    std::fs::write(
        &path,
        format!(
            "{tool}\n{}\n{tool}\n{}\n{thinking}\n",
            assistant("final answer", false),
            assistant("side answer", true)
        ),
    )
    .unwrap();
    assert_eq!(read(&path).as_deref(), Some("final answer"));
    std::fs::write(
        &path,
        format!("{}\nbroken JSON\n", assistant("old answer", false)),
    )
    .unwrap();
    assert_eq!(read(&path), None);
    std::fs::write(&path, b"invalid UTF-8: \xff").unwrap();
    assert_eq!(read(&path), None);
    assert_eq!(read(&dir.path().join("missing")), None);
    std::fs::write(&path, "").unwrap();
    assert_eq!(read(&path), None);
}
