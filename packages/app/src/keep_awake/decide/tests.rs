use super::*;

const MINUTE: Duration = Duration::from_secs(60);

fn caps() -> Caps {
    Caps {
        working: WORKING_CAP,
        approval_wait: 30 * MINUTE,
    }
}

fn workspace(n: u128) -> WorkspaceUuid {
    WorkspaceUuid(uuid::Uuid::from_u128(n))
}

fn seen(ws: u128, session: &str, status: SessionStatus) -> Observation {
    Observation {
        key: SessionKey {
            workspace: workspace(ws),
            session: session.to_owned(),
        },
        status,
    }
}

fn agent(observations: Vec<Observation>, clocks: &mut SessionClocks, now: Instant) -> Decision {
    decide(KeepAwakeMode::Agent, observations, clocks, caps(), now)
}

#[test]
fn off_is_inactive_and_forgets_every_session() {
    let t0 = Instant::now();
    let mut clocks = SessionClocks::default();
    agent(vec![seen(1, "a", SessionStatus::Working)], &mut clocks, t0);
    let decision = decide(
        KeepAwakeMode::Off,
        vec![seen(1, "a", SessionStatus::Working)],
        &mut clocks,
        caps(),
        t0,
    );
    assert_eq!(decision, Decision::default());
    assert!(clocks.0.is_empty());
}

#[test]
fn on_is_active_with_no_sessions() {
    let decision = decide(
        KeepAwakeMode::On,
        Vec::new(),
        &mut SessionClocks::default(),
        caps(),
        Instant::now(),
    );
    assert!(decision.active);
    assert_eq!((decision.working, decision.waiting), (0, 0));
}

#[test]
fn a_working_or_tool_running_session_keeps_it_awake() {
    for status in [SessionStatus::Working, SessionStatus::ExecutingTool] {
        let decision = agent(
            vec![seen(1, "a", status)],
            &mut SessionClocks::default(),
            Instant::now(),
        );
        assert!(decision.active, "{status:?}");
        assert_eq!(decision.working, 1);
    }
}

#[test]
fn an_approval_wait_counts_until_its_cap() {
    let t0 = Instant::now();
    let mut clocks = SessionClocks::default();
    let waiting = || vec![seen(1, "a", SessionStatus::NeedsAttention)];
    assert!(agent(waiting(), &mut clocks, t0).active);
    let at_29 = agent(waiting(), &mut clocks, t0 + 29 * MINUTE);
    assert_eq!((at_29.active, at_29.waiting), (true, 1));
    let at_31 = agent(waiting(), &mut clocks, t0 + 31 * MINUTE);
    assert_eq!((at_31.active, at_31.waiting), (false, 0));
}

#[test]
fn one_uninterrupted_working_status_counts_for_two_hours() {
    let t0 = Instant::now();
    let mut clocks = SessionClocks::default();
    let working = || vec![seen(1, "a", SessionStatus::Working)];
    agent(working(), &mut clocks, t0);
    assert!(agent(working(), &mut clocks, t0 + WORKING_CAP - MINUTE).active);
    assert!(!agent(working(), &mut clocks, t0 + WORKING_CAP + MINUTE).active);
}

#[test]
fn flipping_to_a_tool_and_back_restarts_the_clock() {
    let t0 = Instant::now();
    let mut clocks = SessionClocks::default();
    let one = |status| vec![seen(1, "a", status)];
    agent(one(SessionStatus::Working), &mut clocks, t0);
    let t1 = t0 + 90 * MINUTE;
    agent(one(SessionStatus::ExecutingTool), &mut clocks, t1);
    let t2 = t1 + 90 * MINUTE;
    agent(one(SessionStatus::Working), &mut clocks, t2);
    let decision = agent(one(SessionStatus::Working), &mut clocks, t2 + 90 * MINUTE);
    assert!(decision.active, "4.5h in total, but never 2h in one status");
}

#[test]
fn idle_connecting_and_failed_never_count() {
    let decision = agent(
        vec![
            seen(1, "a", SessionStatus::Idle),
            seen(1, "b", SessionStatus::Connecting),
            seen(1, "c", SessionStatus::Failed),
        ],
        &mut SessionClocks::default(),
        Instant::now(),
    );
    assert_eq!(decision, Decision::default());
}

#[test]
fn a_session_missing_from_a_round_is_dropped_and_restarts_on_return() {
    let t0 = Instant::now();
    let mut clocks = SessionClocks::default();
    let working = || vec![seen(1, "a", SessionStatus::Working)];
    agent(working(), &mut clocks, t0);
    assert!(!agent(Vec::new(), &mut clocks, t0 + 60 * MINUTE).active);
    assert!(clocks.0.is_empty());
    agent(working(), &mut clocks, t0 + 110 * MINUTE);
    assert!(
        agent(working(), &mut clocks, t0 + 200 * MINUTE).active,
        "the clock restarted when it came back"
    );
}

#[test]
fn the_same_chat_pane_id_in_two_windows_is_two_sessions() {
    let decision = agent(
        vec![
            seen(1, "acp:3", SessionStatus::Working),
            seen(2, "acp:3", SessionStatus::Working),
        ],
        &mut SessionClocks::default(),
        Instant::now(),
    );
    assert_eq!(decision.working, 2);
}

#[test]
fn counts_split_working_from_waiting() {
    let decision = agent(
        vec![
            seen(1, "a", SessionStatus::Working),
            seen(1, "b", SessionStatus::ExecutingTool),
            seen(1, "c", SessionStatus::NeedsAttention),
            seen(1, "d", SessionStatus::Idle),
        ],
        &mut SessionClocks::default(),
        Instant::now(),
    );
    assert_eq!(
        decision,
        Decision {
            active: true,
            working: 2,
            waiting: 1,
        }
    );
}

#[test]
fn caps_read_the_approval_wait_in_minutes() {
    let desktop = daruda_config::desktop::DesktopConfig {
        keep_awake_approval_wait_minutes: 7,
        ..Default::default()
    };
    let caps = Caps::from_config(&desktop);
    assert_eq!(caps.approval_wait, 7 * MINUTE);
    assert_eq!(caps.working, WORKING_CAP);
}
