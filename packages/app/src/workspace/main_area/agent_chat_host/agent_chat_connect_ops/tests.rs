use super::{retries_fresh, session_resume};

#[gpui::test]
fn account_preparation_failure_records_the_selected_account_was_unavailable(
    cx: &mut gpui::TestAppContext,
) {
    use crate::runs::seed::{self, SeedEnd, SeedRun};
    use crate::runs::{GlobalTasks, RunsHost};
    use daruda_store::tasks::Lifecycle;
    use daruda_store::tasks::machine::{ErrorCause, StartFailure};
    use gpui::AppContext as _;
    let (window, workspace) = crate::workspace::tests::build_workspace(cx);
    let (pane, id) = cx
        .update_window(window.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let pane = ws.create_agent_chat_pane(
                    Some(daruda_store::project::PaneCwd::Local(std::env::temp_dir())),
                    None,
                    ws.mirrors.agents[0].id.clone(),
                    None,
                    window,
                    cx,
                );
                let pane_id = pane.id;
                ws.active_runtime_mut().panes.push(pane);
                let run = SeedRun::chat(std::env::temp_dir());
                let id = seed::task(
                    seed::spec(Default::default(), "Account"),
                    Some((&run, SeedEnd::Dispatching)),
                    cx,
                );
                RunsHost::bind_execution(ws.run_pane(pane_id), run.exec(&id), cx);
                ws.send_agent_task_prompt_text(
                    pane_id,
                    "task prompt".into(),
                    Some(run.exec(&id)),
                    cx,
                );
                (pane_id, id)
            })
        })
        .unwrap();
    super::fail_connect_account_prepare(
        &workspace.downgrade(),
        pane,
        "test preparation error".into(),
        &mut cx.to_async(),
    );
    cx.run_until_parked();
    workspace.read_with(cx, |ws, cx| {
        assert!(
            !ws.agent_chat_view(pane)
                .unwrap()
                .read(cx)
                .queue()
                .has_pending()
        )
    });
    cx.update(|cx| {
        assert!(matches!(
            cx.global::<GlobalTasks>().get(&id).unwrap().lifecycle,
            Lifecycle::Error {
                cause: ErrorCause::StartFailed(StartFailure::AccountUnavailable),
                ..
            }
        ))
    });
}

/// An exact resume asks the adapter for that session and nothing else,
/// and its load failure is never retried as a fresh conversation.
#[test]
fn an_exact_resume_never_becomes_a_fresh_session() {
    use daruda_acp::{AcpEvent, AcpFailure, SessionResume};
    let failed = AcpEvent::Error(AcpFailure::unclassified("session not found"));

    assert!(matches!(
        session_resume(Some("cli".into()), true),
        SessionResume::Required(_)
    ));
    assert!(!retries_fresh(true, true, false, &failed));

    assert!(matches!(
        session_resume(Some("chat".into()), false),
        SessionResume::BestEffort(_)
    ));
    assert!(retries_fresh(true, false, false, &failed));
    assert!(!retries_fresh(true, false, true, &failed));
    assert!(matches!(session_resume(None, true), SessionResume::Fresh));
}
