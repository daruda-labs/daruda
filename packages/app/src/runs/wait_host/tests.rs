use crate::runs::{
    RunsHost,
    result::{RunKey, RunState},
    watch::WaitOutcome,
};
use gpui::TestAppContext;

/// A real engine thread reaches the ledger without a workspace or pane.
#[gpui::test]
async fn real_flow_thread_and_two_tasks_each_deliver_once(cx: &mut TestAppContext) {
    use crate::runs::{
        FlowSource, FlowStart,
        seed::{self, SeedEnd, SeedRun},
    };
    use daruda_flow::{
        request::{Budget, RunRequest},
        runner::{AcpRunner, ProcessRunner, Runners},
    };
    cx.executor().allow_parking();
    let dir = tempfile::tempdir().unwrap();
    let tree = daruda_flow::lock::CanonicalTree::resolve(dir.path()).unwrap();
    let (events_tx, events) = smol::channel::unbounded();
    let (asks_tx, asks) = smol::channel::unbounded();
    let run_dir = dir.path().join("flow-result");
    let request = RunRequest {
        started_by: Some(daruda_flow::request::StartedBy::Orchestrator),
        loaded: daruda_flow::load(
            "version: 1\nnodes:\n  - id: check\n    kind: command\n    run: \"echo done\"\n",
            None,
        )
        .unwrap(),
        cwd: dir.path().into(),
        run_dir: run_dir.clone(),
        lock_dir: dir.path().join("locks"),
        flow_dir: dir.path().into(),
        agents: Default::default(),
        node_install_dir: dir.path().join("node"),
        budget: Budget::unlimited(),
        is_alive: Box::new(|_| false),
        git_status: None,
        events: Some(events_tx),
        ask: Some(asks_tx),
        until: None,
        pinned: vec![],
        resume: None,
    };
    let flow = RunKey::Flow("flow-result".into());
    let mut expected = vec![flow.clone()];
    cx.update(|cx| {
        RunsHost::ensure_installed(cx);
        for title in ["a", "b"] {
            let run = SeedRun::chat(dir.path());
            let task = seed::task(
                seed::spec(Default::default(), title),
                Some((&run, SeedEnd::Done)),
                cx,
            );
            let key = RunKey::Task(run.exec(&task));
            RunsHost::register_run(key.clone(), cx);
            expected.push(key);
        }
        RunsHost::start_flow(
            FlowStart {
                tree,
                request,
                source: FlowSource::File(dir.path().join("flow.yaml")),
                runners: Box::new(|request| Runners {
                    agent: AcpRunner::new(request.agents.clone(), request.node_install_dir.clone()),
                    command: ProcessRunner::new(vec![]),
                }),
                overage: Default::default(),
                submitted_by: daruda_flow::request::StartedBy::Orchestrator,
                solicited: false,
                events,
                asks,
            },
            cx,
        )
        .unwrap();
        RunsHost::register_run(flow, cx);
    });
    let mut received = Vec::new();
    for _ in 0..3 {
        let (id, wake) = cx.update(|cx| RunsHost::open_run_wait(None, cx).unwrap());
        let answer = loop {
            cx.update(|cx| {
                for key in RunsHost::run_scope(cx) {
                    let snapshot = RunsHost::run_query(key, cx).read();
                    RunsHost::seed_run_wait(snapshot, cx);
                }
            });
            if let Some(answer) = cx.update(|cx| RunsHost::poll_run_wait(id, false, cx)) {
                break answer;
            }
            wake.recv().await.unwrap();
        };
        assert_eq!(answer.outcome, WaitOutcome::Settled);
        for end in answer.ended {
            assert!(matches!(end.state, RunState::Ended { .. }));
            received.push(end.key);
        }
        cx.update(|cx| RunsHost::finish_run_wait(id, true, cx));
        if received.len() == 3 {
            break;
        }
    }
    assert_eq!(received.len(), 3);
    for key in expected {
        assert_eq!(received.iter().filter(|seen| **seen == key).count(), 1);
    }
    cx.update(|cx| {
        let (id, _) = RunsHost::open_run_wait(None, cx).unwrap();
        assert_eq!(
            RunsHost::poll_run_wait(id, false, cx).unwrap().outcome,
            WaitOutcome::Nothing
        );
        RunsHost::finish_run_wait(id, true, cx);
    });
}

#[gpui::test]
fn stale_terminal_text_read_cannot_overwrite_a_resumed_flows_new_result(cx: &mut TestAppContext) {
    use crate::runs::result::{EndedAs, RunSnapshot};
    use gpui::BorrowAppContext as _;
    let key = RunKey::Flow("same-handle".into());
    let result = |text: &str| RunSnapshot {
        key: key.clone(),
        what: "flow".into(),
        why: None,
        state: RunState::Ended {
            ended_as: EndedAs::Completed,
            text: Some(text.into()),
            pointers: Default::default(),
        },
    };
    cx.update(|cx| {
        RunsHost::ensure_installed(cx);
        let old = result("old read");
        cx.update_global::<RunsHost, _>(|host, _| host.watch.retain(old.clone()));
        let revision = RunsHost::run_revision(&key, cx);
        cx.update_global::<RunsHost, _>(|host, _| { host.watch.begin(&key); host.watch.retain(result("new result")); });
        RunsHost::seed_run_wait_if_current(old, revision, cx);
        assert!(matches!(RunsHost::run_query(key.clone(), cx).read().state, RunState::Ended { text: Some(text), .. } if text == "new result"));
    });
}

#[gpui::test]
fn a_cancelled_read_cannot_overwrite_a_recreated_keys_newer_observation(cx: &mut TestAppContext) {
    use crate::runs::result::{RunSnapshot, RunState};
    use gpui::BorrowAppContext as _;
    let key = RunKey::Flow("recreated".into());
    let snapshot = |state| RunSnapshot {
        key: key.clone(),
        what: "flow".into(),
        why: None,
        state,
    };
    cx.update(|cx| {
        RunsHost::ensure_installed(cx);
        let (cancelled, _) = RunsHost::open_run_wait(Some(vec![key.clone()]), cx).unwrap();
        RunsHost::seed_run_wait(snapshot(RunState::Gone), cx);
        let old = RunsHost::run_query(key.clone(), cx).read_observation();
        let revision = RunsHost::run_revision(&key, cx);
        RunsHost::finish_run_wait(cancelled, false, cx);
        cx.update_global::<RunsHost, _>(|host, _| {
            for number in 0..1300 {
                host.watch.observe_disk(
                    RunSnapshot {
                        key: RunKey::Flow(format!("pressure-{number}")),
                        what: String::new(),
                        why: None,
                        state: RunState::Gone,
                    },
                    0,
                );
            }
            assert!(host.watch.get(&key).is_none());
        });
        let (current, _) = RunsHost::open_run_wait(Some(vec![key.clone()]), cx).unwrap();
        RunsHost::seed_run_wait(
            snapshot(RunState::Running {
                progress: Default::default(),
                waiting_on: None,
            }),
            cx,
        );
        RunsHost::seed_run_observation_if_current(old, revision, cx);
        let answer = RunsHost::poll_run_wait(current, true, cx).unwrap();
        assert_eq!(answer.outcome, WaitOutcome::TimedOut);
        assert_eq!(answer.pending.len(), 1);
        assert!(answer.not_running.is_empty());
        RunsHost::finish_run_wait(current, false, cx);
    });
}

#[gpui::test]
fn shutdown_does_not_create_a_terminal_wait_event(cx: &mut TestAppContext) {
    let tree = std::path::PathBuf::from("/work/shutdown-flow");
    let run_dir = tree.join("run");
    cx.update(|cx| {
        RunsHost::ensure_installed(cx);
        RunsHost::seed_flow_run(
            tree.clone(),
            crate::runs::RunView::new(
                run_dir.clone(),
                crate::runs::FlowSource::Resumed {
                    run_dir: run_dir.clone(),
                },
                vec![],
            ),
            cx,
        );
        RunsHost::register_run(RunKey::Flow("run".into()), cx);
        RunsHost::apply_flow_event_for_test(
            &tree,
            &daruda_flow::event::FlowEvent::RunEnded {
                end: daruda_flow::event::RunEnd::ShutDown,
            },
            cx,
        );
        assert!(RunsHost::unreported_run_events(cx).is_empty());
    });
}

#[gpui::test]
async fn resumed_pins_describe_live_and_retained_results_like_disk(cx: &mut TestAppContext) {
    use crate::runs::{FlowSource, FlowStart};
    use daruda_flow::{
        request::{Budget, RunRequest},
        runner::{AcpRunner, ProcessRunner, Runners},
    };
    cx.executor().allow_parking();
    let dir = tempfile::tempdir().unwrap();
    let run_dir = dir.path().join("resumed-pins");
    let locks = dir.path().join("locks");
    let tree = daruda_flow::lock::CanonicalTree::resolve(dir.path()).unwrap();
    let lock_dir = daruda_flow::lock::lock_dir_for(&locks, &tree);
    let spec = "version: 1\nnodes:\n  - id: reused\n    kind: agent\n    agent: {id: claude, mode: acceptEdits}\n    output: reused.md\n    prompt: unused\n  - id: check\n    kind: command\n    deps: [reused]\n    run: \"echo resumed\"\n";
    std::fs::create_dir_all(&run_dir).unwrap();
    std::fs::write(run_dir.join("run.yaml"), spec).unwrap();
    std::fs::write(run_dir.join("reused.md"), "original output").unwrap();
    let log = daruda_flow::run_log::log_file(&run_dir);
    std::fs::create_dir_all(log.parent().unwrap()).unwrap();
    let records = [
        serde_json::json!({"v":1,"seq":1,"at_ms":0,"kind":"opened","epoch":1,"build":"test","schema":2}),
        serde_json::json!({"v":1,"seq":2,"at_ms":1,"kind":"event","event":{"type":"run_started","nodes":[{"id":"reused","produces":"file"},{"id":"check","produces":"nothing"}],"pinned":["reused"],"started_by":"orchestrator"}}),
    ];
    std::fs::write(
        log,
        records
            .into_iter()
            .map(|r| format!("{r}\n"))
            .collect::<String>(),
    )
    .unwrap();
    let resumed = daruda_flow::resume::prepare(&run_dir, Some(&lock_dir), &|_| false).unwrap();
    let continuation = resumed.continuation();
    let (events_tx, events) = smol::channel::unbounded();
    let (asks_tx, asks) = smol::channel::unbounded();
    let request = RunRequest {
        started_by: resumed.started_by,
        loaded: resumed.loaded,
        cwd: dir.path().into(),
        run_dir: run_dir.clone(),
        lock_dir: locks,
        flow_dir: run_dir.clone(),
        agents: std::collections::HashMap::from([(
            "claude".into(),
            daruda_acp::LaunchSpec {
                command: "unused-pinned-adapter".into(),
                strip_env: vec![],
            },
        )]),
        node_install_dir: dir.path().join("node"),
        budget: Budget::unlimited(),
        is_alive: Box::new(|_| false),
        git_status: None,
        events: Some(events_tx),
        ask: Some(asks_tx),
        until: None,
        pinned: vec![],
        resume: Some(continuation),
    };
    let key = RunKey::Flow("resumed-pins".into());
    let live = cx.update(|cx| {
        RunsHost::ensure_installed(cx);
        RunsHost::start_flow(
            FlowStart {
                tree,
                request,
                source: FlowSource::Resumed {
                    run_dir: run_dir.clone(),
                },
                runners: Box::new(|request| Runners {
                    agent: AcpRunner::new(request.agents.clone(), request.node_install_dir.clone()),
                    command: ProcessRunner::new(vec![]),
                }),
                overage: Default::default(),
                submitted_by: daruda_flow::request::StartedBy::Orchestrator,
                solicited: false,
                events,
                asks,
            },
            cx,
        )
        .unwrap();
        RunsHost::register_run(key.clone(), cx);
        RunsHost::run_query(key.clone(), cx).read()
    });
    let (wait, wake) = cx.update(|cx| RunsHost::open_run_wait(None, cx).unwrap());
    let held = loop {
        if let Some(answer) = cx.update(|cx| RunsHost::poll_run_wait(wait, false, cx)) {
            break answer.ended.into_iter().next().expect("flow ended");
        }
        wake.recv().await.unwrap();
    };
    cx.update(|cx| RunsHost::finish_run_wait(wait, true, cx));
    let disk = super::super::result_io::flow_disk(&crate::runs::result::FlowRunLocation {
        run_dir: run_dir.clone(),
        lock_dir: Some(lock_dir),
    });
    assert_eq!(live.why.as_deref(), Some("pinned: reused"));
    assert!(
        matches!(
            held.state,
            RunState::Ended {
                ended_as: crate::runs::result::EndedAs::Completed,
                ..
            }
        ),
        "the resumed run must finish using the original pinned output"
    );
    assert_eq!(held.why, live.why);
    assert_eq!(held.why, disk.why);
    assert_eq!(
        std::fs::read_to_string(run_dir.join("reused.md")).unwrap(),
        "original output"
    );
}
