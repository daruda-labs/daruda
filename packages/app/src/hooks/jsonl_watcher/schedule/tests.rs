use super::*;
use chrono::Duration as CDuration;
use std::cell::Cell;

fn start() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-10-26T00:00:00.125Z")
        .unwrap()
        .with_timezone(&Utc)
}

fn snapshot(root: &Path, stop: Option<&str>, text: &str, timestamp: DateTime<Utc>) -> Observation {
    let path = root.join("quiet.jsonl");
    let line = serde_json::json!({
        "type": "assistant", "uuid": "quiet", "timestamp": timestamp.to_rfc3339(),
        "message": { "model": "claude", "id": "m", "role": "assistant",
            "content": [{ "type": "text", "text": text }], "stop_reason": stop }
    });
    std::fs::write(&path, line.to_string()).unwrap();
    super::super::process_jsonl_file(
        &path,
        root,
        &PermissionChecker::default(),
        super::super::ReadStrategy::Full,
        timestamp,
    )
    .unwrap()
}

fn quiet_transition(stop: Option<&str>, text: &str, seconds: i64) -> Vec<JsonlEvent> {
    let root = tempfile::tempdir().unwrap();
    let initial = snapshot(root.path(), stop, text, start());
    let clock = Cell::new(start());
    let mut wakes = 0;
    let (tx, rx) = mpsc::channel();
    forward(
        vec![initial],
        &PermissionChecker::default(),
        tx,
        || clock.get(),
        |timeout| {
            wakes += 1;
            if wakes == 1 {
                assert_eq!(timeout, Some(Duration::from_secs(seconds as u64)));
                clock.set(start() + CDuration::seconds(seconds));
                Err(mpsc::RecvTimeoutError::Timeout)
            } else {
                assert_eq!(timeout, None, "settled input must stop scheduling timers");
                Err(mpsc::RecvTimeoutError::Disconnected)
            }
        },
        |_| panic!("a quiet timeout must not rescan the watched directories"),
    );
    assert_eq!(wakes, 2);
    rx.into_iter().collect()
}

#[test]
fn quiet_streaming_expires_without_another_file_event() {
    let events = quiet_transition(None, "partial", 5);
    assert_eq!(
        events.iter().map(|e| e.status).collect::<Vec<_>>(),
        [SessionStatus::Working, SessionStatus::Idle]
    );
    assert!(events.iter().all(|e| e.timestamp == start()));
}

#[test]
fn quiet_question_becomes_attention_without_another_file_event_and_cannot_override_hook() {
    use daruda_agent::hooks::status_file::{Source, StatusFile};
    let events = quiet_transition(Some("end_turn"), "Continue?", 20);
    assert_eq!(
        events.iter().map(|e| e.status).collect::<Vec<_>>(),
        [SessionStatus::Idle, SessionStatus::NeedsAttention]
    );
    assert!(events.iter().all(|e| e.timestamp == start()));
    let mut store = daruda_agent::ClaudeStatusStore::new();
    let mut hook = StatusFile::new_hook("quiet", &events[0].cwd, SessionStatus::Idle, "Stop");
    hook.timestamp = start() + CDuration::seconds(1);
    assert!(store.update(hook.clone()));
    for event in events {
        let incoming = StatusFile {
            status: event.status,
            timestamp: event.timestamp,
            source: Source::Jsonl,
            ..hook.clone()
        };
        assert!(!store.update(incoming));
    }
    assert_eq!(store.get("quiet").unwrap().status, SessionStatus::Idle);
}

#[test]
fn disconnect_cancels_pending_quiet_transitions() {
    let root = tempfile::tempdir().unwrap();
    let initial = snapshot(root.path(), None, "partial", start());
    let (tx, rx) = mpsc::channel();
    forward(
        vec![initial],
        &PermissionChecker::default(),
        tx,
        start,
        |timeout| {
            assert_eq!(timeout, Some(Duration::from_secs(5)));
            Err(mpsc::RecvTimeoutError::Disconnected)
        },
        |_| panic!("disconnect must not rescan"),
    );
    let events: Vec<_> = rx.into_iter().collect();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].status, SessionStatus::Working);
}

#[test]
fn new_file_snapshot_replaces_the_old_pending_deadline() {
    let root = tempfile::tempdir().unwrap();
    let initial = snapshot(root.path(), None, "partial", start());
    let replacement = snapshot(
        root.path(),
        Some("end_turn"),
        "done",
        start() + CDuration::seconds(1),
    );
    let mut replacement = Some(replacement);
    let clock = Cell::new(start());
    let (tx, rx) = mpsc::channel();
    forward(
        vec![initial],
        &PermissionChecker::default(),
        tx,
        || clock.get(),
        |timeout| {
            if let Some(replacement) = replacement.take() {
                assert_eq!(timeout, Some(Duration::from_secs(5)));
                clock.set(start() + CDuration::seconds(5));
                Ok(JsonlRaw::Event(replacement))
            } else {
                assert_eq!(timeout, None);
                Err(mpsc::RecvTimeoutError::Disconnected)
            }
        },
        |_| panic!("file events must not rescan"),
    );
    let events: Vec<_> = rx.into_iter().collect();
    assert_eq!(events.len(), 2);
    assert_eq!(events[1].timestamp, start() + CDuration::seconds(1));
    assert_eq!(events[1].status, SessionStatus::Idle);
}

#[test]
fn deleted_pending_file_cancels_its_deadline() {
    let root = tempfile::tempdir().unwrap();
    let initial = snapshot(root.path(), None, "partial", start());
    let path = initial.event.jsonl_path.clone();
    let mut removed = false;
    let (tx, rx) = mpsc::channel();
    forward(
        vec![initial],
        &PermissionChecker::default(),
        tx,
        start,
        |timeout| {
            if !removed {
                removed = true;
                assert_eq!(timeout, Some(Duration::from_secs(5)));
                Ok(JsonlRaw::Forget(path.clone()))
            } else {
                assert_eq!(timeout, None);
                Err(mpsc::RecvTimeoutError::Disconnected)
            }
        },
        |_| panic!("deletion must not rescan"),
    );
    assert_eq!(rx.into_iter().count(), 1);
}
