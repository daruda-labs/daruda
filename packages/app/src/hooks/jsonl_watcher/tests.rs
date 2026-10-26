use super::*;

#[test]
fn encode_replaces_non_alphanumeric_with_dash() {
    assert_eq!(
        encode_path_for_claude("/Users/Name/My_Project"),
        "-Users-Name-My-Project"
    );
    // Hidden directories.
    assert_eq!(
        encode_path_for_claude("/home/user/.config/proj"),
        "-home-user--config-proj"
    );
    // Spaces.
    assert_eq!(
        encode_path_for_claude("/home/user/My Project"),
        "-home-user-My-Project"
    );
    // Dots.
    assert_eq!(
        encode_path_for_claude("/home/user/proj.v2"),
        "-home-user-proj-v2"
    );
}

#[test]
fn encode_matches_claude_project_dir_encoding() {
    // Verifies the encoding rule produces the expected Claude Code
    // project-dir format: non-alphanumeric characters → dash.
    assert_eq!(
        encode_path_for_claude("/home/user/git/myproject"),
        "-home-user-git-myproject"
    );
}

#[test]
fn project_dir_concatenates_under_home() {
    let home = Path::new("/Users/x");
    let dir = project_dir_for(home, Path::new("/Users/x/proj"));
    assert_eq!(dir, home.join(".claude/projects/-Users-x-proj"));
}

/// Build an `assistant` JSONL line with a known uuid. Single-line
/// output so file-based tests can read it back through
/// `read_last_n_lines` (which splits on `\n`).
fn assistant_line(uuid: &str) -> String {
    format!(
        r#"{{"type":"assistant","uuid":"{uuid}","timestamp":"2026-05-01T00:00:00Z","message":{{"model":"claude-x","id":"m-{uuid}","role":"assistant","content":[{{"type":"text","text":"hi"}}],"stop_reason":"end_turn"}}}}"#
    )
}

fn user_line(uuid: &str) -> String {
    format!(
        r#"{{"type":"user","uuid":"{uuid}","timestamp":"2026-05-01T00:00:00Z","message":{{"role":"user","content":"hi"}}}}"#
    )
}

/// Helper for `process_jsonl_file` tests — write `lines` (each
/// already JSON-encoded) into a freshly-named jsonl inside the
/// given `dir` and return the path.
fn write_jsonl(dir: &Path, stem: &str, lines: &[String]) -> PathBuf {
    let path = dir.join(format!("{stem}.jsonl"));
    std::fs::write(&path, lines.join("\n")).expect("write jsonl");
    path
}

#[test]
fn process_jsonl_file_skips_non_jsonl_extension() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("notes.txt");
    std::fs::write(&path, "irrelevant").unwrap();
    let perms = PermissionChecker::default();
    assert!(
        process_jsonl_file(
            &path,
            tmp.path(),
            &perms,
            ReadStrategy::Tail(TAIL_ENTRIES),
            Utc::now()
        )
        .is_none(),
        "non-jsonl extension must be skipped"
    );
}

#[test]
fn process_jsonl_file_skips_agent_prefix() {
    let tmp = tempfile::tempdir().unwrap();
    let path = write_jsonl(tmp.path(), "agent-subagent", &[assistant_line("a1")]);
    let perms = PermissionChecker::default();
    assert!(
        process_jsonl_file(
            &path,
            tmp.path(),
            &perms,
            ReadStrategy::Tail(TAIL_ENTRIES),
            Utc::now()
        )
        .is_none(),
        "agent-* files must be skipped"
    );
}

#[test]
fn process_jsonl_file_returns_event_with_session_identity() {
    let tmp = tempfile::tempdir().unwrap();
    let lines = vec![
        user_line("u1"),
        assistant_line("a1"),
        user_line("u2"),
        assistant_line("a2"),
    ];
    let path = write_jsonl(tmp.path(), "abc12345", &lines);
    let perms = PermissionChecker::default();
    let cwd = PathBuf::from("/tmp/repo");

    let event = process_jsonl_file(&path, &cwd, &perms, ReadStrategy::Full, Utc::now())
        .expect("should produce an event");
    assert_eq!(event.event.session_id, "abc12345");
    assert_eq!(event.event.cwd, cwd);
    assert_eq!(event.event.jsonl_path, path);
}

#[test]
fn process_jsonl_file_returns_none_for_empty_file() {
    let tmp = tempfile::tempdir().unwrap();
    let path = write_jsonl(tmp.path(), "empty", &[]);
    let perms = PermissionChecker::default();
    assert!(
        process_jsonl_file(
            &path,
            tmp.path(),
            &perms,
            ReadStrategy::Tail(TAIL_ENTRIES),
            Utc::now()
        )
        .is_none(),
        "empty file produces no entries"
    );
}

#[test]
fn full_read_does_not_retain_the_history_allocation_for_pending_status() {
    let tmp = tempfile::tempdir().unwrap();
    let now = Utc::now();
    let mut lines: Vec<_> = (0..4096)
        .map(|i| user_line(&format!("history-{i}")))
        .collect();
    let mut latest: serde_json::Value = serde_json::from_str(&assistant_line("latest")).unwrap();
    latest["timestamp"] = now.to_rfc3339().into();
    latest["message"]["stop_reason"] = serde_json::Value::Null;
    lines.push(latest.to_string());
    lines.push(serde_json::json!({"type": "progress"}).to_string());
    let path = write_jsonl(tmp.path(), "large", &lines);
    let observation = process_jsonl_file(
        &path,
        tmp.path(),
        &PermissionChecker::default(),
        ReadStrategy::Full,
        now,
    )
    .unwrap();
    assert_eq!(observation.event.status, SessionStatus::Working);
    assert_eq!(observation.entries.len(), 2);
    assert!(
        observation.entries.capacity() <= TAIL_ENTRIES,
        "a pending cache must stay within a tail-sized allocation, not retain {} history slots",
        observation.entries.capacity()
    );
}
