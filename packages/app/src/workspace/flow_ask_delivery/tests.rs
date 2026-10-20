use super::*;
use crate::remote_channel::bridge::Outbound;
use daruda_flow::request::StartedBy;
use futures::{FutureExt as _, StreamExt as _};
use std::path::Path;

#[test]
fn an_active_lane_only_suppresses_a_flow_wait_when_its_question_will_be_visible() {
    assert!(!question_in_view(false, true, false));
    assert!(question_in_view(false, true, true));
    assert!(question_in_view(true, false, false));
    assert!(!question_in_view(false, false, true));
}

fn parked(
    tree: &Path,
    run_dir: &Path,
    ask_id: u64,
    cx: &mut App,
) -> smol::channel::Receiver<daruda_acp::PermissionDecision> {
    let (reply, receiver) = smol::channel::bounded(1);
    RunsHost::park_flow_ask(
        tree,
        run_dir,
        daruda_flow::runner::PendingAsk {
            owner: daruda_flow::runner::AskOwner::Node {
                node: "writer".into(),
                generation: 0,
                attempt: 1,
            },
            ask_id,
            request: daruda_flow::runner::AskRequest {
                tool: "Tool".into(),
                detail: None,
                options: vec![],
            },
            reply,
        },
        cx,
    );
    receiver
}

fn change(
    tree: &Path,
    run_dir: &Path,
    ask_id: u64,
    submitted_by: StartedBy,
    solicited: bool,
) -> FlowChange {
    FlowChange {
        worktree: daruda_flow::lock::CanonicalTree::resolve(tree).unwrap(),
        what: FlowChanged::Asked {
            run_dir: run_dir.to_path_buf(),
            ask_id,
            alone: true,
            submitted_by,
            solicited,
        },
    }
}

#[gpui::test]
fn flow_question_delivery_uses_the_submission_stamp_and_each_connections_presence_policy(
    cx: &mut gpui::TestAppContext,
) {
    let tree = tempfile::tempdir().unwrap();
    let run_dir = tree.path().join("run");
    cx.update(|cx| {
        let mut remote = crate::remote_channel::global::tests::setup(cx);
        let mut telegram = crate::telegram::global::install_for_test(true, Some(42), cx);
        RunsHost::ensure_installed(cx);
        RunsHost::seed_flow_run(
            tree.path().to_path_buf(),
            crate::runs::RunView::new(
                run_dir.clone(),
                crate::runs::FlowSource::Resumed {
                    run_dir: run_dir.clone(),
                },
                vec![],
            ),
            cx,
        );
        for (index, (submitted_by, solicited, away, expected)) in [
            (StartedBy::Desktop, false, false, 1),
            (StartedBy::Desktop, false, true, 2),
            (StartedBy::Orchestrator, true, false, 2),
            (StartedBy::Phone, false, false, 2),
        ]
        .into_iter()
        .enumerate()
        {
            let idle = std::time::Duration::from_secs(if away { 3600 } else { 0 });
            crate::app_presence::seed_for_test(
                crate::platform::presence::AwaySignal::HERE.observe(
                    !away,
                    Some(idle),
                    std::time::Instant::now() - std::time::Duration::from_secs(3600),
                ),
                !away,
                Some(idle),
                cx,
            );
            let ask_id = index as u64 + 1;
            let _receiver = parked(tree.path(), &run_dir, ask_id, cx);
            deliver(
                &change(tree.path(), &run_dir, ask_id, submitted_by, solicited),
                cx,
            );
            let mut channels = Vec::new();
            while let Some(Some(pending)) = remote.next().now_or_never() {
                assert!(matches!(pending.outbound, Outbound::FlowAsk(_)));
                channels.push(pending.id);
            }
            assert_eq!(channels.len(), expected);
            assert!(channels.contains(&"personal".to_string()));
            assert_eq!(telegram.try_recv().is_ok(), expected == 2);
        }
    });
}

#[gpui::test]
fn an_already_closed_or_replaced_flow_question_never_sends_or_notifies(
    cx: &mut gpui::TestAppContext,
) {
    let tree = tempfile::tempdir().unwrap();
    let run_dir = tree.path().join("run");
    cx.update(|cx| {
        let mut remote = crate::remote_channel::global::tests::setup(cx);
        let mut telegram = crate::telegram::global::install_for_test(true, Some(42), cx);
        RunsHost::ensure_installed(cx);
        RunsHost::seed_flow_run(
            tree.path().to_path_buf(),
            crate::runs::RunView::new(
                run_dir.clone(),
                crate::runs::FlowSource::Resumed {
                    run_dir: run_dir.clone(),
                },
                vec![],
            ),
            cx,
        );
        let _ = crate::platform::notifications::take_recorded_for_test();
        let receiver = parked(tree.path(), &run_dir, 1, cx);
        drop(receiver);
        deliver(
            &change(tree.path(), &run_dir, 1, StartedBy::Phone, true),
            cx,
        );
        let _receiver = parked(tree.path(), &run_dir, 2, cx);
        deliver(
            &change(
                tree.path(),
                &tree.path().join("old"),
                2,
                StartedBy::Phone,
                true,
            ),
            cx,
        );
        assert!(remote.next().now_or_never().is_none());
        assert!(telegram.try_recv().is_err());
        assert!(crate::platform::notifications::take_recorded_for_test().is_empty());
    });
}

#[gpui::test]
fn native_flow_waiting_notifications_honor_the_waiting_preference(cx: &mut gpui::TestAppContext) {
    let tree = tempfile::tempdir().unwrap();
    let run_dir = tree.path().join("run");
    cx.update(|cx| {
        crate::settings_store::SettingsStore::init(cx);
        RunsHost::ensure_installed(cx);
        RunsHost::seed_flow_run(
            tree.path().to_path_buf(),
            crate::runs::RunView::new(
                run_dir.clone(),
                crate::runs::FlowSource::Resumed {
                    run_dir: run_dir.clone(),
                },
                vec![],
            ),
            cx,
        );
        let _receiver = parked(tree.path(), &run_dir, 1, cx);
        let _ = crate::platform::notifications::take_recorded_for_test();
        for enabled in [false, true] {
            let mut config = crate::settings_store::SettingsStore::global(cx)
                .user()
                .clone();
            config.notifications.agent_waiting_enabled = enabled;
            config.notifications.skip_focused_pane = false;
            cx.global_mut::<crate::settings_store::SettingsStore>()
                .set_user_for_testing(config);
            deliver(
                &change(tree.path(), &run_dir, 1, StartedBy::Desktop, false),
                cx,
            );
            assert_eq!(
                crate::platform::notifications::take_recorded_for_test().len(),
                usize::from(enabled)
            );
        }
    });
}
