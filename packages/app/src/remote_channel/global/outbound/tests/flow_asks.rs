use super::*;
use crate::remote_channel::dispatch::{self, Aimed, Effect, Target};
use crate::runs::{FlowSource, RunView, RunsHost};
use gpui::AppContext as _;
use std::path::{Path, PathBuf};

fn fixture(cx: &mut gpui::TestAppContext) -> (crate::test_support::ControlFixture, PathBuf) {
    let fixture = crate::test_support::workspace_for_control(cx);
    let run_dir = fixture.root().join("run");
    cx.update(|cx| {
        RunsHost::seed_flow_run_submitted_by(
            fixture.root().to_path_buf(),
            RunView::new(
                run_dir.clone(),
                FlowSource::Resumed {
                    run_dir: run_dir.clone(),
                },
                vec![],
            ),
            daruda_flow::request::StartedBy::Phone,
            cx,
        );
    });
    cx.run_until_parked();
    (fixture, run_dir)
}

fn park(
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
                tool: "Custom tool".into(),
                detail: Some("Keep every original option".into()),
                options: vec![
                    daruda_acp::PermissionChoice {
                        option_id: "once-original".into(),
                        name: "Allow once".into(),
                        kind: daruda_acp::PermissionKindView::AllowOnce,
                    },
                    daruda_acp::PermissionChoice {
                        option_id: "always-original".into(),
                        name: "Allow always".into(),
                        kind: daruda_acp::PermissionKindView::AllowAlways,
                    },
                    daruda_acp::PermissionChoice {
                        option_id: "reject-original".into(),
                        name: "Refuse this".into(),
                        kind: daruda_acp::PermissionKindView::RejectOnce,
                    },
                ],
            },
            reply,
        },
        cx,
    );
    receiver
}

fn allow() -> daruda_acp::PermissionDecision {
    daruda_acp::PermissionDecision::Allow {
        option_id: "always-original".into(),
    }
}

fn dialog(fixture: &crate::test_support::ControlFixture, cx: &mut gpui::TestAppContext) -> bool {
    cx.update_window(fixture.window.into(), |_, window, cx| {
        crate::ui::WindowExt::has_active_dialog(window, cx)
    })
    .unwrap()
}

#[gpui::test]
fn a_phone_flow_answer_closes_the_modal_and_settles_all_channels(cx: &mut gpui::TestAppContext) {
    let (fixture, run_dir) = fixture(cx);
    let (mut remote, mut telegram) = cx.update(queued);
    let receiver = cx.update(|cx| park(fixture.root(), &run_dir, 1, cx));
    cx.run_until_parked();
    assert!(dialog(&fixture, cx));
    let sending = cx.update(|cx| begin(&mut remote, cx));
    assert_eq!(sending.len(), 2);
    let (pending, prepared) = &sending[0];
    assert!(prepared.pane.is_none());
    let buttons = &prepared.message.keyboard.as_ref().unwrap().rows[0];
    assert_eq!(
        buttons
            .iter()
            .map(|(label, _)| label.as_str())
            .collect::<Vec<_>>(),
        ["Allow once", "Allow always", "Refuse this"]
    );
    assert!(
        buttons
            .iter()
            .all(|(_, token)| token.len() <= 64 && !token.contains("/"))
    );
    let action = cx.update(|cx| {
        cx.global_mut::<RemoteChannels>()
            .connections
            .get_mut(&pending.id)
            .unwrap()
            .core
            .route_callback(buttons[1].1.clone())
    });
    let losing_action = cx.update(|cx| {
        let (pending, prepared) = &sending[1];
        cx.global_mut::<RemoteChannels>()
            .connections
            .get_mut(&pending.id)
            .unwrap()
            .core
            .route_callback(
                prepared.message.keyboard.as_ref().unwrap().rows[0][2]
                    .1
                    .clone(),
            )
    });
    let telegram_sent = cx.update(|cx| telegram.begin_send(101, cx).unwrap());
    cx.update(|cx| {
        assert!(matches!(
            dispatch::handle_in_app(
                Aimed {
                    action,
                    adopted: None
                },
                &Target::Remote(pending.id.clone()),
                cx
            ),
            Effect::Feedback {
                edit: dispatch::Edit::ManagedCard,
                ..
            }
        ));
        dispatch::handle_in_app(
            Aimed {
                action: losing_action,
                adopted: None,
            },
            &Target::Remote("other".into()),
            cx,
        );
    });
    assert_eq!(receiver.try_recv(), Ok(allow()));
    cx.run_until_parked();
    assert!(!dialog(&fixture, cx));
    cx.update(|cx| {
        assert!(cx.global::<EditCapture>().edits.is_empty());
        complete(sending, cx);
        telegram_sent(cx);
        assert_eq!(cx.global::<EditCapture>().edits.len(), 3);
        assert!(
            cx.global::<EditCapture>()
                .edits
                .iter()
                .all(|edit| edit.label
                    == crate::surface::strings::notification::telegram_permission_allowed())
        );
        for connection in cx.global_mut::<RemoteChannels>().connections.values_mut() {
            assert!(matches!(
                connection.core.route_text("hello".into(), None),
                crate::remote_channel::bridge::Routed::NeedsTarget(_)
            ));
        }
    });
}

#[gpui::test]
fn a_desktop_flow_answer_settles_delivered_cards_and_keeps_sibling_questions(
    cx: &mut gpui::TestAppContext,
) {
    let (fixture, run_dir) = fixture(cx);
    let (mut remote, mut telegram) = cx.update(queued);
    let first = cx.update(|cx| park(fixture.root(), &run_dir, 1, cx));
    let second = cx.update(|cx| park(fixture.root(), &run_dir, 2, cx));
    cx.run_until_parked();
    cx.update(|cx| {
        complete(begin(&mut remote, cx), cx);
        telegram.begin_send(102, cx).unwrap()(cx);
        telegram.begin_send(103, cx).unwrap()(cx);
        RunsHost::answer_flow_ask(fixture.root(), &run_dir, 1, allow(), cx).unwrap();
    });
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(cx.global::<EditCapture>().edits.len(), 3);
        assert!(
            RunsHost::flow_run(fixture.root(), cx)
                .unwrap()
                .view()
                .question(2)
                .is_some()
        );
        assert!(second.is_empty());
    });
    assert_eq!(first.try_recv(), Ok(allow()));
}

#[gpui::test]
fn a_flow_answer_before_transport_preparation_retires_the_queued_buttons(
    cx: &mut gpui::TestAppContext,
) {
    let (fixture, run_dir) = fixture(cx);
    let (mut remote, mut telegram) = cx.update(queued);
    let _receiver = cx.update(|cx| park(fixture.root(), &run_dir, 1, cx));
    cx.run_until_parked();
    cx.update(|cx| RunsHost::answer_flow_ask(fixture.root(), &run_dir, 1, allow(), cx).unwrap());
    cx.run_until_parked();
    cx.update(|cx| {
        while let Some(Some(pending)) = remote.next().now_or_never() {
            assert!(prepare(&pending, cx).is_none());
        }
        assert!(telegram.begin_send(105, cx).is_none());
        assert!(cx.global::<EditCapture>().edits.is_empty());
    });
}

#[gpui::test]
fn a_retired_flow_send_cannot_forget_a_new_owners_callback_tokens(cx: &mut gpui::TestAppContext) {
    let (fixture, run_dir) = fixture(cx);
    let (mut remote, _telegram) = cx.update(queued);
    let _receiver = cx.update(|cx| park(fixture.root(), &run_dir, 1, cx));
    cx.run_until_parked();
    let mut sending = cx.update(|cx| begin(&mut remote, cx));
    let (old, old_prepared) = sending.pop().unwrap();
    cx.update(|cx| {
        let connection = cx
            .global_mut::<RemoteChannels>()
            .connections
            .get_mut(&old.id)
            .unwrap();
        connection.cards = Default::default();
        connection.generation = uuid::Uuid::new_v4();
        connection.bot = Some(crate::remote_channel::global::tests::ready_bot());
        let Outbound::FlowAsk(prompt) = &old.outbound else {
            panic!("flow card");
        };
        assert!(RemoteChannels::send_flow_ask(
            prompt.clone(),
            Delivery::Explicit,
            cx
        ));
        let mut replacement = begin(&mut remote, cx);
        assert_eq!(replacement.len(), 1);
        let (pending, prepared) = replacement.pop().unwrap();
        let token = prepared.message.keyboard.as_ref().unwrap().rows[0][1]
            .1
            .clone();
        finish(&old, old_prepared.pane, vec![], old_prepared.original, cx);
        assert!(matches!(
            cx.global_mut::<RemoteChannels>()
                .connections
                .get_mut(&pending.id)
                .unwrap()
                .core
                .route_callback(token),
            crate::remote_channel::bridge::InboundAction::ResolveFlowAsk { .. }
        ));
    });
}

#[gpui::test]
fn a_flow_tap_with_no_window_still_reaches_the_process_owned_run(cx: &mut gpui::TestAppContext) {
    let (fixture, run_dir) = fixture(cx);
    let (mut remote, _telegram) = cx.update(queued);
    let receiver = cx.update(|cx| park(fixture.root(), &run_dir, 1, cx));
    cx.run_until_parked();
    let sending = cx.update(|cx| begin(&mut remote, cx));
    let (pending, prepared) = &sending[0];
    let action = cx.update(|cx| {
        cx.global_mut::<RemoteChannels>()
            .connections
            .get_mut(&pending.id)
            .unwrap()
            .core
            .route_callback(
                prepared.message.keyboard.as_ref().unwrap().rows[0][1]
                    .1
                    .clone(),
            )
    });
    cx.update_window(fixture.window.into(), |_, window, _| window.remove_window())
        .unwrap();
    cx.update(|cx| {
        dispatch::handle_in_app(
            Aimed {
                action,
                adopted: None,
            },
            &Target::Remote(pending.id.clone()),
            cx,
        );
    });
    assert_eq!(receiver.try_recv(), Ok(allow()));
}

#[gpui::test]
fn old_run_phone_tokens_cannot_answer_a_replacement_with_the_same_question_number(
    cx: &mut gpui::TestAppContext,
) {
    let (fixture, run_dir) = fixture(cx);
    let (mut remote, _telegram) = cx.update(queued);
    let _old = cx.update(|cx| park(fixture.root(), &run_dir, 1, cx));
    cx.run_until_parked();
    let sending = cx.update(|cx| begin(&mut remote, cx));
    let (pending, prepared) = &sending[0];
    let action = cx.update(|cx| {
        cx.global_mut::<RemoteChannels>()
            .connections
            .get_mut(&pending.id)
            .unwrap()
            .core
            .route_callback(
                prepared.message.keyboard.as_ref().unwrap().rows[0][1]
                    .1
                    .clone(),
            )
    });
    let next_run = fixture.root().join("next");
    let receiver = cx.update(|cx| {
        RunsHost::seed_flow_run(
            fixture.root().to_path_buf(),
            RunView::new(
                next_run.clone(),
                FlowSource::Resumed {
                    run_dir: next_run.clone(),
                },
                vec![],
            ),
            cx,
        );
        let receiver = park(fixture.root(), &next_run, 1, cx);
        dispatch::handle_in_app(
            Aimed {
                action,
                adopted: None,
            },
            &Target::Remote(pending.id.clone()),
            cx,
        );
        assert_eq!(
            RunsHost::answer_flow_ask(fixture.root(), &run_dir, 1, allow(), cx),
            Err(crate::runs::AnswerRefused::NotOpen)
        );
        receiver
    });
    assert!(receiver.is_empty());
}

#[gpui::test]
fn flow_stop_end_and_both_pruning_paths_settle_cards(cx: &mut gpui::TestAppContext) {
    let (fixture, run_dir) = fixture(cx);
    let (mut remote, mut telegram) = cx.update(queued);
    for cause in ["stop", "end", "advance", "park", "closed_answer"] {
        let tree = fixture.root().join(cause);
        std::fs::create_dir_all(&tree).unwrap();
        let run = run_dir.join(cause);
        cx.update(|cx| {
            RunsHost::seed_flow_run_submitted_by(
                tree.clone(),
                RunView::new(
                    run.clone(),
                    FlowSource::Resumed {
                        run_dir: run.clone(),
                    },
                    vec![],
                ),
                daruda_flow::request::StartedBy::Phone,
                cx,
            )
        });
        let receiver = cx.update(|cx| park(&tree, &run, 1, cx));
        cx.run_until_parked();
        let sending = cx.update(|cx| begin(&mut remote, cx));
        let telegram_sent = cx.update(|cx| telegram.begin_send(104, cx).unwrap());
        if matches!(cause, "advance" | "park" | "closed_answer") {
            drop(receiver);
        }
        cx.update(|cx| match cause {
            "stop" => {
                RunsHost::stop_flow(&tree, cx);
            }
            "end" => RunsHost::apply_flow_event(
                &tree,
                &run,
                &daruda_flow::event::FlowEvent::RunEnded {
                    end: daruda_flow::event::RunEnd::Done,
                },
                None,
                cx,
            ),
            "advance" => RunsHost::apply_flow_event(
                &tree,
                &run,
                &daruda_flow::event::FlowEvent::NodeStarted {
                    node: "sibling".into(),
                    attempt: 1,
                },
                None,
                cx,
            ),
            "park" => {
                let _sibling = park(&tree, &run, 2, cx);
            }
            "closed_answer" => assert_eq!(
                RunsHost::answer_flow_ask(&tree, &run, 1, allow(), cx),
                Err(crate::runs::AnswerRefused::Closed)
            ),
            _ => unreachable!(),
        });
        cx.run_until_parked();
        cx.update(|cx| {
            complete(sending, cx);
            telegram_sent(cx);
            let edits = std::mem::take(&mut cx.global_mut::<EditCapture>().edits);
            assert_eq!(edits.len(), 3, "{cause}");
            assert!(
                edits
                    .iter()
                    .all(|edit| edit.label == crate::surface::strings::flow::ask_withdrawn()),
                "{cause}"
            );
            while telegram.try_recv().is_ok() {}
        });
    }
}
