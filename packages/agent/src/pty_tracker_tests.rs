use super::*;

#[test]
fn a_process_exits_once_and_only_after_confirmation() {
    let mut known = HashMap::from([(
        ("session".to_owned(), 42),
        KnownProcess {
            pid: 42,
            proof: ProcessProof::Observed {
                started_at: Some(1),
            },
        },
    )]);
    assert!(confirmed_exits(&mut known, &|_, _| true).is_empty());
    assert_eq!(
        confirmed_exits(&mut known, &|_, _| false),
        vec![ProcessExited {
            session_id: "session".into(),
            pid: 42
        }]
    );
    assert!(confirmed_exits(&mut known, &|_, _| false).is_empty());
}

#[test]
fn routed_processes_use_birth_identity_instead_of_session_files() {
    let pid = std::process::id();
    let started_at = process_started_at(pid).expect("the test process is visible");
    let mut process = KnownProcess {
        pid,
        proof: ProcessProof::Observed {
            started_at: Some(started_at),
        },
    };
    assert!(still_runs("no-session-file", &process));
    process.proof = ProcessProof::Observed {
        started_at: Some(started_at.saturating_add(1)),
    };
    assert!(!still_runs("no-session-file", &process));
}

#[test]
fn restored_claims_check_only_the_named_pid_file() {
    let tmp = tempfile::tempdir().unwrap();
    let pid = std::process::id();
    let process = KnownProcess {
        pid,
        proof: ProcessProof::SessionFile(tmp.path().to_path_buf()),
    };
    assert!(!still_runs("mine", &process));
    std::fs::write(
        tmp.path().join(format!("{pid}.json")),
        serde_json::json!({"pid": pid, "sessionId": "mine", "cwd": tmp.path()}).to_string(),
    )
    .unwrap();
    assert!(still_runs("mine", &process));
    assert!(!still_runs("another", &process));
}

#[test]
fn an_observed_agent_is_confirmed_after_sigkill_without_a_session_end_hook() {
    let child = test_process::sleeping();
    let pid = child.id();
    let (tracker, events) = PtyTracker::spawn();
    tracker.track_agent("killed".into(), pid);
    assert!(tracker.is_running("killed"));
    assert!(events.try_recv().is_err());
    drop(child);
    tracker.poke();
    assert_eq!(
        events.recv_timeout(Duration::from_secs(5)).unwrap(),
        ProcessExited {
            session_id: "killed".into(),
            pid
        }
    );
    assert!(!tracker.is_running("killed"));
    assert!(events.try_recv().is_err());
}

#[test]
fn the_last_handle_stops_an_idle_worker() {
    let (tracker, events) = PtyTracker::spawn();
    let other = tracker.clone();
    drop(tracker);
    assert!(matches!(events.try_recv(), Err(mpsc::TryRecvError::Empty)));
    drop(other);
    assert_eq!(
        events.recv_timeout(Duration::from_secs(5)),
        Err(mpsc::RecvTimeoutError::Disconnected)
    );
}

#[test]
fn resumed_sessions_track_each_process_until_its_own_exit() {
    let mut known = HashMap::new();
    for pid in [41, 42] {
        known.insert(
            ("shared".to_owned(), pid),
            KnownProcess {
                pid,
                proof: ProcessProof::Observed {
                    started_at: Some(1),
                },
            },
        );
    }
    assert_eq!(
        confirmed_exits(&mut known, &|_, process| process.pid == 42),
        vec![ProcessExited {
            session_id: "shared".into(),
            pid: 41
        }]
    );
    assert_eq!(known.len(), 1);
    assert_eq!(
        confirmed_exits(&mut known, &|_, _| false),
        vec![ProcessExited {
            session_id: "shared".into(),
            pid: 42
        }]
    );
}
