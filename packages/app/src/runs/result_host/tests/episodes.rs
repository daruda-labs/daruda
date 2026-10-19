use super::*;
use crate::runs::result::{EndedAs, FlowRunLocation, Pointers};

#[gpui::test]
fn confirmed_local_end_only_yields_to_a_later_durable_resume(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let key = RunKey::Flow(
        dir.path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned(),
    );
    let path = daruda_flow::run_log::log_file(dir.path());
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut lines = vec![
        serde_json::json!({"v":1,"seq":1,"at_ms":0,"kind":"opened","epoch":1,"build":"test","schema":2}),
        serde_json::json!({"v":1,"seq":2,"at_ms":1,"kind":"event","event":{"type":"run_started","nodes":[]}}),
        serde_json::json!({"v":1,"seq":3,"at_ms":2,"kind":"event","event":{"type":"resumed","carried":0}}),
        serde_json::json!({"v":1,"seq":4,"at_ms":3,"kind":"event","event":{"type":"run_ended","end":{"kind":"stalled","nodes":[]}}}),
    ];
    let write = |lines: &[serde_json::Value]| {
        std::fs::write(
            &path,
            lines
                .iter()
                .map(|line| format!("{line}\n"))
                .collect::<String>(),
        )
        .unwrap()
    };
    write(&lines);
    let local = RunSnapshot {
        key: key.clone(),
        what: "local".into(),
        why: None,
        state: RunState::Ended {
            ended_as: EndedAs::Error,
            text: Some("unrecorded error".into()),
            pointers: Pointers::default(),
        },
    };
    cx.update(|cx| {
        RunsHost::ensure_installed(cx);
        cx.update_global::<RunsHost, _>(|host, _| {
            host.watch.locate(FlowRunLocation {
                run_dir: dir.path().into(),
                lock_dir: None,
            });
            host.watch.retain(local.clone());
        });
        assert_eq!(RunsHost::run_query(key.clone(), cx).read(), local);
        cx.update_global::<RunsHost, _>(|host, _| host.watch.record_flow_sequence(key.clone(), 4));
        assert_eq!(RunsHost::run_query(key.clone(), cx).read(), local);
    });
    lines.push(serde_json::json!({"v":1,"seq":5,"at_ms":4,"kind":"event","event":{"type":"resumed","carried":0}}));
    lines.push(serde_json::json!({"v":1,"seq":6,"at_ms":5,"kind":"event","event":{"type":"run_ended","end":{"kind":"done"}}}));
    write(&lines);
    cx.update(|cx| {
        let query = RunsHost::run_query(key.clone(), cx);
        let revision = RunsHost::run_revision(&key, cx);
        let observation = query.read_observation();
        assert!(matches!(
            observation.snapshot.state,
            RunState::Ended {
                ended_as: EndedAs::Completed,
                ..
            }
        ));
        RunsHost::seed_run_observation_if_current(observation, revision, cx);
        let (wait, _) = RunsHost::open_run_wait(Some(vec![key.clone()]), cx).unwrap();
        let answer = RunsHost::poll_run_wait(wait, false, cx).unwrap();
        assert_eq!(answer.ended.len(), 1);
        RunsHost::finish_run_wait(wait, true, cx);
        let (wait, _) = RunsHost::open_run_wait(Some(vec![key.clone()]), cx).unwrap();
        assert_eq!(
            RunsHost::poll_run_wait(wait, false, cx)
                .unwrap()
                .already_reported
                .len(),
            1
        );
        RunsHost::finish_run_wait(wait, true, cx);
    });
}

#[gpui::test]
fn removed_disk_run_directory_does_not_preserve_cached_running(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let run_dir = dir.path().join("foreign-running");
    let lock_dir = dir.path().join("locks");
    std::fs::create_dir(&run_dir).unwrap();
    let lock = daruda_flow::lock::RunLock::acquire(
        &lock_dir,
        "foreign-running",
        &daruda_core::process::is_alive,
    )
    .unwrap();
    let key = RunKey::Flow("foreign-running".into());
    cx.update(|cx| {
        RunsHost::ensure_installed(cx);
        cx.update_global::<RunsHost, _>(|host, _| {
            host.watch.locate(FlowRunLocation {
                run_dir: run_dir.clone(),
                lock_dir: Some(lock_dir),
            });
        });
        let revision = RunsHost::run_revision(&key, cx);
        let observation = RunsHost::run_query(key.clone(), cx).read_observation();
        assert!(matches!(
            observation.snapshot.state,
            RunState::Running { .. }
        ));
        RunsHost::seed_run_observation_if_current(observation, revision, cx);
    });
    std::fs::remove_dir_all(&run_dir).unwrap();
    cx.update(|cx| {
        let revision = RunsHost::run_revision(&key, cx);
        let observation = RunsHost::run_query(key.clone(), cx).read_observation();
        assert!(matches!(observation.snapshot.state, RunState::Gone));
        RunsHost::seed_run_observation_if_current(observation, revision, cx);
        let (wait, _) = RunsHost::open_run_wait(Some(vec![key]), cx).unwrap();
        let answer = RunsHost::poll_run_wait(wait, true, cx).unwrap();
        assert!(answer.pending.is_empty());
        assert_eq!(answer.not_running.len(), 1);
        assert!(matches!(answer.not_running[0].state, RunState::Gone));
        RunsHost::finish_run_wait(wait, true, cx);
    });
    lock.release().unwrap();
}
