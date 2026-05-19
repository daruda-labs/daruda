//! JSONL fallback watcher — engaged when hooks aren't installed.
//!
//! For each tracked worktree, we watch
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
//! a Workspace re-spawns the watcher after a worktree change, or on
//! Workspace teardown) wakes the watcher thread out of its blocking
//! `recv` and lets the `notify::Watcher` drop, unregistering FSEvents
//! cleanly. There is no `loop { park() }` — that pattern would leak
//! one thread per re-spawn.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};

use chrono::{DateTime, Utc};
use daruda_claude::SessionStatus;
use daruda_claude::jsonl::fsm::{determine_status, last_meaningful_timestamp};
use daruda_claude::jsonl::parser::{SessionEntry, parse_jsonl_entries};
use daruda_claude::jsonl::permissions::PermissionChecker;
use daruda_claude::jsonl::tail::read_last_n_lines;
use daruda_claude::usage::UsageDelta;
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;
use daruda_store::observability::system_info::redact_home;

/// Tail length read for FSEvent-driven fires. Mirrors c9watch's
/// `parse_last_n_entries(20)`. Brief bursts of `> 20` assistant
/// messages between two fires can drop a handful of usage updates;
/// in practice each filesystem write fires the watcher before more
/// than a few messages stack up.
const TAIL_ENTRIES: usize = 20;

/// One inferred status update from a JSONL change.
///
/// `usage` is `Some` only when the fire surfaced new assistant
/// entries with a `usage` block — by-design idempotent across fires
/// thanks to the watcher's per-session uuid tracker.
#[derive(Clone, Debug)]
pub struct JsonlEvent {
    pub session_id: String,
    pub cwd: PathBuf,
    pub jsonl_path: PathBuf,
    pub status: SessionStatus,
    pub timestamp: DateTime<Utc>,
    /// Aggregated usage from assistant entries new to this watcher
    /// since the previous fire (or the entire session history on the
    /// first fire). `None` when no new usage-bearing entries were
    /// found, so consumers can skip an `update_usage` call entirely.
    pub usage: Option<UsageDelta>,
}

/// Per-session set of assistant uuids the watcher has already
/// folded into a `UsageDelta`. Threaded through every fire so each
/// uuid contributes its usage exactly once across the watcher's
/// lifetime.
type UsageTracker = HashMap<String, HashSet<String>>;

/// Walk parsed `entries` and accumulate the usage of every assistant
/// entry whose uuid has not yet been seen for `session_id`. Inserts
/// the new uuids into `tracker` as a side effect, providing
/// at-most-once emission per (session, uuid).
///
/// The `usage` check happens **before** `seen.insert` so an assistant
/// entry that lands without a `usage` block (rare but possible during
/// streaming or partial writes) does not poison the tracker; when a
/// later fire sees the same uuid with usage attached, that fire still
/// emits.
///
/// Returns `None` when no new usage-bearing entries are found, so
/// callers can suppress empty `update_usage` calls.
fn extract_new_usage_delta(
    entries: &[SessionEntry],
    session_id: &str,
    tracker: &mut UsageTracker,
) -> Option<UsageDelta> {
    let seen = tracker.entry(session_id.to_string()).or_default();
    let mut total = UsageDelta::default();
    let mut emitted = false;
    for entry in entries {
        if let SessionEntry::Assistant { base, message } = entry
            && let Some(u) = &message.usage
            && seen.insert(base.uuid.clone())
        {
            total.add_assign(&UsageDelta::from_jsonl_usage(u));
            emitted = true;
        }
    }
    emitted.then_some(total)
}

/// Encode a path the way Claude Code does for its project directory
/// names: every non-alphanumeric ASCII char becomes `-`.
pub fn encode_path_for_claude(path: &str) -> String {
    path.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// Resolve `~/.claude/projects/<encoded(cwd)>/` for a worktree path.
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

/// Spawn the jsonl watcher across `worktree_dirs`. Each entry pairs a
/// worktree's path (used as `cwd` in emitted events) with the
/// `~/.claude/projects/<encoded>/` directory to watch.
///
/// The watcher reloads `~/.claude/settings.json` permissions on
/// startup; live permission changes are not reflected until restart
/// (acceptable for the fallback path; the hook channel is real-time
/// for active permission events anyway).
pub fn spawn(worktree_dirs: Vec<(PathBuf, PathBuf)>) -> JsonlWatcherHandle {
    use notify::{EventKind, RecursiveMode, Watcher};

    let (tx, rx) = mpsc::channel();
    let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>();

    std::thread::spawn(move || {
        // Wrap `PermissionChecker` and the uuid tracker in `Arc` so
        // both the FSEvent callback (a `Fn` closure that captures by
        // move) and the cold-restore loop (running here in the worker
        // thread *after* the closure has captured) can share state.
        let permissions = Arc::new(PermissionChecker::from_settings_file());
        let usage_tracker: Arc<Mutex<UsageTracker>> = Arc::new(Mutex::new(HashMap::new()));
        // Map watched dir → owning worktree cwd. `notify::Event`
        // gives us absolute file paths; the closure looks up the
        // parent dir here to attach the right cwd to the emitted event.
        let lookup: Vec<(PathBuf, PathBuf)> = worktree_dirs.clone();

        let tx_inner = tx.clone();
        let permissions_inner = permissions.clone();
        let tracker_inner = usage_tracker.clone();
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
                        &tracker_inner,
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
        // tracker dedupes shared uuids between the two paths.
        for (_cwd, dir) in &worktree_dirs {
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
        // jsonl file so the Usage tab populates immediately on
        // launch + after every refresh_jsonl_watcher restart, instead
        // of staying empty until the next FSEvent fires.
        cold_restore(&worktree_dirs, &permissions, &usage_tracker, &tx);

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
///
/// Shares `permissions` and `usage_tracker` with the FSEvent path so
/// dedup across the two channels is automatic.
fn process_jsonl_file(
    path: &Path,
    cwd: &Path,
    permissions: &PermissionChecker,
    usage_tracker: &Mutex<UsageTracker>,
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
    let usage = {
        let Ok(mut tracker) = usage_tracker.lock() else {
            LogWriter::log(
                ErrorReport::new("usage tracker mutex poisoned; skipping JSONL event")
                    .at(file!(), line!())
                    .dedup("jsonl.usage_tracker.poisoned")
                    .build(),
            );
            return None;
        };
        extract_new_usage_delta(&entries, &stem, &mut tracker)
    };
    Some(JsonlEvent {
        session_id: stem,
        cwd: cwd.to_path_buf(),
        jsonl_path: path.to_path_buf(),
        status,
        timestamp,
        usage,
    })
}

/// Walk every watched directory once and emit a `JsonlEvent` for
/// each existing jsonl file. Run after `watcher.watch(...)` so any
/// fs activity during the walk is also captured by FSEvents (the
/// tracker dedupes uuids shared between the two paths). Uses
/// [`ReadStrategy::Full`] so non-ASCII history isn't silently
/// dropped by the byte-seek tail reader. Errors on individual
/// files (read failures, malformed JSONL) are swallowed so one bad
/// session doesn't block the rest of the cold restore.
fn cold_restore(
    worktree_dirs: &[(PathBuf, PathBuf)],
    permissions: &PermissionChecker,
    usage_tracker: &Mutex<UsageTracker>,
    tx: &mpsc::Sender<JsonlEvent>,
) {
    for (cwd, dir) in worktree_dirs {
        let read_dir = match std::fs::read_dir(dir) {
            Ok(rd) => rd,
            Err(_) => continue,
        };
        for entry in read_dir.flatten() {
            let path = entry.path();
            if let Some(ev) =
                process_jsonl_file(&path, cwd, permissions, usage_tracker, ReadStrategy::Full)
            {
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

    /// Build an `assistant` JSONL line with a known uuid and a
    /// `usage` block. Single-line output so file-based tests can read
    /// it back through `read_last_n_lines` (which splits on `\n`).
    fn assistant_line(uuid: &str, input: u32, output: u32) -> String {
        format!(
            r#"{{"type":"assistant","uuid":"{uuid}","timestamp":"2026-05-01T00:00:00Z","message":{{"model":"claude-x","id":"m-{uuid}","role":"assistant","content":[{{"type":"text","text":"hi"}}],"stop_reason":"end_turn","usage":{{"input_tokens":{input},"output_tokens":{output},"cache_creation_input_tokens":0,"cache_read_input_tokens":0}}}}}}"#
        )
    }

    fn user_line(uuid: &str) -> String {
        format!(
            r#"{{"type":"user","uuid":"{uuid}","timestamp":"2026-05-01T00:00:00Z","message":{{"role":"user","content":"hi"}}}}"#
        )
    }

    /// Assistant entry with the `usage` field omitted — simulates a
    /// streamed write that landed before the usage tally was attached.
    /// `AssistantMessage::usage: Option<Usage>` decodes the missing key
    /// as `None`.
    fn assistant_line_no_usage(uuid: &str) -> String {
        format!(
            r#"{{"type":"assistant","uuid":"{uuid}","timestamp":"2026-05-01T00:00:00Z","message":{{"model":"claude-x","id":"m-{uuid}","role":"assistant","content":[{{"type":"text","text":"hi"}}],"stop_reason":"end_turn"}}}}"#
        )
    }

    #[test]
    fn extract_new_usage_delta_first_call_aggregates_all_assistant_usage() {
        let lines = vec![
            user_line("u1"),
            assistant_line("a1", 100, 50),
            user_line("u2"),
            assistant_line("a2", 200, 80),
        ];
        let entries = parse_jsonl_entries(&lines);
        let mut tracker = UsageTracker::new();
        let delta = extract_new_usage_delta(&entries, "sess-1", &mut tracker).unwrap();
        assert_eq!(delta.input_tokens, 300);
        assert_eq!(delta.output_tokens, 130);
        assert_eq!(tracker["sess-1"].len(), 2);
    }

    #[test]
    fn extract_new_usage_delta_dedupes_already_seen_uuids() {
        let lines = vec![assistant_line("a1", 100, 50)];
        let entries = parse_jsonl_entries(&lines);
        let mut tracker = UsageTracker::new();
        // First fire — emits.
        let first = extract_new_usage_delta(&entries, "sess-1", &mut tracker);
        assert!(first.is_some());
        // Second fire on the exact same tail — emits nothing.
        let second = extract_new_usage_delta(&entries, "sess-1", &mut tracker);
        assert!(second.is_none());
    }

    #[test]
    fn extract_new_usage_delta_only_emits_new_uuids_on_subsequent_calls() {
        let mut tracker = UsageTracker::new();

        // First fire — one assistant entry.
        let first_lines = vec![assistant_line("a1", 100, 50)];
        let first_entries = parse_jsonl_entries(&first_lines);
        let first = extract_new_usage_delta(&first_entries, "sess-1", &mut tracker).unwrap();
        assert_eq!(first.input_tokens, 100);

        // Second fire — the original line plus a new one. Only the
        // new uuid contributes.
        let second_lines = vec![assistant_line("a1", 100, 50), assistant_line("a2", 25, 10)];
        let second_entries = parse_jsonl_entries(&second_lines);
        let second = extract_new_usage_delta(&second_entries, "sess-1", &mut tracker).unwrap();
        assert_eq!(second.input_tokens, 25);
        assert_eq!(second.output_tokens, 10);
    }

    #[test]
    fn extract_new_usage_delta_returns_none_on_only_user_entries() {
        let lines = vec![user_line("u1"), user_line("u2")];
        let entries = parse_jsonl_entries(&lines);
        let mut tracker = UsageTracker::new();
        assert!(extract_new_usage_delta(&entries, "sess-1", &mut tracker).is_none());
        // Tracker stayed empty — no spurious entries created.
        assert!(tracker.get("sess-1").is_none_or(|s| s.is_empty()));
    }

    #[test]
    fn extract_new_usage_delta_keeps_sessions_separate() {
        let lines = vec![assistant_line("a1", 100, 50)];
        let entries = parse_jsonl_entries(&lines);
        let mut tracker = UsageTracker::new();
        let _ = extract_new_usage_delta(&entries, "sess-A", &mut tracker);
        // Same uuid under a different session id is a different entry.
        let other = extract_new_usage_delta(&entries, "sess-B", &mut tracker).unwrap();
        assert_eq!(other.input_tokens, 100);
    }

    /// Helper for `process_jsonl_file` tests — write `lines` (each
    /// already JSON-encoded) into a freshly-named jsonl inside the
    /// given `dir` and return the path.
    fn write_jsonl(dir: &Path, stem: &str, lines: &[String]) -> PathBuf {
        let path = dir.join(format!("{stem}.jsonl"));
        std::fs::write(&path, lines.join("\n")).expect("write jsonl");
        path
    }

    fn fresh_tracker() -> Mutex<UsageTracker> {
        Mutex::new(HashMap::new())
    }

    #[test]
    fn process_jsonl_file_skips_non_jsonl_extension() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("notes.txt");
        std::fs::write(&path, "irrelevant").unwrap();
        let perms = PermissionChecker::default();
        let tracker = fresh_tracker();
        assert!(
            process_jsonl_file(
                &path,
                tmp.path(),
                &perms,
                &tracker,
                ReadStrategy::Tail(TAIL_ENTRIES)
            )
            .is_none(),
            "non-jsonl extension must be skipped"
        );
    }

    #[test]
    fn process_jsonl_file_skips_agent_prefix() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write_jsonl(tmp.path(), "agent-subagent", &[assistant_line("a1", 10, 5)]);
        let perms = PermissionChecker::default();
        let tracker = fresh_tracker();
        assert!(
            process_jsonl_file(
                &path,
                tmp.path(),
                &perms,
                &tracker,
                ReadStrategy::Tail(TAIL_ENTRIES)
            )
            .is_none(),
            "agent-* files must be skipped"
        );
    }

    #[test]
    fn process_jsonl_file_returns_event_with_aggregated_usage() {
        let tmp = tempfile::tempdir().unwrap();
        let lines = vec![
            user_line("u1"),
            assistant_line("a1", 100, 50),
            user_line("u2"),
            assistant_line("a2", 200, 80),
        ];
        let path = write_jsonl(tmp.path(), "abc12345", &lines);
        let perms = PermissionChecker::default();
        let tracker = fresh_tracker();
        let cwd = PathBuf::from("/tmp/repo");

        let event = process_jsonl_file(&path, &cwd, &perms, &tracker, ReadStrategy::Full)
            .expect("should produce an event");
        assert_eq!(event.session_id, "abc12345");
        assert_eq!(event.cwd, cwd);
        assert_eq!(event.jsonl_path, path);
        let usage = event.usage.expect("usage delta present");
        assert_eq!(usage.input_tokens, 300);
        assert_eq!(usage.output_tokens, 130);
    }

    #[test]
    fn process_jsonl_file_returns_none_for_empty_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write_jsonl(tmp.path(), "empty", &[]);
        let perms = PermissionChecker::default();
        let tracker = fresh_tracker();
        assert!(
            process_jsonl_file(
                &path,
                tmp.path(),
                &perms,
                &tracker,
                ReadStrategy::Tail(TAIL_ENTRIES)
            )
            .is_none(),
            "empty file produces no entries"
        );
    }

    #[test]
    fn process_jsonl_file_full_strategy_reads_every_line_in_a_large_file() {
        // Regression: the seek-based `Tail` strategy can land
        // mid-UTF-8 in big files and abort `BufReader::lines()` on
        // the first invalid chunk, dropping nearly every entry. The
        // cold-restore path uses `Full`, which must always return
        // every assistant entry.
        let tmp = tempfile::tempdir().unwrap();
        let mut lines = Vec::with_capacity(50);
        for i in 0..50 {
            lines.push(assistant_line(&format!("a{i:04}"), 100, 50));
        }
        let path = write_jsonl(tmp.path(), "big-session", &lines);
        let perms = PermissionChecker::default();
        let tracker = fresh_tracker();

        let event =
            process_jsonl_file(&path, tmp.path(), &perms, &tracker, ReadStrategy::Full).unwrap();
        let usage = event.usage.unwrap();
        // Every assistant entry contributes 100 + 50 tokens, so the
        // aggregate must reflect all 50 messages.
        assert_eq!(usage.input_tokens, 50 * 100);
        assert_eq!(usage.output_tokens, 50 * 50);
    }

    #[test]
    fn process_jsonl_file_dedupes_across_repeat_calls() {
        // Two reads of the same file — second call should see no new
        // assistant uuids and emit `usage = None`. The status field
        // still reports because `process_jsonl_file` always
        // recomputes it; usage is the only field gated by the tracker.
        let tmp = tempfile::tempdir().unwrap();
        let path = write_jsonl(tmp.path(), "repeat-test", &[assistant_line("a1", 100, 50)]);
        let perms = PermissionChecker::default();
        let tracker = fresh_tracker();
        let cwd = PathBuf::from("/tmp/r");

        let first = process_jsonl_file(
            &path,
            &cwd,
            &perms,
            &tracker,
            ReadStrategy::Tail(TAIL_ENTRIES),
        )
        .unwrap();
        assert!(first.usage.is_some());

        let second = process_jsonl_file(
            &path,
            &cwd,
            &perms,
            &tracker,
            ReadStrategy::Tail(TAIL_ENTRIES),
        )
        .unwrap();
        assert!(second.usage.is_none());
    }

    #[test]
    fn extract_new_usage_delta_does_not_poison_tracker_for_usage_less_assistant() {
        // First fire — assistant entry has no usage block yet.
        let v1 = vec![assistant_line_no_usage("a1")];
        let mut tracker = UsageTracker::new();
        assert!(
            extract_new_usage_delta(&parse_jsonl_entries(&v1), "sess-1", &mut tracker).is_none(),
            "no usage means no emit"
        );
        // The tracker must not have recorded the uuid — otherwise a
        // later fire with the usage attached would silently drop it.
        assert!(tracker.get("sess-1").is_none_or(|s| !s.contains("a1")));

        // Second fire — same uuid, now with usage attached.
        let v2 = vec![assistant_line("a1", 100, 50)];
        let delta = extract_new_usage_delta(&parse_jsonl_entries(&v2), "sess-1", &mut tracker)
            .expect("usage should now be captured");
        assert_eq!(delta.input_tokens, 100);
        assert_eq!(delta.output_tokens, 50);
    }
}
