use super::*;

fn elider() -> Elider {
    Elider {
        cap: DEFAULT_MAX_FIELD,
        record: true,
        spills: Vec::new(),
    }
}

fn os(value: &str) -> Option<&OsStr> {
    Some(OsStr::new(value))
}

#[test]
fn an_explicit_base_wins_over_the_default() {
    let settings = TapSettings::resolve(
        os("/tmp/tap/acp.log"),
        None,
        None,
        Some(PathBuf::from("/logs/acp-wire.log")),
    )
    .unwrap();
    assert_eq!(settings.base, PathBuf::from("/tmp/tap/acp.log"));
}

#[test]
fn the_default_base_applies_only_when_none_is_set() {
    let settings =
        TapSettings::resolve(None, None, None, Some(PathBuf::from("/logs/acp-wire.log"))).unwrap();
    assert_eq!(
        settings,
        TapSettings {
            base: PathBuf::from("/logs/acp-wire.log"),
            cap: DEFAULT_MAX_FIELD,
            sidecar: true,
        }
    );
    assert_eq!(TapSettings::resolve(None, os("0"), None, None), None);
}

#[test]
fn no_elision_means_no_sidecar() {
    let settings = TapSettings::resolve(os("/tmp/a.log"), os("0"), os("on"), None).unwrap();
    assert_eq!((settings.cap, settings.sidecar), (0, false));
    let settings = TapSettings::resolve(os("/tmp/a.log"), None, os("off"), None).unwrap();
    assert_eq!((settings.cap, settings.sidecar), (DEFAULT_MAX_FIELD, false));
}

#[test]
fn wire_log_path_splices_agent_id_before_the_extension() {
    assert_eq!(
        wire_log_path_for(Path::new("/logs/acp-wire.log"), "claude"),
        PathBuf::from("/logs/acp-wire-claude.log")
    );
}

#[test]
fn wire_log_path_handles_no_extension() {
    assert_eq!(
        wire_log_path_for(Path::new("/logs/acp-wire"), "codex"),
        PathBuf::from("/logs/acp-wire-codex")
    );
}

#[test]
fn wire_log_path_leaves_base_untouched_when_agent_id_is_empty() {
    assert_eq!(
        wire_log_path_for(Path::new("/logs/acp-wire.log"), ""),
        PathBuf::from("/logs/acp-wire.log")
    );
}

#[test]
fn a_new_run_keeps_the_previous_capture() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("acp-wire-rotate-test.log");
    std::fs::write(&path, "run one\n").unwrap();

    drop(open_log(&path).expect("the log opens"));

    assert_eq!(
        std::fs::read_to_string(previous_generation_path(&path)).unwrap(),
        "run one\n",
        "the earlier run is moved aside, not overwritten"
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "",
        "the new run starts empty under the stable name"
    );
}

/// The rotated copy keeps its file name, so the reader's two naming
/// conventions still apply: the agent id parses, and its sidecar sits beside
/// it under the name [`payload_sidecar_path`] derives.
#[test]
fn a_rotated_capture_is_still_readable_by_the_replay_naming() {
    let dir = tempfile::tempdir().unwrap();
    let slim = dir.path().join("acp-wire-claude.log");
    let sidecar = crate::wire_log::payload_sidecar_path(&slim);
    std::fs::write(&slim, "slim\n").unwrap();
    std::fs::write(&sidecar, "payload\n").unwrap();

    drop(open_log(&slim).expect("slim opens"));
    drop(open_log(&sidecar).expect("sidecar opens"));

    let rotated = previous_generation_path(&slim);
    assert_eq!(
        crate::wire_log::agent_id_from_path(&rotated).as_deref(),
        Some("claude"),
        "the agent id still parses out of the rotated name"
    );
    assert_eq!(
        std::fs::read_to_string(crate::wire_log::payload_sidecar_path(&rotated)).unwrap(),
        "payload\n",
        "and its sidecar is where the reader derives it"
    );
}

/// `attach` runs per ACP session, so only the first open of a process
/// rotates — a second session must not push its sibling's log out of the
/// one slot, and both must land in the file intact — a non-append handle
/// keeps its own offset and overwrites whatever its sibling appended.
#[test]
fn two_sessions_in_one_run_interleave_without_losing_lines() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("acp-wire-rotate-twice-test.log");
    std::fs::write(&path, "run one\n").unwrap();

    let first = open_log(&path).expect("first session");
    let second = open_log(&path).expect("second session");
    writeln!(first.lock().unwrap(), "a1").unwrap();
    writeln!(second.lock().unwrap(), "b1").unwrap();
    writeln!(first.lock().unwrap(), "a2").unwrap();
    writeln!(second.lock().unwrap(), "b2").unwrap();
    drop((first, second));

    assert_eq!(
        std::fs::read_to_string(previous_generation_path(&path)).unwrap(),
        "run one\n",
        "the rotated copy is the previous *run*, not the first session"
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "a1\nb1\na2\nb2\n",
        "neither session's lines are overwritten by the other's offset"
    );
}

/// Nothing to move aside is the normal first-ever run, not an error.
#[test]
fn a_first_ever_run_rotates_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("acp-wire-rotate-fresh-test.log");

    open_log(&path).expect("the log opens");

    assert!(path.exists(), "the live log is created");
    assert!(
        !previous_generation_path(&path).exists(),
        "no phantom previous generation"
    );
}

#[test]
fn previous_generation_keeps_the_file_name_and_changes_the_directory() {
    assert_eq!(
        previous_generation_path(Path::new("/logs/acp-wire-claude.log")),
        PathBuf::from("/logs/prev/acp-wire-claude.log")
    );
    assert_eq!(
        previous_generation_path(Path::new("/logs/acp-wire-claude.payload.jsonl")),
        PathBuf::from("/logs/prev/acp-wire-claude.payload.jsonl")
    );
}

#[test]
fn first_open_is_once_per_path_per_process() {
    let path_a = PathBuf::from("/logs/acp-wire-first-open-test-a.log");
    let path_b = PathBuf::from("/logs/acp-wire-first-open-test-b.log");
    assert!(first_open(&path_a), "first open truncates");
    assert!(!first_open(&path_a), "second open appends");
    assert!(!first_open(&path_a), "third open still appends");
    assert!(first_open(&path_b), "a different path truncates once");
}

#[test]
fn wire_log_path_sanitizes_unsafe_agent_id_characters() {
    // A user-editable catalog id must never let a path separator escape the
    // configured log directory.
    assert_eq!(
        wire_log_path_for(Path::new("/logs/acp-wire.log"), "../evil"),
        PathBuf::from("/logs/acp-wire-___evil.log")
    );
}

#[test]
fn wire_log_path_handles_a_bare_file_name_with_no_directory() {
    assert_eq!(
        wire_log_path_for(Path::new("acp-wire.log"), "claude"),
        PathBuf::from("acp-wire-claude.log")
    );
}

#[test]
fn a_fat_field_is_replaced_by_a_resolvable_marker_and_short_ones_survive() {
    let line = serde_json::json!({
        "params": {"update": {
            "status": "completed",
            "content": [{"oldText": "a".repeat(600), "newText": "short"}],
        }},
    })
    .to_string();
    let mut elider = elider();
    let slim: Value = serde_json::from_str(&elider.slim(&line)).expect("slim line is JSON");

    let update = &slim["params"]["update"];
    assert_eq!(update["status"], "completed");
    assert_eq!(update["content"][0]["newText"], "short");

    let [spill] = &elider.spills[..] else {
        panic!("exactly one spill, got {}", elider.spills.len());
    };
    assert_eq!(spill.path, "params.update.content[0].oldText");
    assert_eq!(spill.text.len(), 600);
    let elided = update["content"][0]["oldText"]
        .as_str()
        .expect("still a string");
    assert!(
        elided.ends_with(&format!("@@acp-payload:{}:600@@", spill.id)),
        "marker resolves to the record: {elided}"
    );
}

#[test]
fn a_multibyte_payload_previews_on_a_char_boundary() {
    // Every char is 3 bytes, so PREVIEW_BYTES lands mid-character unless the
    // boundary is honored — a naive slice would panic here.
    let line = serde_json::json!({"text": "한".repeat(400)}).to_string();
    let slim: Value = serde_json::from_str(&elider().slim(&line)).expect("slim line is JSON");
    let elided = slim["text"].as_str().expect("still a string");
    assert!(elided.starts_with("한"), "preview kept: {elided}");
    assert!(elided.contains("@@acp-payload:"));
}
