//! Skills filesystem watcher — recursive notify subscriptions on
//! `<lane>/.claude/skills/` (project) and `~/.claude/skills/`
//! (personal).
//!
//! Two threads, mirroring `panels_watcher`:
//! 1. **FSEvent thread** — owns the `notify::Watcher`, blocks on the
//!    shutdown channel, drops the watcher when the caller releases the
//!    handle.
//! 2. **Debounce thread** — collapses event bursts (atomic-rename
//!    saves emit ≥3 events) into one [`SkillsEvent`] per scope per
//!    [`DEBOUNCE`] window. Exits when its raw channel disconnects,
//!    which happens automatically once the FSEvent thread drops the
//!    watcher (and with it the closure-owned raw sender).
//!
//! Lifecycle: dropping [`SkillsWatcherHandle`] closes the shutdown
//! channel → FSEvent thread unblocks → `notify::Watcher` drops →
//! `raw_tx` drops with the closure → debounce thread sees its receiver
//! disconnect and exits.
//!
//! macOS quirk: `tempfile::tempdir()` returns paths under
//! `/var/folders/...`, which the kernel canonicalises to
//! `/private/var/folders/...`. FSEvents reports the canonicalised
//! form, so the closure compares paths against `canonicalize(...)` of
//! each watched root rather than the raw input. Without this the
//! watcher fires events with no matching scope and tests panic with
//! empty event vectors.
//!
//! Pre-existing-directory quirk: when the user has never created
//! `~/.claude/skills/` (or `<lane>/.claude/skills/`), the target
//! directory does not exist at spawn time, so `notify::watch` would
//! fail. We fall back to watching the closest existing ancestor
//! (typically `~/.claude` / `<lane>/.claude`) and rely on the
//! callback's `starts_with` filter to throw away events outside the
//! skills subtree. That way creating the first skill via an external
//! editor still fires a `Reloaded` event without daruda needing to
//! restart its watcher.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

use crate::agent::skills::SkillScope;

/// Coalescing window. Atomic-rename saves emit a burst (delete +
/// create + modify within ~ms); 100 ms is long enough to cover the
/// burst, short enough that the panel feels live.
const DEBOUNCE: Duration = Duration::from_millis(100);

/// One coalesced reload signal per scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkillsEvent {
    Reloaded(SkillScope),
}

/// Caller-side handle. Dropping this closes both watcher threads.
pub struct SkillsWatcherHandle {
    /// Holding the sender keeps the FSEvent thread alive; dropping it
    /// triggers a coordinated shutdown of both worker threads.
    pub(super) _shutdown_tx: mpsc::Sender<()>,
}

/// Spawn the watcher. `project_dir` is `None` when no lane is
/// active (welcome window) — the watcher then only subscribes to the
/// personal scope.
///
/// Returns `(events, handle)`: the pump task takes ownership of
/// `events`, while the handle stays on `Workspace` so dropping it
/// stops the worker threads.
pub fn spawn(
    project_dir: Option<PathBuf>,
    personal_dir: PathBuf,
    plugin_cache_dir: PathBuf,
) -> (mpsc::Receiver<SkillsEvent>, SkillsWatcherHandle) {
    use notify::{RecursiveMode, Watcher};

    let (raw_tx, raw_rx) = mpsc::channel::<SkillScope>();
    let (event_tx, event_rx) = mpsc::channel::<SkillsEvent>();
    let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>();

    // Resolve canonical forms so the FSEvent callback's `starts_with`
    // checks line up with what FSEvents reports. When the target
    // doesn't exist yet, we still need to derive a canonical match
    // path from whatever ancestor *does* exist — otherwise the raw
    // target path won't match the canonicalised event paths macOS
    // reports through its `/var → /private/var` redirect. See
    // `canonical_match_for_target` for the derivation.
    let project_anchor = project_dir
        .as_deref()
        .and_then(|p| nearest_existing_ancestor(p, 2));
    let personal_anchor = nearest_existing_ancestor(&personal_dir, 1);
    // Plugin cache lives under `~/.claude/plugins/cache/`. If the
    // user has never installed a marketplace plugin, this whole tree
    // is missing, so allow ascending to `~/.claude` — same depth as
    // personal.
    let plugin_anchor = nearest_existing_ancestor(&plugin_cache_dir, 2);

    let project_match = project_dir
        .as_deref()
        .map(|p| canonical_match_for_target(p, project_anchor.as_deref()));
    let personal_match = canonical_match_for_target(&personal_dir, personal_anchor.as_deref());
    let plugin_match = canonical_match_for_target(&plugin_cache_dir, plugin_anchor.as_deref());

    std::thread::spawn(move || {
        let raw_tx_inner = raw_tx.clone();
        let project_match_clone = project_match.clone();
        let personal_match_clone = personal_match.clone();
        let plugin_match_clone = plugin_match.clone();

        let mut watcher =
            match notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
                let Ok(event) = res else {
                    return;
                };
                for path in &event.paths {
                    let scope = if let Some(proj) = &project_match_clone
                        && path.starts_with(proj)
                    {
                        SkillScope::Project
                    } else if path.starts_with(&plugin_match_clone) {
                        // Plugin cache lives under `~/.claude`, so
                        // its prefix overlaps with personal_match.
                        // Test it first so plugin events don't get
                        // mis-routed to the personal scope.
                        SkillScope::Plugin
                    } else if path.starts_with(&personal_match_clone) {
                        SkillScope::Personal
                    } else {
                        continue;
                    };
                    let _ = raw_tx_inner.send(scope);
                }
            }) {
                Ok(w) => w,
                Err(_) => return,
            };

        // Subscribe to the closest existing ancestor when the target
        // skill directory itself doesn't exist yet. Same fallback for
        // every scope: parent watch + callback `starts_with` filter
        // ignores events outside the target subtree.
        if let Some(anchor) = project_anchor.as_deref() {
            let _ = watcher.watch(anchor, RecursiveMode::Recursive);
        }
        if let Some(anchor) = personal_anchor.as_deref() {
            let _ = watcher.watch(anchor, RecursiveMode::Recursive);
        }
        if let Some(anchor) = plugin_anchor.as_deref() {
            // Plugin and personal anchors often coincide
            // (`~/.claude`). `notify` is happy to attach the same
            // path twice on macOS, but we de-dup to avoid double
            // FSEvents bookkeeping when the user has both directories
            // resolved to the same parent.
            let already_covered = personal_anchor
                .as_deref()
                .is_some_and(|p| anchor.starts_with(p) || p.starts_with(anchor));
            if !already_covered {
                let _ = watcher.watch(anchor, RecursiveMode::Recursive);
            }
        }

        // Block until the caller drops the handle. `recv()` on a
        // disconnected channel returns immediately, so this sleeps
        // without a poll loop.
        let _ = shutdown_rx.recv();
        // `watcher` drops here; FSEvents subscriptions unregister and
        // the closure-owned `raw_tx_inner` drops with it. The debounce
        // thread then sees `raw_rx` disconnect and exits.
        drop(raw_tx);
    });

    // Debounce thread: drain raw events per DEBOUNCE window, emit one
    // event per scope that fired during the window. Exits when the
    // raw channel disconnects (FSEvent thread shut down).
    std::thread::spawn(move || {
        while let Ok(first_scope) = raw_rx.recv() {
            let mut pending = ScopeFlags::default();
            pending.set(first_scope);

            let deadline = std::time::Instant::now() + DEBOUNCE;
            loop {
                let now = std::time::Instant::now();
                if now >= deadline {
                    break;
                }
                match raw_rx.recv_timeout(deadline - now) {
                    Ok(scope) => pending.set(scope),
                    Err(RecvTimeoutError::Timeout) => break,
                    Err(RecvTimeoutError::Disconnected) => {
                        flush(&pending, &event_tx);
                        return;
                    }
                }
            }
            if !flush(&pending, &event_tx) {
                break;
            }
        }
    });

    (
        event_rx,
        SkillsWatcherHandle {
            _shutdown_tx: shutdown_tx,
        },
    )
}

/// Per-scope pending bits used by the debounce thread. Adding a
/// scope is idempotent; the entire bitset is flushed at the end of
/// each window. Centralising this means new scopes (e.g. Plugin) don't
/// touch the burst-collapse logic at all.
#[derive(Clone, Copy, Default)]
struct ScopeFlags {
    project: bool,
    personal: bool,
    plugin: bool,
}

impl ScopeFlags {
    fn set(&mut self, scope: SkillScope) {
        match scope {
            SkillScope::Project => self.project = true,
            SkillScope::Personal => self.personal = true,
            SkillScope::Plugin => self.plugin = true,
        }
    }
}

/// Send one `Reloaded(scope)` per pending bit. Returns `false` when
/// the receiver has dropped, so the debounce thread can exit.
fn flush(pending: &ScopeFlags, tx: &mpsc::Sender<SkillsEvent>) -> bool {
    if pending.project && tx.send(SkillsEvent::Reloaded(SkillScope::Project)).is_err() {
        return false;
    }
    if pending.personal
        && tx
            .send(SkillsEvent::Reloaded(SkillScope::Personal))
            .is_err()
    {
        return false;
    }
    if pending.plugin && tx.send(SkillsEvent::Reloaded(SkillScope::Plugin)).is_err() {
        return false;
    }
    true
}

/// Resolve `path` to its canonical form (resolving symlinks, including
/// the `/var → /private/var` redirect on macOS). Returns the original
/// path on failure so the caller can still attempt to watch it later
/// once it exists.
fn canonicalize_or_self(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Walk up at most `max_ascend` levels from `target` until we find a
/// directory that exists, and return that. `None` when even the root
/// of `target`'s tree doesn't exist.
fn nearest_existing_ancestor(target: &Path, max_ascend: usize) -> Option<PathBuf> {
    let mut current = target.to_path_buf();
    let mut hops = 0usize;
    loop {
        if current.exists() {
            return Some(current);
        }
        if hops >= max_ascend {
            return None;
        }
        match current.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => {
                current = parent.to_path_buf();
                hops += 1;
            }
            _ => return None,
        }
    }
}

/// Build the canonical path the FSEvent callback should compare
/// reported event paths against, even when `target` itself doesn't
/// exist yet.
///
/// `target` is the "logical" skills root (e.g. `~/.claude/skills`).
/// `existing_anchor` is the closest ancestor that actually exists
/// today (could be `~/.claude` if `skills/` hasn't been created).
/// We canonicalise the anchor (so symlink redirects line up with what
/// FSEvents will report) and re-attach the trailing path components
/// from `target` that lie below the anchor. The result is canonical
/// up to the anchor and raw thereafter — enough for `starts_with` to
/// match every future event under `target`.
fn canonical_match_for_target(target: &Path, existing_anchor: Option<&Path>) -> PathBuf {
    if let Some(anchor) = existing_anchor {
        let canonical_anchor = canonicalize_or_self(anchor);
        match target.strip_prefix(anchor) {
            Ok(tail) if !tail.as_os_str().is_empty() => canonical_anchor.join(tail),
            _ => canonical_anchor,
        }
    } else {
        canonicalize_or_self(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::Instant;

    /// Convenience wrapper used by every test below — give all three
    /// scope arguments at once and a default empty plugin path so a
    /// test only has to think about the scopes it actually exercises.
    fn spawn_test(
        project: Option<PathBuf>,
        personal: PathBuf,
    ) -> (
        mpsc::Receiver<SkillsEvent>,
        SkillsWatcherHandle,
        tempfile::TempDir,
    ) {
        let plugin_tmp = tempfile::tempdir().unwrap();
        let plugin_cache = plugin_tmp.path().join("missing-plugin-cache");
        let (events_rx, handle) = spawn(project, personal, plugin_cache);
        // Hold `plugin_tmp` so the empty plugin tree it lives under
        // outlives the watcher.
        (events_rx, handle, plugin_tmp)
    }

    fn drain(rx: &mpsc::Receiver<SkillsEvent>, deadline: Duration) -> Vec<SkillsEvent> {
        let mut events = Vec::new();
        let start = Instant::now();
        while start.elapsed() < deadline {
            if let Ok(ev) = rx.recv_timeout(Duration::from_millis(50)) {
                events.push(ev);
            }
        }
        events
    }

    #[test]
    #[ignore = "FSEvents-dependent — run via `cargo test --ignored`"]
    fn spawn_emits_event_when_skill_md_appears_under_personal() {
        let _g = crate::hooks::tests_common::fsevent_serial();
        let tmp = tempfile::tempdir().unwrap();
        let personal = tmp.path().to_path_buf();
        // Watcher only subscribes to existing dirs — fixture must
        // create the personal root first.
        fs::create_dir_all(&personal).unwrap();
        let (events_rx, handle, _plugin_tmp) = spawn_test(None, personal.clone());
        std::thread::sleep(Duration::from_millis(250));

        let dir = personal.join("test-skill");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("SKILL.md"), "---\nname: test\n---\n").unwrap();

        let events = drain(&events_rx, Duration::from_millis(1500));
        assert!(
            events.contains(&SkillsEvent::Reloaded(SkillScope::Personal)),
            "expected personal-scope reload, got: {events:?}"
        );

        drop(handle);
    }

    #[test]
    #[ignore = "FSEvents-dependent — run via `cargo test --ignored`"]
    fn spawn_with_project_dir_routes_each_scope_correctly() {
        let _g = crate::hooks::tests_common::fsevent_serial();
        let project_root = tempfile::tempdir().unwrap();
        let personal_root = tempfile::tempdir().unwrap();
        let project = project_root.path().to_path_buf();
        let personal = personal_root.path().to_path_buf();

        let (events_rx, handle, _plugin_tmp) = spawn_test(Some(project.clone()), personal.clone());
        std::thread::sleep(Duration::from_millis(250));

        let pdir = personal.join("p-skill");
        fs::create_dir_all(&pdir).unwrap();
        fs::write(pdir.join("SKILL.md"), "x").unwrap();

        let proj_skill = project.join("proj-skill");
        fs::create_dir_all(&proj_skill).unwrap();
        fs::write(proj_skill.join("SKILL.md"), "x").unwrap();

        let events = drain(&events_rx, Duration::from_millis(2000));
        assert!(
            events.contains(&SkillsEvent::Reloaded(SkillScope::Personal)),
            "events: {events:?}",
        );
        assert!(
            events.contains(&SkillsEvent::Reloaded(SkillScope::Project)),
            "events: {events:?}",
        );

        drop(handle);
    }

    #[test]
    #[ignore = "FSEvents-dependent — run via `cargo test --ignored`"]
    fn debounce_collapses_burst_to_one_event() {
        let _g = crate::hooks::tests_common::fsevent_serial();
        let tmp = tempfile::tempdir().unwrap();
        let personal = tmp.path().to_path_buf();
        fs::create_dir_all(&personal).unwrap();
        let (events_rx, handle, _plugin_tmp) = spawn_test(None, personal.clone());
        std::thread::sleep(Duration::from_millis(250));

        let dir = personal.join("burst");
        fs::create_dir_all(&dir).unwrap();
        for i in 0..5 {
            fs::write(dir.join(format!("file-{i}.txt")), "x").unwrap();
        }

        let events = drain(&events_rx, Duration::from_millis(1000));
        let personal_count = events
            .iter()
            .filter(|e| **e == SkillsEvent::Reloaded(SkillScope::Personal))
            .count();
        // Burst of ≥5 fs writes should collapse into a small handful
        // (≤3) of debounced events, not one-per-write.
        assert!(
            personal_count <= 3,
            "got {personal_count} events: {events:?}"
        );
        assert!(personal_count >= 1, "expected at least one event");

        drop(handle);
    }

    #[test]
    fn nearest_existing_ancestor_returns_self_when_target_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        let result = nearest_existing_ancestor(&dir, 2).unwrap();
        assert_eq!(result, dir);
    }

    #[test]
    fn nearest_existing_ancestor_walks_up_when_target_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("missing").join("skills");
        let result = nearest_existing_ancestor(&target, 2).unwrap();
        assert_eq!(result, tmp.path());
    }

    #[test]
    fn nearest_existing_ancestor_respects_ascend_limit() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("a").join("b").join("c");
        // Only allow ascending one level — `a` and `b` and the
        // tempdir all don't exist at `target`'s depth, so the first
        // ascend lands on `tmp.path()/a/b`, which doesn't exist
        // either, and we run out of hops.
        assert!(nearest_existing_ancestor(&target, 1).is_none());
    }

    #[test]
    #[ignore = "FSEvents-dependent — run via `cargo test --ignored`"]
    fn watcher_subscribes_via_parent_when_skills_dir_missing() {
        let _g = crate::hooks::tests_common::fsevent_serial();
        // Personal root doesn't have a `skills/` subdir yet, but the
        // user creates one through an external editor mid-session.
        // Without the parent-fallback the watcher would never see it.
        let tmp = tempfile::tempdir().unwrap();
        let personal_root = tmp.path().to_path_buf();
        let personal_skills = personal_root.join("skills");
        // `personal_root` exists; `personal_skills` does not yet.
        let (events_rx, handle, _plugin_tmp) = spawn_test(None, personal_skills.clone());
        std::thread::sleep(Duration::from_millis(250));

        // Now the user externally creates the skills dir + a file.
        fs::create_dir_all(&personal_skills).unwrap();
        let dir = personal_skills.join("late-arrival");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("SKILL.md"), "---\nname: late\n---\n").unwrap();

        let events = drain(&events_rx, Duration::from_millis(2000));
        assert!(
            events.contains(&SkillsEvent::Reloaded(SkillScope::Personal)),
            "expected fallback watcher to fire, got: {events:?}"
        );

        drop(handle);
    }

    #[test]
    #[ignore = "FSEvents-dependent — run via `cargo test --ignored`"]
    fn handle_drop_terminates_threads_cleanly() {
        let _g = crate::hooks::tests_common::fsevent_serial();
        // No assertion on threads (Rust gives no JoinHandle by default
        // here), but exercise the drop path so a leaked handle would
        // surface as a hung test rather than silently passing.
        let tmp = tempfile::tempdir().unwrap();
        let personal = tmp.path().to_path_buf();
        fs::create_dir_all(&personal).unwrap();
        let (events_rx, handle, _plugin_tmp) = spawn_test(None, personal);
        std::thread::sleep(Duration::from_millis(50));
        drop(handle);
        // After the handle drops, the FSEvent thread shuts the watcher
        // down, the raw channel disconnects, and the debounce thread
        // exits. The events receiver should disconnect within a few
        // hundred ms.
        let start = Instant::now();
        let mut disconnected = false;
        while start.elapsed() < Duration::from_secs(2) {
            match events_rx.recv_timeout(Duration::from_millis(50)) {
                Ok(_) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    disconnected = true;
                    break;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
        assert!(disconnected, "events channel never disconnected after drop");
    }
}
