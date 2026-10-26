use super::*;
use crate::control::{approval, exec::CommandOrigin, guards};
use gpui::TestAppContext;

fn setup(cx: &mut App) -> crate::telegram::global::TestOutbox {
    crate::settings_store::SettingsStore::init(cx);
    cx.global_mut::<crate::settings_store::SettingsStore>()
        .set_user_for_testing(daruda_config::Config {
            telegram: daruda_config::TelegramConfig {
                enabled: true,
                authorized_chat_id: Some(42),
                only_when_away: false,
            },
            ..Default::default()
        });
    for _ in 0..15 {
        guards::note_agent_created_lane(cx);
    }
    crate::telegram::global::install_for_test(true, Some(42), cx)
}

fn command() -> GatedCommand {
    GatedCommand::LaneCreate {
        workspace: Default::default(),
        project: 0,
        name: "new-work".into(),
        base_ref: None,
        agent: None,
        prompt: None,
    }
}

fn approve(dispatch: Dispatch, cx: &mut App) -> smol::channel::Receiver<Option<ControlOutcome>> {
    let Dispatch::Deferred {
        outcome,
        pending: Pending::Approval(id),
    } = dispatch
    else {
        panic!("the available slot must be offered for approval");
    };
    assert!(approval::resolve(
        id,
        approval::ApprovalChoice::Approved,
        approval::Answerer::Desktop,
        cx
    ));
    outcome
}

/// Only the external creation completion is controlled; guards, approvals and
/// the dispatch future run normally while two different creations overlap.
#[gpui::test]
async fn an_approved_creation_holds_the_last_budget_slot_until_completion(cx: &mut TestAppContext) {
    let _outbound = cx.update(setup);
    let (release, resume) = smol::channel::bounded(1);
    let first = cx.update(|cx| {
        let dispatch = run_gated_with(
            command(),
            CommandOrigin::Orchestrator,
            move |_, _, _, _| {
                async move {
                    resume.recv().await.unwrap();
                    Ok(ControlResult::TaskStarting {
                        task: "first".into(),
                        title: "First".into(),
                    })
                }
                .boxed_local()
            },
            cx,
        );
        approve(dispatch, cx)
    });
    cx.run_until_parked();
    let second = cx.update(|cx| {
        run_gated_with(
            command(),
            CommandOrigin::Orchestrator,
            |_, _, _, _| {
                async { panic!("a second creation cannot begin in the reserved slot") }
                    .boxed_local()
            },
            cx,
        )
    });
    assert!(
        matches!(
            second,
            Dispatch::Ready(Err(ControlError::AgentLimitReached))
        ),
        "an in-flight creation must reserve the last slot before awaiting git"
    );
    assert!(first.is_empty());
    release.send(()).await.unwrap();
    cx.run_until_parked();
    assert!(first.recv().await.unwrap().unwrap().is_ok());
    cx.update(|cx| assert_eq!(guards::spent_budget_for_test(cx), 16));
}

#[gpui::test]
async fn a_failed_approved_creation_returns_its_reserved_budget_slot(cx: &mut TestAppContext) {
    let _outbound = cx.update(setup);
    let (release, resume) = smol::channel::bounded(1);
    let first = cx.update(|cx| {
        let dispatch = run_gated_with(
            command(),
            CommandOrigin::Orchestrator,
            move |_, _, _, _| {
                async move {
                    resume.recv().await.unwrap();
                    Err(ControlError::TargetGone)
                }
                .boxed_local()
            },
            cx,
        );
        approve(dispatch, cx)
    });
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            guards::guard_agent_budget(cx),
            Err(ControlError::AgentLimitReached)
        )
    });
    release.send(()).await.unwrap();
    cx.run_until_parked();
    assert_eq!(
        first.recv().await.unwrap(),
        Some(Err(ControlError::TargetGone))
    );
    cx.update(|cx| {
        assert_eq!(guards::spent_budget_for_test(cx), 15);
        assert_eq!(guards::guard_agent_budget(cx), Ok(()));
    });
}

#[gpui::test]
async fn already_open_approvals_cannot_share_the_last_creation_slot(cx: &mut TestAppContext) {
    let _outbound = cx.update(setup);
    let (release, resume) = smol::channel::bounded(1);
    let (first, second) = cx.update(|cx| {
        let first = run_gated_with(
            command(),
            CommandOrigin::Orchestrator,
            move |_, _, _, _| {
                async move {
                    resume.recv().await.unwrap();
                    Ok(ControlResult::TaskStarting {
                        task: "first".into(),
                        title: "First".into(),
                    })
                }
                .boxed_local()
            },
            cx,
        );
        let second = run_gated_with(
            command(),
            CommandOrigin::Orchestrator,
            |_, _, _, _| {
                async {
                    Ok(ControlResult::TaskStarting {
                        task: "second".into(),
                        title: "Second".into(),
                    })
                }
                .boxed_local()
            },
            cx,
        );
        (approve(first, cx), second)
    });
    cx.run_until_parked();
    let second = cx.update(|cx| approve(second, cx));
    cx.run_until_parked();
    assert_eq!(
        second.recv().await.unwrap(),
        Some(Err(ControlError::AgentLimitReached))
    );
    assert!(first.is_empty());
    release.send(()).await.unwrap();
    cx.run_until_parked();
    assert!(first.recv().await.unwrap().unwrap().is_ok());
    cx.update(|cx| assert_eq!(guards::spent_budget_for_test(cx), 16));
}
