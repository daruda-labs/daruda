use super::*;

#[gpui::test]
async fn review_regression_foreign_resume_starts_another_delivery_episode(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let run_dir = dir.path().join("foreign");
    let locks = dir.path().join("locks");
    let path = daruda_flow::run_log::log_file(&run_dir);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut lines = vec![
        serde_json::json!({"v":1,"seq":1,"at_ms":0,"kind":"opened","epoch":1,"build":"test","schema":2}),
        serde_json::json!({"v":1,"seq":2,"at_ms":1,"kind":"event","event":{"type":"run_started","nodes":[]}}),
        serde_json::json!({"v":1,"seq":3,"at_ms":2,"kind":"event","event":{"type":"run_ended","end":{"kind":"stalled","nodes":[]}}}),
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
    let key = RunKey::Flow("foreign".into());
    cx.update(|cx| {
        RunsHost::ensure_installed(cx);
        RunsHost::install_effects(
            std::rc::Rc::new(DiskLocation(crate::runs::result::FlowRunLocation {
                run_dir: run_dir.clone(),
                lock_dir: Some(locks.clone()),
            })),
            cx,
        );
    });
    for episode in 0..3 {
        if episode > 0 {
            let lock = daruda_flow::lock::RunLock::acquire(
                &locks,
                "foreign",
                &daruda_core::process::is_alive,
            )
            .unwrap();
            lines.push(serde_json::json!({"v":1,"seq":lines.len()+1,"at_ms":lines.len(),"kind":"event","event":{"type":"resumed","carried":0}}));
            write(&lines);
            if episode == 1 {
                cx.update(|cx| {
                    assert!(matches!(
                        RunsHost::run_query(key.clone(), cx).read().state,
                        RunState::Running { .. }
                    ))
                });
                let prepared = cx.update(|cx| {
                    prepare(Some(vec!["flow:foreign".into()]), 0, cx)
                        .unwrap_or_else(|_| panic!("prepared"))
                });
                let pending = prepared.pending;
                let (tx, rx) = smol::channel::unbounded();
                cx.update(|cx| prepared.send(serde_json::json!(episode), tx, cx));
                let reply = rx.recv().await.unwrap();
                assert_eq!(
                    frame_payload(&reply)["pending"].as_array().unwrap().len(),
                    1
                );
                assert!(
                    frame_payload(&reply)["already_reported"]
                        .as_array()
                        .unwrap()
                        .is_empty()
                );
                cx.update(|cx| RunsHost::finish_run_wait(pending.0, true, cx));
                drop(reply);
                cx.run_until_parked();
            }
            lines.push(serde_json::json!({"v":1,"seq":lines.len()+1,"at_ms":lines.len(),"kind":"event","event":{"type":"run_ended","end":{"kind":"stalled","nodes":[]}}}));
            write(&lines);
            lock.release().unwrap();
        }
        cx.update(|cx| {
            assert!(matches!(
                RunsHost::run_query(key.clone(), cx).read().state,
                RunState::Ended { .. }
            ))
        });
        for delivered in [false, true] {
            let prepared = cx.update(|cx| {
                prepare(Some(vec!["flow:foreign".into()]), 0, cx)
                    .unwrap_or_else(|_| panic!("prepared"))
            });
            let pending = prepared.pending;
            let (tx, rx) = smol::channel::unbounded();
            cx.update(|cx| prepared.send(serde_json::json!(episode), tx, cx));
            let reply = rx.recv().await.unwrap();
            let value = frame_payload(&reply);
            assert_eq!(
                value["ended"].as_array().unwrap().len(),
                usize::from(!delivered),
                "episode {episode}: {value}"
            );
            assert_eq!(
                value["already_reported"].as_array().unwrap().len(),
                usize::from(delivered)
            );
            cx.update(|cx| RunsHost::finish_run_wait(pending.0, true, cx));
            drop(reply);
            cx.run_until_parked();
        }
    }
    std::fs::remove_dir_all(&run_dir).unwrap();
    cx.update(|cx| {
        assert!(matches!(
            RunsHost::run_query(key, cx).read().state,
            RunState::Ended { .. }
        ))
    });
}
