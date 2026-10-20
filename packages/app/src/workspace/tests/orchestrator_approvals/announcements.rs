use super::*;
use crate::control::{
    exec::{self, CommandOrigin, Dispatch, Pending},
    result::{ControlResult, LaneHandle},
    spec::GatedCommand,
};
use futures::{FutureExt as _, StreamExt as _};

#[derive(Clone, Copy)]
enum Announcement {
    Chat,
    TaskStart,
    TaskOpen,
    Lane,
}

#[gpui::test]
async fn chat_announcement_keeps_the_request_solicitation(cx: &mut TestAppContext) {
    delayed_announcement(Announcement::Chat, cx).await;
}

#[gpui::test]
async fn task_start_announcement_keeps_the_request_solicitation(cx: &mut TestAppContext) {
    delayed_announcement(Announcement::TaskStart, cx).await;
}

#[gpui::test]
async fn task_open_announcement_keeps_the_request_solicitation(cx: &mut TestAppContext) {
    delayed_announcement(Announcement::TaskOpen, cx).await;
}

#[gpui::test]
async fn lane_announcement_keeps_the_request_solicitation_and_prompt_origin(
    cx: &mut TestAppContext,
) {
    delayed_announcement(Announcement::Lane, cx).await;
}

async fn delayed_announcement(kind: Announcement, cx: &mut TestAppContext) {
    for (initially_solicited, away_when_announcing) in
        [(true, false), (false, false), (false, true)]
    {
        let (fixture, orchestrator) = desktop_host(cx);
        let _connections = cx.update(crate::workspace::Workspace::fake_agent_connections_for_test);
        let lane = fixture.workspace.read_with(cx, |ws, _| {
            LaneHandle::new(ws.uuid(), ws.control_active_lane())
        });
        let mut checkout = None;
        let command = match kind {
            Announcement::Chat => GatedCommand::ChatNew { lane, agent: None },
            Announcement::TaskStart | Announcement::TaskOpen => {
                let created = fixture.workspace.update(cx, |ws, cx| {
                    ws.control_task_create(
                        lane.project,
                        "Announcement",
                        "do the work".into(),
                        Some(lane.lane_ref()),
                        cx,
                    )
                    .unwrap()
                });
                cx.run_until_parked();
                let task = created.await.unwrap();
                if matches!(kind, Announcement::TaskOpen) {
                    let pending = cx.update(|cx| {
                        exec::begin_task_start(task.clone(), CommandOrigin::Phone, true, cx)
                            .unwrap()
                    });
                    cx.run_until_parked();
                    assert!(cx.update(|cx| pending.try_finish(cx)).unwrap().is_ok());
                    GatedCommand::TaskOpen { task }
                } else {
                    GatedCommand::TaskStart { task }
                }
            }
            Announcement::Lane => {
                for args in [
                    vec!["init", "--quiet"],
                    vec![
                        "-c",
                        "user.name=Fixture",
                        "-c",
                        "user.email=fixture@example.invalid",
                        "-c",
                        "commit.gpgsign=false",
                        "commit",
                        "--quiet",
                        "--allow-empty",
                        "-m",
                        "fixture",
                    ],
                ] {
                    let output = daruda_core::process::command("git")
                        .args(args)
                        .current_dir(fixture.root())
                        .output()
                        .unwrap();
                    assert!(output.status.success(), "{output:?}");
                }
                checkout = Some(fixture.workspace.read_with(cx, |ws, _| {
                    ws.control_lane_plan(lane.project, "announcement", None)
                        .unwrap()
                        .new_path
                }));
                GatedCommand::LaneCreate {
                    workspace: lane.workspace,
                    project: lane.project,
                    name: "announcement".into(),
                    base_ref: Some("HEAD".into()),
                    agent: None,
                    prompt: Some("child work from the orchestrator".into()),
                }
            }
        };
        let mut outbound = cx.update(crate::remote_channel::global::tests::setup);
        let previous = fixture.pane_ref(cx);
        cx.update(|cx| {
            crate::app_presence::seed_for_test(
                crate::platform::presence::AwaySignal::HERE,
                true,
                Some(Duration::ZERO),
                cx,
            );
            for connection in cx
                .global_mut::<crate::remote_channel::global::RemoteChannels>()
                .connections
                .values_mut()
            {
                connection.core.command_state_mut().select(Some(previous));
            }
        });
        orchestrator.update(cx, |view, _| {
            if initially_solicited {
                view.start_phone_turn_for_test(std::time::Instant::now());
            } else {
                view.end_phone_turn();
            }
        });
        let dispatch = cx.update(|cx| exec::run_gated(command, CommandOrigin::Orchestrator, cx));
        let Dispatch::Deferred {
            outcome,
            pending: Pending::Approval(id),
        } = dispatch
        else {
            panic!("the command waits for approval")
        };
        cx.run_until_parked();
        while outbound.next().now_or_never().flatten().is_some() {}
        orchestrator.update(cx, |view, _| {
            if initially_solicited {
                view.end_phone_turn();
            } else {
                view.start_phone_turn_for_test(std::time::Instant::now());
            }
        });
        cx.update(|cx| {
            crate::app_presence::seed_for_test(
                crate::platform::presence::AwaySignal::HERE,
                true,
                Some(Duration::from_secs(if away_when_announcing {
                    3600
                } else {
                    0
                })),
                cx,
            );
        });
        cx.update(|cx| assert!(resolve_desktop(id.0, "approve", cx)));
        cx.executor().allow_parking();
        let result = outcome.recv().await.unwrap().unwrap().unwrap();
        let chat = match result {
            ControlResult::ChatCreated { target, .. } => target,
            ControlResult::TaskStarted {
                chat: Some(chat), ..
            }
            | ControlResult::TaskOpened { chat, .. }
            | ControlResult::LaneCreated { chat, .. } => chat,
            other => panic!("unexpected result: {other:?}"),
        };
        cx.run_until_parked();
        let mut channels = Vec::new();
        while let Some(Some(pending)) = outbound.next().now_or_never() {
            channels.push(pending.id);
            match pending.outbound {
                crate::remote_channel::bridge::Outbound::Ping(ping) => {
                    assert!(initially_solicited);
                    assert_eq!(ping.pane, chat);
                }
                crate::remote_channel::bridge::Outbound::Notice(_) => {
                    assert!(!initially_solicited);
                }
                other => panic!("unexpected announcement: {other:?}"),
            }
        }
        channels.sort();
        assert_eq!(
            channels,
            if initially_solicited || away_when_announcing {
                vec!["personal", "work"]
            } else {
                vec!["personal"]
            }
        );
        cx.update(|cx| {
            for connection in cx
                .global_mut::<crate::remote_channel::global::RemoteChannels>()
                .connections
                .values_mut()
            {
                assert_eq!(
                    connection.core.command_state_mut().selected(),
                    Some(if initially_solicited { chat } else { previous })
                );
            }
        });
        if let Some(checkout) = checkout {
            fixture.workspace.read_with(cx, |ws, cx| {
                assert!(!ws.phone_turn_open(chat.pane, cx));
            });
            std::fs::remove_dir_all(checkout).unwrap();
        }
    }
}
