//! Startup scope follows every terminal route, including live CLI sessions.
use super::*;
use crate::runs::{result::RunKey, watch::WaitOutcome};
use daruda_store::tasks::TaskStore;

#[gpui::test]
async fn carried_cli_runs_report_normal_error_lost_and_confirmed_exit_once(
    cx: &mut TestAppContext,
) {
    let dir = tempfile::tempdir().unwrap();
    let store = TaskStore::in_directory(dir.path(), None, "test", crate::runs::no_hook()).unwrap();
    let mut keys = Vec::new();
    for (index, process) in [
        CliProcessState::Running { pid: 42 },
        CliProcessState::Running { pid: 43 },
        CliProcessState::Unknown,
        CliProcessState::ExitConfirmed {
            ended_at: Utc::now(),
        },
    ]
    .into_iter()
    .enumerate()
    {
        let spec = seed::spec(Default::default(), "carried CLI");
        let run = SeedRun::cli(dir.path(), process).session(format!("s{index}"));
        let mut events = seed::created(spec.clone());
        events.extend(seed::run(&spec.id, &run, SeedEnd::Running));
        for mut event in events {
            if let TaskEvent::DispatchRequested { started_by, .. } = &mut event {
                *started_by = Some(daruda_store::tasks::StartedBy::Orchestrator);
            }
            smol::block_on(store.propose(event)).unwrap();
        }
        keys.push(RunKey::Task(run.exec(&spec.id)));
    }
    cx.update(|cx| {
        RunsHost::install(RunsHost::open(Ok(store)), cx);
        assert_eq!(RunsHost::run_scope(cx).len(), 4);
        RunsHost::settle_cli_session("s0", SessionEndReason::Stop, cx);
        RunsHost::settle_cli_session("s1", SessionEndReason::Error, cx);
        for key in &keys[2..] {
            let RunKey::Task(exec) = key else {
                panic!("task")
            };
            RunsHost::reconcile_lost_cli(exec.clone(), cx);
        }
    });
    cx.run_until_parked();
    cx.update(|cx| {
        let (id, _) = RunsHost::open_run_wait(None, cx).unwrap();
        let answer = RunsHost::poll_run_wait(id, false, cx).unwrap();
        assert_eq!(answer.outcome, WaitOutcome::Settled);
        assert_eq!(answer.ended.len(), 4);
        for key in keys {
            assert_eq!(answer.ended.iter().filter(|end| end.key == key).count(), 1);
        }
        RunsHost::finish_run_wait(id, true, cx);
        let (id, _) = RunsHost::open_run_wait(None, cx).unwrap();
        assert_eq!(
            RunsHost::poll_run_wait(id, false, cx).unwrap().outcome,
            WaitOutcome::Nothing
        );
        RunsHost::finish_run_wait(id, true, cx);
    });
}
