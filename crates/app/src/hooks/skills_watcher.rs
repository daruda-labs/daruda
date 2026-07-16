//! Skills filesystem watcher — recursive notify subscriptions on
//! `<lane>/.claude/skills/` (project) and `~/.claude/skills/`
//! (personal).
//!
//! An FSEvent thread owns the `notify::Watcher`; a debounce thread
//! collapses event bursts (atomic-rename saves emit ≥3 events) into one
//! [`SkillsEvent`] per scope per [`DEBOUNCE`] window. Dropping
//! [`SkillsWatcherHandle`] drops the watcher, which disconnects the raw
//! channel and lets the debounce thread exit.
//!
//! Two macOS quirks:
//! - FSEvents reports canonicalised paths (`/var → /private/var`), so
//!   the closure compares against `canonicalize(...)` of each root.
//! - When the target skills dir doesn't exist yet, subscribing to it
//!   would fail, so we watch the closest existing ancestor and filter
//!   events via `starts_with`. Creating the first skill externally then
//!   still fires a `Reloaded` without restarting the watcher.

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

/// Caller-side handle. Dropping it drops the [`crate::dir_watch::DirWatcher`]
/// (stops the watch), which disconnects the raw channel and ends the debounce
/// thread. Re-spawned on lane changes — dropping the old handle releases the
/// old watcher, so re-anchoring never leaks.
pub struct SkillsWatcherHandle {
    pub(super) _watcher: crate::dir_watch::DirWatcher,
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
    use notify::RecursiveMode;

    let (event_tx, event_rx) = mpsc::channel::<SkillsEvent>();

    // Resolve canonical forms so the `starts_with` checks line up with what
    // FSEvents reports. When the target doesn't exist yet, we still derive a
    // canonical match path from whatever ancestor *does* exist — otherwise the
    // raw target path won't match the canonicalised event paths macOS reports
    // through its `/var → /private/var` redirect. See
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

    // Subscribe to the closest existing ancestor when the target skill
    // directory itself doesn't exist yet; the `classify` `starts_with` filter
    // ignores events outside the target subtree. Plugin and personal anchors
    // often coincide (`~/.claude`) — de-dup to avoid double FSEvents
    // bookkeeping.
    let mut anchors: Vec<PathBuf> = Vec::new();
    if let Some(a) = &project_anchor {
        anchors.push(a.clone());
    }
    if let Some(a) = &personal_anchor {
        anchors.push(a.clone());
    }
    if let Some(a) = &plugin_anchor {
        let already_covered = personal_anchor
            .as_deref()
            .is_some_and(|p| a.starts_with(p) || p.starts_with(a));
        if !already_covered {
            anchors.push(a.clone());
        }
    }

    let has_project = project_match.is_some();
    let classify = move |event: &notify::Event| {
        let mut out = Vec::new();
        for path in &event.paths {
            let scope = if let Some(proj) = &project_match
                && path.starts_with(proj)
            {
                SkillScope::Project
            } else if path.starts_with(&plugin_match) {
                // Plugin cache lives under `~/.claude`, so its prefix overlaps
                // with personal_match. Test it first so plugin events don't get
                // mis-routed to the personal scope.
                SkillScope::Plugin
            } else if path.starts_with(&personal_match) {
                SkillScope::Personal
            } else {
                continue;
            };
            out.push(scope);
        }
        out
    };
    // An FSEvents drop (sleep/wake) can't say which scope changed, so reload
    // every scope `classify` can route. This MUST stay in lockstep with
    // classify: a missing scope here silently breaks recovery for it.
    // Plugin and Personal are listed unconditionally on purpose — their dirs
    // overlap (plugin cache lives under `~/.claude`), so gating on a single
    // anchor could under-emit a scope still reachable via the other's watch.
    // Over-emitting an absent scope is a harmless no-op reload; under-emitting
    // is the dangerous case. Project is the only scope truly absent with no
    // active lane.
    let rescan = move || {
        let mut out = vec![SkillScope::Plugin, SkillScope::Personal];
        if has_project {
            out.push(SkillScope::Project);
        }
        out
    };

    let (raw_rx, watcher) =
        crate::dir_watch::spawn_dir_watcher(&anchors, RecursiveMode::Recursive, classify, rescan);

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

    (event_rx, SkillsWatcherHandle { _watcher: watcher })
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
