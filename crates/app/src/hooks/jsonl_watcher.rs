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
//! Lifecycle is shutdown-receiver based: the spawn function returns
//! `(shutdown_tx, event_rx)`; dropping `shutdown_tx` (typically when
//! a Workspace re-spawns the watcher after a lane change, or on
//! Workspace teardown) wakes the watcher thread out of its blocking
//! `recv` and lets the `notify::Watcher` drop, unregistering FSEvents
//! cleanly. There is no `loop { park() }` — that pattern would leak
//! one thread per re-spawn.

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

/// Handle that keeps the jsonl watcher thread alive. Dropping the
/// `shutdown_tx` half closes the worker thread cleanly:
/// `recv()` on its shutdown receiver returns `Err(Disconnected)` →
/// the `notify::Watcher` is dropped → FSEvents subscriptions are
/// unregistered. Hold the sender for the lifetime you want the
/// watcher to keep running; drop it to refresh or tear down.
pub struct JsonlWatcherHandle {
    pub shutdown_tx: mpsc::Sender<()>,
    pub events: mpsc::Receiver<JsonlEvent>,
}

/// Spawn the jsonl watcher across `lane_dirs`. Each entry pairs a
/// lane's path (used as `cwd` in emitted events) with the
/// `~/.claude/projects/<encoded>/` directory to watch.
///
/// The watcher reloads `~/.claude/settings.json` permissions on
/// startup; live permission changes are not reflected until restart
/// (acceptable for the fallback path; the hook channel is real-time
/// for active permission events anyway).
pub fn spawn(lane_dirs: Vec<(PathBuf, PathBuf)>) -> JsonlWatcherHandle {
    use notify::{EventKind, RecursiveMode, Watcher};

    let (tx, rx) = mpsc::channel();
    let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>();

    std::thread::spawn(move || {
        // Wrap `PermissionChecker` in `Arc` so both the FSEvent
        // callback (a `Fn` closure that captures by move) and the
        // cold-restore loop (running here in the worker thread
        // *after* the closure has captured) can share it.
        let permissions = Arc::new(PermissionChecker::from_settings_file());
        // Map watched dir → owning lane cwd. `notify::Event`
        // gives us absolute file paths; the closure looks up the
        // parent dir here to attach the right cwd to the emitted event.
        let lookup: Vec<(PathBuf, PathBuf)> = lane_dirs.clone();

        let tx_inner = tx.clone();
        let permissions_inner = permissions.clone();
        let mut watcher =
            match notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
                let Ok(event) = res else {
                    return;
                };
                if !matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_)) {
                    return;
                }
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
                        &permissions_inner,
                        ReadStrategy::Tail(TAIL_ENTRIES),
                    ) {
                        let _ = tx_inner.send(ev);
                    }
                }
            }) {
                Ok(w) => w,
                Err(_) => return,
            };

        // Subscribe before the cold-restore walk so any modify events
        // that happen during cold-restore are still picked up — the
        // store's race policy orders duplicate status updates.
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
            if let Err(e) = watcher.watch(dir, RecursiveMode::NonRecursive) {
                LogWriter::log(
                    ErrorReport::new("jsonl watcher subscribe failed")
                        .severity(ErrorSeverity::Warning)
                        .from_error(&e)
                        .at(file!(), line!())
                        .with_context("path", redact_home(dir))
                        .dedup("jsonl.watcher.subscribe")
                        .build(),
                );
            }
        }

        // Cold-restore: emit one synthetic JsonlEvent per existing
        // jsonl file so per-lane session indicators populate
        // immediately on launch + after every refresh_jsonl_watcher
        // restart, instead of staying empty until the next FSEvent
        // fires.
        cold_restore(&lane_dirs, &permissions, &tx);

        // Block until the caller drops `shutdown_tx`. `recv()` on a
        // disconnected channel returns immediately, so this sleeps
        // for the watcher's full lifetime without a poll loop.
        let _ = shutdown_rx.recv();
        // `watcher` drops here — notify backend unregisters watches.
    });

    JsonlWatcherHandle {
        shutdown_tx,
        events: rx,
    }
}

/// How a `process_jsonl_file` caller wants the file's lines read.
///
/// The two strategies trade memory for completeness:
/// - [`ReadStrategy::Tail`]: byte-seek to a chunk near the end, read
///   forward via `read_last_n_lines`. Cheap; the right choice for
///   FSEvent fires where each modification typically appends just
///   a handful of lines. **Caveat:** the seek can land mid-UTF-8,
///   in which case `BufReader::lines()` errors out and stops the
///   iterator early — fine for the latest 20 entries, brittle for
///   deeper history.
/// - [`ReadStrategy::Full`]: `read_to_string` then split — uses one
///   string allocation the size of the file, but every line is
///   returned. Used at cold-restore so historical sessions that
///   contain non-ASCII content don't get silently dropped.
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
///
/// The `strategy` parameter picks `Tail(N)` for FSEvent-driven
/// fires (cheap, latest entries only) and `Full` for the one-shot
/// startup walk where every historical entry matters and the file
/// might contain non-ASCII content the seek-and-parse path can't
/// safely chunk.
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

/// Walk every watched directory once and emit a `JsonlEvent` for
/// each existing jsonl file. Run after `watcher.watch(...)` so any
/// fs activity during the walk is also captured by FSEvents. Uses
/// [`ReadStrategy::Full`] so non-ASCII history isn't silently
/// dropped by the byte-seek tail reader. Errors on individual
/// files (read failures, malformed JSONL) are swallowed so one bad
/// session doesn't block the rest of the cold restore.
fn cold_restore(
    lane_dirs: &[(PathBuf, PathBuf)],
    permissions: &PermissionChecker,
    tx: &mpsc::Sender<JsonlEvent>,
) {
    for (cwd, dir) in lane_dirs {
        let read_dir = match std::fs::read_dir(dir) {
            Ok(rd) => rd,
            Err(_) => continue,
        };
        for entry in read_dir.flatten() {
            let path = entry.path();
            if let Some(ev) = process_jsonl_file(&path, cwd, permissions, ReadStrategy::Full) {
                let _ = tx.send(ev);
            }
        }
    }
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
