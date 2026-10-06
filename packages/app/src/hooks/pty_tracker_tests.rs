//! Pure resolution, diffing and exit-confirmation rules of the PTY tracker.

use super::*;

#[test]
fn binding_identity_is_pid_and_session() {
    // The diff loop's duplicate suppression compares each poll's
    // freshly-built binding against the previous one — equality
    // must hold across polls for an unchanged (pid, session).
    let base = PtyBinding {
        claude_pid: 7,
        session_id: "sess".into(),
    };
    assert_eq!(base, base.clone());
    assert_ne!(
        base,
        PtyBinding {
            claude_pid: 8,
            ..base.clone()
        }
    );
    assert_ne!(
        base,
        PtyBinding {
            session_id: "other".into(),
            ..base.clone()
        }
    );
}

fn meta(pid: u32, session: &str) -> pty_link::PidSessionMeta {
    pty_link::PidSessionMeta {
        pid,
        session_id: session.into(),
        cwd: std::path::PathBuf::from("/tmp"),
    }
}

#[test]
fn resolves_binding_when_claude_is_direct_child_of_pane_shell() {
    // pane 1's PTY shell is pid 100; the `claude` process (pid 200)
    // is its direct child. The session file is keyed by 200.
    let panes = HashMap::from([(1u64, 100u32)]);
    let sessions = vec![meta(200, "sess-a")];
    // 200 → 100 (shell) → 1 (login shell / launchd).
    let parent_of = |pid: u32| match pid {
        200 => Some(100),
        100 => Some(1),
        _ => None,
    };

    let bindings = resolve_pane_bindings(&panes, &sessions, &parent_of);

    assert_eq!(
        bindings.get(&1),
        Some(&PtyBinding {
            claude_pid: 200,
            session_id: "sess-a".into(),
        })
    );
}

#[test]
fn resolves_binding_through_an_intermediate_process() {
    // claude (300) runs as `node` (250) which is a child of the
    // pane shell (100): 300 → 250 → 100. The walk must climb two
    // hops to reach the registered shell.
    let panes = HashMap::from([(7u64, 100u32)]);
    let sessions = vec![meta(300, "sess-b")];
    let parent_of = |pid: u32| match pid {
        300 => Some(250),
        250 => Some(100),
        100 => Some(1),
        _ => None,
    };

    let bindings = resolve_pane_bindings(&panes, &sessions, &parent_of);

    assert_eq!(bindings.get(&7).map(|b| b.claude_pid), Some(300));
}

#[test]
fn no_binding_when_claude_is_not_under_any_registered_shell() {
    // claude (200) belongs to a shell (999) daruda never registered.
    let panes = HashMap::from([(1u64, 100u32)]);
    let sessions = vec![meta(200, "stray")];
    let parent_of = |pid: u32| match pid {
        200 => Some(999),
        999 => Some(1),
        _ => None,
    };

    let bindings = resolve_pane_bindings(&panes, &sessions, &parent_of);

    assert!(bindings.is_empty());
}

#[test]
fn resolves_independent_bindings_for_multiple_panes() {
    let panes = HashMap::from([(1u64, 100u32), (2u64, 200u32)]);
    let sessions = vec![meta(110, "sess-1"), meta(210, "sess-2")];
    let parent_of = |pid: u32| match pid {
        110 => Some(100),
        210 => Some(200),
        100 | 200 => Some(1),
        _ => None,
    };

    let bindings = resolve_pane_bindings(&panes, &sessions, &parent_of);

    assert_eq!(bindings.get(&1).map(|b| &b.session_id[..]), Some("sess-1"));
    assert_eq!(bindings.get(&2).map(|b| &b.session_id[..]), Some("sess-2"));
}

#[test]
fn parent_cycle_does_not_hang_the_walk() {
    // A pathological 200 ↔ 201 cycle that never reaches a shell.
    let panes = HashMap::from([(1u64, 100u32)]);
    let sessions = vec![meta(200, "loop")];
    let parent_of = |pid: u32| match pid {
        200 => Some(201),
        201 => Some(200),
        _ => None,
    };

    let bindings = resolve_pane_bindings(&panes, &sessions, &parent_of);

    assert!(bindings.is_empty());
}

fn binding(pid: u32, session: &str) -> PtyBinding {
    PtyBinding {
        claude_pid: pid,
        session_id: session.into(),
    }
}

#[test]
fn rescan_covers_every_pane() {
    // pane 1 has a resolvable claude; pane 2 does not.
    let panes = HashMap::from([(1u64, 100u32), (2u64, 500u32)]);
    let sessions = vec![meta(110, "sess-1")];
    let parent_of = |pid: u32| match pid {
        110 => Some(100),
        100 | 500 => Some(1),
        _ => None,
    };

    let bindings = rescan(&panes, &sessions, &parent_of);

    assert_eq!(bindings.get(&1), Some(&Some(binding(110, "sess-1"))));
    assert_eq!(bindings.get(&2), Some(&None));
}

#[test]
fn binding_change_events_reports_gain_loss_and_change() {
    let prev = HashMap::from([
        (1u64, Some(binding(10, "a"))), // unchanged
        (2u64, Some(binding(20, "b"))), // claude exits → None
        (3u64, Some(binding(30, "c"))), // swapped for a different session
    ]);
    let new = HashMap::from([
        (1u64, Some(binding(10, "a"))),
        (2u64, None),
        (3u64, Some(binding(31, "c2"))),
        (4u64, Some(binding(40, "d"))), // brand-new pane binding
    ]);

    let mut events = binding_change_events(&prev, &new);
    events.sort_by_key(|(pane, _)| *pane);

    assert_eq!(
        events,
        vec![
            (2u64, None),
            (3u64, Some(binding(31, "c2"))),
            (4u64, Some(binding(40, "d"))),
        ]
    );
}

#[test]
fn binding_change_events_reports_none_for_unregistered_pane() {
    // Pane 2 was bound last pass but is gone from `new` (the caller
    // unregistered it) — must still emit a clearing `None`.
    let prev = HashMap::from([
        (1u64, Some(binding(10, "a"))),
        (2u64, Some(binding(20, "b"))),
    ]);
    let new = HashMap::from([(1u64, Some(binding(10, "a")))]);

    let events = binding_change_events(&prev, &new);

    assert_eq!(events, vec![(2u64, None)]);
}

fn discovering(shell_pid: u32, until: Option<Instant>) -> PaneEntry {
    PaneEntry {
        shell_pid,
        task: Some(TaskLaunch {
            sessions_dir: PathBuf::from("/sessions"),
            discover_until: until,
        }),
    }
}

fn later() -> Option<Instant> {
    Some(Instant::now() + TASK_DISCOVERY_WINDOW)
}

fn dir(_: u32) -> PathBuf {
    PathBuf::from("/sessions")
}

#[test]
fn a_bound_process_outlives_its_pane_and_ends_the_discovery_poll() {
    let mut inner = TrackerInner::default();
    inner.panes.insert(1, discovering(100, later()));
    inner.panes.insert(2, discovering(200, later()));

    let bound = HashMap::from([(1u64, Some(binding(10, "a"))), (2u64, None)]);
    let pass = commit_pass(&mut inner, bound, &dir, &|_, _| true);
    assert_eq!(pass.binding_events.len(), 2);
    assert!(pass.exited.is_empty());
    assert!(
        inner.panes[&1]
            .task
            .as_ref()
            .unwrap()
            .discover_until
            .is_none()
    );
    assert!(
        inner.panes[&2]
            .task
            .as_ref()
            .unwrap()
            .discover_until
            .is_some()
    );
    assert!(inner.discovering());
    assert_eq!(
        inner.known_processes.get("a"),
        Some(&KnownProcess {
            pid: 10,
            sessions_dir: dir(10)
        })
    );

    // The pane goes away; the process is still what it was.
    let pass = commit_pass(&mut inner, HashMap::new(), &dir, &|_, _| true);
    assert_eq!(pass.binding_events, vec![(1, None)]);
    assert!(pass.exited.is_empty());
    assert!(inner.known_processes.contains_key("a"));

    let pass = commit_pass(&mut inner, HashMap::new(), &dir, &|_, known| {
        known.pid != 10
    });
    assert_eq!(pass.exited, vec![("a".to_string(), 10)]);
    assert!(inner.known_processes.is_empty());
}

#[test]
fn detached_process_requires_os_exit_and_emits_once() {
    let process = KnownProcess {
        pid: 42,
        sessions_dir: dir(42),
    };
    let mut known = HashMap::from([("session".to_string(), process)]);
    assert!(confirmed_exits(&mut known, &|_, _| true).is_empty());
    assert_eq!(known.len(), 1);
    assert_eq!(
        confirmed_exits(&mut known, &|_, _| false),
        vec![("session".into(), 42)]
    );
    assert!(confirmed_exits(&mut known, &|_, _| false).is_empty());
}

/// A live PID whose session file is gone or names another session no longer
/// runs the session it was observed on — a reused PID after a reboot, or a
/// CLI that moved on to a new session.
#[test]
fn a_live_pid_that_left_its_session_is_not_running_it() {
    let tmp = tempfile::tempdir().unwrap();
    let pid = std::process::id();
    let known = KnownProcess {
        pid,
        sessions_dir: tmp.path().to_path_buf(),
    };
    assert!(!still_runs("mine", &known));
    std::fs::write(
        tmp.path().join(format!("{pid}.json")),
        format!(r#"{{"pid":{pid},"sessionId":"mine","cwd":"/w"}}"#),
    )
    .unwrap();
    assert!(still_runs("mine", &known));
    assert!(!still_runs("earlier", &known));
}

/// A task launch attaches to a registered pane only, and a re-register keeps
/// it: the shell pid is what the walk ends at, the launch is what it looks for.
#[test]
fn a_task_launch_needs_its_pane_and_survives_a_re_register() {
    let (tracker, _rx) = PtyTracker::spawn();
    tracker.track_task(1, PathBuf::from("/sessions"));
    assert!(!tracker.awaits_binding(1));

    tracker.register(1, 100);
    tracker.track_task(1, PathBuf::from("/sessions"));
    assert!(tracker.awaits_binding(1));
    tracker.register(1, 101);
    let inner = tracker.inner.lock().unwrap();
    assert_eq!(inner.panes[&1].shell_pid, 101);
    assert!(inner.panes[&1].task.is_some());
}

#[test]
fn discovery_stops_once_its_window_passes() {
    let now = Instant::now();
    let mut inner = TrackerInner::default();
    inner
        .panes
        .insert(1, discovering(100, Some(now + Duration::from_secs(1))));
    inner.panes.insert(2, discovering(200, Some(now)));
    expire_discovery(&mut inner.panes, now);
    assert!(
        inner.panes[&1]
            .task
            .as_ref()
            .unwrap()
            .discover_until
            .is_some()
    );
    assert!(
        inner.panes[&2]
            .task
            .as_ref()
            .unwrap()
            .discover_until
            .is_none()
    );
    assert!(inner.discovering());
    expire_discovery(&mut inner.panes, now + Duration::from_secs(1));
    assert!(!inner.discovering());
    // The directory is still scanned after discovery stops.
    assert!(inner.panes[&2].task.is_some());
}

#[test]
fn register_and_unregister_round_trip() {
    let (tracker, _rx) = PtyTracker::spawn();
    tracker.register(1, 1234);
    tracker.register(2, 5678);
    {
        let inner = tracker.inner.lock().unwrap();
        assert_eq!(inner.panes.len(), 2);
        assert_eq!(inner.panes.get(&1).map(|p| p.shell_pid), Some(1234));
    }
    tracker.unregister(1);
    {
        let inner = tracker.inner.lock().unwrap();
        assert_eq!(inner.panes.len(), 1);
        assert!(!inner.panes.contains_key(&1));
    }
}
