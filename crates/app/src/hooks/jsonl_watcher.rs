//! JSONL fallback watcher — engaged when hooks aren't installed.
//!
//! For each tracked lane, we watch
//! `~/.claude/projects/<encoded(cwd)>/` for modifications and emit a
//! [`JsonlEvent`] containing the inferred [`SessionStatus`]. The
//! Workspace pumps these into the same `ClaudeStatusStore` as hook
//! events with `source=Jsonl`; the store's race policy ensures hook
//! data wins on ties.
//!
//! Path encoding mirrors c9watch's `encode_path_for_matching` —
//! every non-alphanumeric char becomes `-`. Confirmed by the
//! `transcript_path` field in real hook payloads.
//!
//! Lifecycle: [`spawn`] returns a [`JsonlWatcherHandle`] holding the event
//! receiver plus a [`crate::dir_watch::DirWatcher`]. Dropping the handle
//! (typically when a Workspace re-spawns the watcher after a lane change, or
//! on teardown) drops the `DirWatcher` — unregistering FSEvents — which
//! disconnects the raw channel and ends the forward thread.

use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};

use chrono::{DateTime, Utc};
use daruda_claude::SessionStatus;
use daruda_claude::jsonl::fsm::{determine_status, last_meaningful_timestamp};
use daruda_claude::jsonl::parser::parse_jsonl_entries;
use daruda_claude::jsonl::permissions::PermissionChecker;
use daruda_claude::jsonl::tail::read_last_n_lines;
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;
use daruda_store::observability::system_info::redact_home;

/// Tail length read for FSEvent-driven fires. Mirrors c9watch's
/// `parse_last_n_entries(20)` — enough recent entries for the status
/// FSM to classify the session.
const TAIL_ENTRIES: usize = 20;

/// One inferred status update from a JSONL change.
#[derive(Clone, Debug)]
pub struct JsonlEvent {
    pub session_id: String,
    pub cwd: PathBuf,
    pub jsonl_path: PathBuf,
    pub status: SessionStatus,
    pub timestamp: DateTime<Utc>,
}

/// Encode a path the way Claude Code does for its project directory
/// names: every non-alphanumeric ASCII char becomes `-`.
pub fn encode_path_for_claude(path: &str) -> String {
    path.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// Resolve `~/.claude/projects/<encoded(cwd)>/` for a lane path.
pub fn project_dir_for(home: &Path, worktree_path: &Path) -> PathBuf {
    let cwd_str = worktree_path.to_string_lossy();
    let encoded = encode_path_for_claude(&cwd_str);
    home.join(".claude").join("projects").join(encoded)
}

/// Handle that keeps the jsonl watcher alive. Dropping `watcher` stops the
/// [`crate::dir_watch::DirWatcher`] (FSEvents subscriptions unregister), which
/// disconnects the raw channel and ends the forward/cold-restore thread. Hold
/// it for the lifetime you want the watcher running; drop it to refresh or
/// tear down.
pub struct JsonlWatcherHandle {
    pub events: mpsc::Receiver<JsonlEvent>,
    pub watcher: crate::dir_watch::DirWatcher,
}

/// Spawn the jsonl watcher across `lane_dirs`. Each entry pairs a
/// lane's path (used as `cwd` in emitted events) with the
/// `~/.claude/projects/<encoded>/` directory to watch.
///
/// The watcher reloads `~/.claude/settings.json` permissions on
/// startup; live permission changes are not reflected until restart
/// (acceptable for the fallback path; the hook channel is real-time
/// for active permission events anyway).
/// Raw item from the watcher: a per-file event (already read, cheap Tail) or a
/// rescan signal whose heavy full re-read is deferred to the forward thread.
enum JsonlRaw {
    Event(JsonlEvent),
    Rescan,
}

pub fn spawn(lane_dirs: Vec<(PathBuf, PathBuf)>) -> JsonlWatcherHandle {
    use notify::{EventKind, RecursiveMode};

    // `Arc` so the classify closure and the forward thread (cold-restore +
    // rescan re-reads) can share one `PermissionChecker`.
    let permissions = Arc::new(PermissionChecker::from_settings_file());
    let (events_tx, events_rx) = mpsc::channel::<JsonlEvent>();

    // Watch each lane's `~/.claude/projects/<encoded>/` dir; mkdir first so
    // notify can attach on a fresh install.
    let mut anchors: Vec<PathBuf> = Vec::new();
    for (_cwd, dir) in &lane_dirs {
        if let Err(e) = std::fs::create_dir_all(dir) {
            LogWriter::log(
                ErrorReport::new("jsonl watcher mkdir failed")
                    .severity(ErrorSeverity::Warning)
                    .from_error(&e)
                    .at(file!(), line!())
                    .with_context("path", redact_home(dir))
                    .dedup("jsonl.watcher.mkdir")
                    .build(),
            );
        }
        anchors.push(dir.clone());
    }

    // `notify::Event` gives absolute file paths; the closure looks up the
    // parent dir to attach the right cwd. A live fire reads only the tail
    // (cheap) — fine to run on notify's callback thread. A rescan, by
    // contrast, would re-read every file in full; emit a cheap `Rescan`
    // signal here and do that heavy walk on the forward thread instead, so
    // sleep/wake recovery never blocks notify's event dispatch.
    let lookup = lane_dirs.clone();
    let permissions_classify = permissions.clone();
    let classify = move |event: &notify::Event| {
        if !matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_)) {
            return Vec::new();
        }
        let mut out = Vec::new();
        for path in &event.paths {
            let Some(parent) = path.parent() else {
                continue;
            };
            let Some((cwd, _)) = lookup.iter().find(|(_, dir)| dir == parent) else {
                continue;
            };
            if let Some(ev) = process_jsonl_file(
                path,
                cwd,
                &permissions_classify,
                ReadStrategy::Tail(TAIL_ENTRIES),
            ) {
                out.push(JsonlRaw::Event(ev));
            }
        }
        out
    };
    let rescan = || vec![JsonlRaw::Rescan];

    let (raw_rx, watcher) = crate::dir_watch::spawn_dir_watcher(
        &anchors,
        RecursiveMode::NonRecursive,
        classify,
        rescan,
    );

    // Forward thread, off notify's callback thread: first the startup
    // cold-restore walk (so the caller isn't blocked), then forward live
    // events and service rescan signals with the full re-read. The
    // subscription is already live, so events arriving during the walk buffer
    // in `raw_rx` and are forwarded after; the store's race policy orders any
    // duplicates.
    std::thread::spawn(move || {
        for ev in enumerate_jsonl(&lane_dirs, &permissions) {
            if events_tx.send(ev).is_err() {
                return;
            }
        }
        while let Ok(raw) = raw_rx.recv() {
            match raw {
                JsonlRaw::Event(ev) => {
                    if events_tx.send(ev).is_err() {
                        break;
                    }
                }
                JsonlRaw::Rescan => {
                    for ev in enumerate_jsonl(&lane_dirs, &permissions) {
                        if events_tx.send(ev).is_err() {
                            return;
                        }
                    }
                }
            }
        }
    });

    JsonlWatcherHandle {
        events: events_rx,
        watcher,
    }
}

/// How `process_jsonl_file` reads a file's lines — memory vs completeness:
/// - [`ReadStrategy::Tail`]: cheap byte-seek near the end via
///   `read_last_n_lines`, for FSEvent fires that append a few lines.
///   The seek can land mid-UTF-8 and stop the iterator early — fine for
///   the latest entries, brittle for deep history.
/// - [`ReadStrategy::Full`]: `read_to_string` + split; one file-sized
///   allocation but every line is returned. Used at cold-restore so
///   non-ASCII history isn't silently dropped.
#[derive(Clone, Copy, Debug)]
enum ReadStrategy {
    Tail(usize),
    Full,
}

/// Read `path` according to `strategy`, returning a vector of
/// non-blank lines.
fn read_jsonl_lines(path: &Path, strategy: ReadStrategy) -> std::io::Result<Vec<String>> {
    match strategy {
        ReadStrategy::Tail(n) => read_last_n_lines(path, n),
        ReadStrategy::Full => {
            let content = std::fs::read_to_string(path)?;
            Ok(content
                .lines()
                .filter(|l| !l.trim().is_empty())
                .map(String::from)
                .collect())
        }
    }
}

/// Read one jsonl file and turn it into a `JsonlEvent`. Returns
/// `None` for non-`.jsonl` extensions, the `agent-*` subagent
/// pattern, unreadable files, or empty files (no parsed entries).
/// See [`ReadStrategy`] for `strategy`.
fn process_jsonl_file(
    path: &Path,
    cwd: &Path,
    permissions: &PermissionChecker,
    strategy: ReadStrategy,
) -> Option<JsonlEvent> {
    if path.extension().is_none_or(|e| e != "jsonl") {
        return None;
    }
    let stem = path.file_stem()?.to_str()?.to_string();
    // Skip subagent files (`agent-*.jsonl`).
    if stem.starts_with("agent-") {
        return None;
    }
    let lines = read_jsonl_lines(path, strategy).ok()?;
    let entries = parse_jsonl_entries(&lines);
    if entries.is_empty() {
        return None;
    }
    let status = determine_status(&entries, permissions);
    // Use the last meaningful entry's own timestamp so the store's
    // race policy correctly orders JSONL against hook events.
    let timestamp = last_meaningful_timestamp(&entries).unwrap_or_else(Utc::now);
    Some(JsonlEvent {
        session_id: stem,
        cwd: cwd.to_path_buf(),
        jsonl_path: path.to_path_buf(),
        status,
        timestamp,
    })
}

/// Walk every watched directory once and return a `JsonlEvent` for each
/// existing jsonl file. Used for the startup cold-restore (so per-lane
/// indicators populate immediately) and for FSEvents-drop rescan recovery.
/// Uses [`ReadStrategy::Full`] so non-ASCII history isn't silently dropped by
/// the byte-seek tail reader. Errors on individual files (read failures,
/// malformed JSONL) are swallowed so one bad session doesn't block the rest.
fn enumerate_jsonl(
    lane_dirs: &[(PathBuf, PathBuf)],
    permissions: &PermissionChecker,
) -> Vec<JsonlEvent> {
    let mut out = Vec::new();
    for (cwd, dir) in lane_dirs {
        let read_dir = match std::fs::read_dir(dir) {
            Ok(rd) => rd,
            Err(_) => continue,
        };
        for entry in read_dir.flatten() {
            let path = entry.path();
            if let Some(ev) = process_jsonl_file(&path, cwd, permissions, ReadStrategy::Full) {
                out.push(ev);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
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
        assert_eq!(
            dir.to_str().unwrap(),
            "/Users/x/.claude/projects/-Users-x-proj"
        );
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
            process_jsonl_file(&path, tmp.path(), &perms, ReadStrategy::Tail(TAIL_ENTRIES))
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
            process_jsonl_file(&path, tmp.path(), &perms, ReadStrategy::Tail(TAIL_ENTRIES))
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

        let event = process_jsonl_file(&path, &cwd, &perms, ReadStrategy::Full)
            .expect("should produce an event");
        assert_eq!(event.session_id, "abc12345");
        assert_eq!(event.cwd, cwd);
        assert_eq!(event.jsonl_path, path);
    }

    #[test]
    fn process_jsonl_file_returns_none_for_empty_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write_jsonl(tmp.path(), "empty", &[]);
        let perms = PermissionChecker::default();
        assert!(
            process_jsonl_file(&path, tmp.path(), &perms, ReadStrategy::Tail(TAIL_ENTRIES))
                .is_none(),
            "empty file produces no entries"
        );
    }
}
