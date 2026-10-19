use super::*;
use crate::runs::{
    result::{RunKey, RunState},
    seed::{self, SeedEnd, SeedRun},
};
use gpui::TestAppContext;

fn seeded(cx: &mut App, title: &str, end: SeedEnd) -> (String, SeedRun, RunKey) {
    RunsHost::ensure_installed(cx);
    let run = SeedRun::chat("/work");
    let task = seed::task(seed::spec(Default::default(), title), Some((&run, end)), cx);
    let key = RunKey::Task(run.exec(&task));
    RunsHost::register_run(key.clone(), cx);
    (task, run, key)
}
fn frame_payload(out: &super::super::socket::Outgoing) -> serde_json::Value {
    serde_json::from_str::<serde_json::Value>(&out.text).unwrap()["result"]["structuredContent"]
        .clone()
}

#[gpui::test]
async fn nothing_and_zero_poll_are_normal_and_timeout_metadata_is_clamped(cx: &mut TestAppContext) {
    let (tx, rx) = smol::channel::unbounded();
    cx.update(|cx| {
        prepare(None, 999, cx)
            .unwrap_or_else(|_| panic!("prepared"))
            .send(serde_json::json!(1), tx, cx)
    });
    let reply = rx.recv().await.unwrap();
    let value = frame_payload(&reply);
    assert_eq!(value["outcome"], "nothing");
    assert_eq!(
        value["timeout"],
        serde_json::json!({"requested_secs":999,"actual_secs":110,"clamped":true})
    );
    drop(reply);
    cx.run_until_parked();
    let (tx, rx) = smol::channel::unbounded();
    cx.update(|cx| {
        seeded(cx, "working", SeedEnd::Running);
        prepare(None, 0, cx)
            .unwrap_or_else(|_| panic!("prepared"))
            .send(serde_json::json!(2), tx, cx);
    });
    let reply = rx.recv().await.unwrap();
    assert_eq!(frame_payload(&reply)["outcome"], "timed_out");
    assert_eq!(
        frame_payload(&reply)["pending"].as_array().unwrap().len(),
        1
    );
}

#[gpui::test]
async fn receipt_failure_restores_event_and_cancellation_ignores_a_late_receipt(
    cx: &mut TestAppContext,
) {
    let (tx, rx) = smol::channel::unbounded();
    let (pending, key) = cx.update(|cx| {
        let (_, _, key) = seeded(cx, "done", SeedEnd::Done);
        let prepared = prepare(None, 0, cx).unwrap_or_else(|_| panic!("prepared"));
        let pending = prepared.pending;
        prepared.send(serde_json::json!(1), tx, cx);
        (pending, key)
    });
    let reply = rx.recv().await.unwrap();
    assert_eq!(frame_payload(&reply)["ended"].as_array().unwrap().len(), 1);
    cx.update(|cx| {
        assert!(pending.is_live(cx));
        assert!(RunsHost::run_wait_covers(&key, cx));
        assert!(RunsHost::unreported_run_events(cx).is_empty());
    });
    drop(reply);
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(!pending.is_live(cx));
        assert_eq!(RunsHost::unreported_run_events(cx).len(), 1);
        let next = prepare(None, 0, cx).unwrap_or_else(|_| panic!("prepared"));
        let answer = RunsHost::poll_run_wait(next.pending.0, false, cx).unwrap();
        assert_eq!(answer.ended.len(), 1);
        next.pending.withdraw(cx);
        RunsHost::finish_run_wait(next.pending.0, true, cx);
        assert_eq!(RunsHost::unreported_run_events(cx).len(), 1);
    });
}

#[gpui::test]
async fn aliases_resolve_once_and_valid_siblings_survive_malformed_handles(
    cx: &mut TestAppContext,
) {
    let (tx, rx) = smol::channel::unbounded();
    let (task, old_key) = cx.update(|cx| {
        let (task, _, key) = seeded(cx, "first", SeedEnd::Done);
        let prepared = prepare(
            Some(vec![
                "bad".into(),
                "task:missing".into(),
                format!("task:{task}"),
            ]),
            0,
            cx,
        )
        .unwrap_or_else(|_| panic!("prepared"));
        RunsHost::commit_blocking(
            daruda_store::tasks::TaskEvent::Reopened {
                id: task.clone(),
                run_in: Default::default(),
                at: chrono::Utc::now(),
            },
            cx,
        )
        .unwrap();
        seed::commit_now(seed::run(&task, &SeedRun::chat("/work"), SeedEnd::Done), cx);
        prepared.send(serde_json::json!(1), tx, cx);
        (task, key)
    });
    let reply = rx.recv().await.unwrap();
    let value = frame_payload(&reply);
    assert_eq!(
        value["ended"][0]["handle"],
        serde_json::to_value(
            super::super::runs::convert(cx.update(|cx| RunsHost::run_query(old_key, cx).read()))
                .handle
        )
        .unwrap()
    );
    assert_eq!(value["not_running"][0]["handle"], "task:missing");
    assert_eq!(value["errors"][0]["handle"], "bad");
    assert!(
        value["ended"][0]["handle"]
            .as_str()
            .unwrap()
            .starts_with(&format!("task:{task}/"))
    );
}

#[gpui::test]
async fn task_result_outlives_history_and_waiter_wakes_on_foreground_settle(
    cx: &mut TestAppContext,
) {
    let (tx, rx) = smol::channel::unbounded();
    let (task, first) = cx.update(|cx| {
        let (task, run, key) = seeded(cx, "first", SeedEnd::Running);
        prepare(None, 110, cx)
            .unwrap_or_else(|_| panic!("prepared"))
            .send(serde_json::json!(1), tx, cx);
        (task, (run, key))
    });
    cx.run_until_parked();
    assert!(rx.try_recv().is_err());
    cx.update(|cx| {
        RunsHost::commit_blocking(daruda_store::tasks::TaskEvent::Settled { exec: first.0.exec(&task), outcome: daruda_store::tasks::machine::SettleOutcome::Completed { end_reason: daruda_store::tasks::SessionEndReason::Stop }, result: Some("saved answer".into()), at: chrono::Utc::now() }, cx).unwrap();
        for _ in 0..6 {
            RunsHost::commit_blocking(daruda_store::tasks::TaskEvent::Reopened { id: task.clone(), run_in: Default::default(), at: chrono::Utc::now() }, cx).unwrap();
            seed::commit_now(seed::run(&task, &SeedRun::chat("/work"), SeedEnd::Done), cx);
        }
        assert!(matches!(RunsHost::run_query(first.1, cx).read().state, RunState::Ended {text:Some(text), ..} if text == "saved answer"));
    });
    let reply = rx.recv().await.unwrap();
    let value = frame_payload(&reply);
    assert_eq!(value["ended"].as_array().unwrap().len(), 1);
    assert_eq!(value["ended"][0]["state"]["text"], "saved answer");
}

#[gpui::test]
async fn timer_returns_timeout_with_a_reported_without_polling_external_processes(
    cx: &mut TestAppContext,
) {
    let (tx, rx) = smol::channel::unbounded();
    cx.update(|cx| {
        let (_, _, a) = seeded(cx, "a", SeedEnd::Done);
        let (id, _) = RunsHost::open_run_wait(Some(vec![a]), cx).unwrap();
        RunsHost::poll_run_wait(id, false, cx).unwrap();
        RunsHost::finish_run_wait(id, true, cx);
        seeded(cx, "b", SeedEnd::Running);
        let handles = RunsHost::run_scope(cx)
            .into_iter()
            .map(|key| {
                super::super::runs::convert(RunsHost::run_query(key, cx).read())
                    .handle
                    .to_string()
            })
            .collect();
        prepare(Some(handles), 10, cx)
            .unwrap_or_else(|_| panic!("prepared"))
            .send(serde_json::json!(1), tx, cx);
    });
    cx.run_until_parked();
    assert!(rx.try_recv().is_err());
    cx.executor().advance_clock(Duration::from_secs(11));
    let reply = rx.recv().await.unwrap();
    let value = frame_payload(&reply);
    assert_eq!(value["outcome"], "timed_out");
    assert_eq!(value["already_reported"].as_array().unwrap().len(), 1);
    assert_eq!(value["pending"].as_array().unwrap().len(), 1);
}

#[gpui::test]
async fn disk_only_flow_ending_wakes_wait_despite_unrelated_activity(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let dir = tempfile::tempdir().unwrap();
    let run_dir = dir.path().join("foreign");
    let locks = dir.path().join("locks");
    let initial = run_dir.clone();
    let lock_root = locks.clone();
    let lock = smol::unblock(move || {
        std::fs::create_dir_all(&initial).unwrap();
        daruda_flow::lock::RunLock::acquire(&lock_root, "foreign", &daruda_core::process::is_alive)
            .unwrap()
    })
    .await;
    let (tx, rx) = smol::channel::unbounded();
    cx.update(|cx| {
        RunsHost::ensure_installed(cx);
        RunsHost::install_effects(
            std::rc::Rc::new(DiskLocation(crate::runs::result::FlowRunLocation {
                run_dir: run_dir.clone(),
                lock_dir: Some(locks),
            })),
            cx,
        );
        prepare(Some(vec!["flow:foreign".into()]), 110, cx)
            .unwrap_or_else(|_| panic!("prepared"))
            .send(serde_json::json!(1), tx, cx);
    });
    cx.run_until_parked();
    assert!(rx.try_recv().is_err());
    smol::unblock(move || {
        let path = daruda_flow::run_log::log_file(&run_dir); std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let records = [
            serde_json::json!({"v":1,"seq":1,"at_ms":0,"kind":"opened","epoch":1,"build":"test","schema":2}),
            serde_json::json!({"v":1,"seq":2,"at_ms":1,"kind":"event","event":{"type":"run_started","nodes":[],"started_by":"desktop"}}),
            serde_json::json!({"v":1,"seq":3,"at_ms":2,"kind":"event","event":{"type":"run_ended","end":{"kind":"done"}}}),
        ];
        let text = records.into_iter().map(|record| format!("{record}\n")).collect::<String>(); std::fs::write(path, text).unwrap(); lock.release().unwrap();
    }).await;
    for index in 0..3 {
        cx.executor().advance_clock(Duration::from_secs(1));
        cx.update(|cx| {
            let (_, _, key) = seeded(cx, &format!("unrelated-{index}"), SeedEnd::Running);
            let snapshot = RunsHost::run_query(key, cx).read();
            RunsHost::seed_run_wait(snapshot, cx);
        });
        cx.run_until_parked();
    }
    let reply = rx
        .try_recv()
        .expect("background refresh must detect the end before the 110-second deadline");
    assert_eq!(frame_payload(&reply)["outcome"], "settled");
    assert_eq!(
        frame_payload(&reply)["ended"][0]["state"]["ended_as"],
        "completed"
    );
}

struct DiskLocation(crate::runs::result::FlowRunLocation);
impl crate::runs::RunsEffects for DiskLocation {
    fn locate_flow_run(
        &self,
        _: &str,
        _: &mut App,
    ) -> Option<crate::runs::result::FlowRunLocation> {
        Some(self.0.clone())
    }
    fn stop_execution(&self, _: &daruda_store::tasks::ExecutionRef, _: &mut App) {
        panic!("this query must not stop a run")
    }
}

mod episodes;
