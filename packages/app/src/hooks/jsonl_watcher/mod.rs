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
use std::sync::mpsc;

mod schedule;

use chrono::{DateTime, Utc};
use daruda_agent::SessionStatus;
use daruda_agent::jsonl::fsm::{classify_status_at, last_meaningful_timestamp};
use daruda_agent::jsonl::parser::{SessionEntry, parse_jsonl_entries};
use daruda_agent::jsonl::permissions::PermissionChecker;
use daruda_agent::jsonl::tail::read_last_n_lines;
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
    Event(Observation),
    Forget(PathBuf),
    Rescan,
}

pub fn spawn(lane_dirs: Vec<(PathBuf, PathBuf)>) -> JsonlWatcherHandle {
    use notify::{EventKind, RecursiveMode};

    // Capture one permission snapshot for live and scheduled classification.
    let permissions = PermissionChecker::from_settings_file();
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
        if !matches!(
            event.kind,
            EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
        ) {
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
            if matches!(event.kind, EventKind::Remove(_)) {
                out.push(JsonlRaw::Forget(path.clone()));
                continue;
            }
            if let Some(ev) = process_jsonl_file(
                path,
                cwd,
                &permissions_classify,
                ReadStrategy::Tail(TAIL_ENTRIES),
                Utc::now(),
            ) {
                out.push(JsonlRaw::Event(ev));
            } else {
                out.push(JsonlRaw::Forget(path.clone()));
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
        let initial = enumerate_jsonl(&lane_dirs, &permissions, Utc::now());
        schedule::forward(
            initial,
            &permissions,
            events_tx,
            Utc::now,
            |timeout| match timeout {
                Some(timeout) => raw_rx.recv_timeout(timeout),
                None => raw_rx
                    .recv()
                    .map_err(|_| mpsc::RecvTimeoutError::Disconnected),
            },
            |now| enumerate_jsonl(&lane_dirs, &permissions, now),
        );
    });

    JsonlWatcherHandle {
        events: events_rx,
        watcher,
    }
}

struct Observation {
    event: JsonlEvent,
    entries: Vec<SessionEntry>,
}

/// How `process_jsonl_file` reads a file's lines — memory vs completeness:
/// - [`ReadStrategy::Tail`]: cheap byte-seek near the end via
///   `read_last_n_lines`, for FSEvent fires that append a few lines.
///   It reads a bounded window (`n` × 2 KB), drops the window's torn first
///   line and skips any line that is not UTF-8, so it returns only whole
///   lines — but only as many as fit the window, which suits the latest
///   entries and not deep history.
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
    now: DateTime<Utc>,
) -> Option<Observation> {
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
    // Classification reads only the last meaningful entry and whether progress
    // follows it. Pending timers must not retain a full startup transcript.
    let entries = if let Some(idx) = entries.iter().rposition(|entry| {
        matches!(
            entry,
            SessionEntry::User { .. } | SessionEntry::Assistant { .. }
        )
    }) {
        let progress = entries[idx + 1..]
            .iter()
            .any(|entry| matches!(entry, SessionEntry::Unknown));
        let mut compact = Vec::new();
        if let Some(latest) = entries.into_iter().nth(idx) {
            compact.push(latest);
        }
        if progress {
            compact.push(SessionEntry::Unknown);
        }
        compact
    } else {
        Vec::new()
    };
    let status = classify_status_at(&entries, permissions, now).status;
    // Use the last meaningful entry's own timestamp so the store's
    // race policy correctly orders JSONL against hook events.
    let timestamp = last_meaningful_timestamp(&entries).unwrap_or(now);
    Some(Observation {
        entries,
        event: JsonlEvent {
            session_id: stem,
            cwd: cwd.to_path_buf(),
            jsonl_path: path.to_path_buf(),
            status,
            timestamp,
        },
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
    now: DateTime<Utc>,
) -> Vec<Observation> {
    let mut out = Vec::new();
    for (cwd, dir) in lane_dirs {
        let read_dir = match std::fs::read_dir(dir) {
            Ok(rd) => rd,
            Err(_) => continue,
        };
        for entry in read_dir.flatten() {
            let path = entry.path();
            if let Some(ev) = process_jsonl_file(&path, cwd, permissions, ReadStrategy::Full, now) {
                out.push(ev);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests;
