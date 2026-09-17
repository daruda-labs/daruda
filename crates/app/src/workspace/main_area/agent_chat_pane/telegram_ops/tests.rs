use futures::{FutureExt as _, StreamExt as _};
use gpui::AppContext as _;

use super::{permission_buttons, permission_wait_tail, preview_for};
use crate::surface::strings as s;
use crate::telegram::bridge::PermissionDecision;
use daruda_acp::{PermissionChoice, PermissionKindView};
use daruda_store::project::PaneCwd;

/// Every relay in this file is about a chat pane, so every one of them must
/// leave the queue as a pane-attributed ping — a `Notice` here would mean an
/// agent's own message stopped registering a reply-to.
fn expect_ping(outbound: crate::telegram::bridge::Outbound) -> crate::telegram::bridge::BridgePing {
    match outbound {
        crate::telegram::bridge::Outbound::Ping(ping) => ping,
        crate::telegram::bridge::Outbound::Notice(text) => {
            panic!("an agent relay must be a ping, not a standalone notice: {text}")
        }
        crate::telegram::bridge::Outbound::Approval(prompt) => {
            panic!("an agent relay must be a ping, not an approval card: {prompt:?}")
        }
    }
}

/// A completion ping reports a past event, so the gate answers once and keeps
/// nothing: it goes out now or never.
///
/// The declined cases are the logged 2026-09-15 incident — two completions
/// settled at 15:25 while `app_active=true`, were held, and flushed together
/// at 15:28 when a 15s blur alone read as absence. Here they never enter a
/// queue, so there is nothing for a later blur to release. The last case is
/// the one a blur-required rule would lose: away from a frontmost daruda.
#[gpui::test]
async fn a_ping_that_fires_while_the_user_is_present_is_dropped_not_held(
    cx: &mut gpui::TestAppContext,
) {
    use crate::platform::presence::AwaySignal;
    use std::time::{Duration, Instant};

    let mut outbound =
        cx.update(|cx| crate::telegram::global::install_for_test(true, Some(42), cx));
    let mut config = daruda_config::Config::default();
    config.telegram.enabled = true;
    config.telegram.authorized_chat_id = Some(42);
    cx.update(crate::app_presence::init);
    let (handle, workspace) = make_window(cx, &config);
    let pane = handle
        .update(cx, |_, window, cx| {
            workspace.update(cx, |ws, cx| ws.open_agent_chat_pane_for_test(window, cx))
        })
        .unwrap();
    cx.run_until_parked();

    workspace.update(cx, |ws, cx| {
        let blurred_at = Instant::now() - Duration::from_secs(30);

        // At the daruda window: nothing leaves, and nothing is kept.
        crate::app_presence::seed_for_test(AwaySignal::HERE, true, Some(Duration::ZERO), cx);
        ws.relay_when_presence_allows(
            pane,
            "chat".into(),
            crate::telegram::bridge::TelegramTail::Markdown("while present".into()),
            None,
            cx,
        );
        assert!(outbound.next().now_or_never().is_none());

        // The incident's exact shape: blurred past the grace, but the machine
        // was still being used (16s idle against the 60s bar).
        crate::app_presence::seed_for_test(
            AwaySignal::HERE.observe(false, Some(Duration::from_secs(16)), blurred_at),
            false,
            Some(Duration::from_secs(16)),
            cx,
        );
        ws.relay_when_presence_allows(
            pane,
            "chat".into(),
            crate::telegram::bridge::TelegramTail::Markdown("while blurred but busy".into()),
            None,
            cx,
        );
        assert!(outbound.next().now_or_never().is_none());

        // Reading a long answer in daruda: silent, but under the stricter
        // foreground bar, so still present.
        crate::app_presence::seed_for_test(
            AwaySignal::HERE.observe(true, Some(Duration::from_secs(60)), blurred_at),
            true,
            Some(Duration::from_secs(60)),
            cx,
        );
        ws.relay_when_presence_allows(
            pane,
            "chat".into(),
            crate::telegram::bridge::TelegramTail::Markdown("while reading".into()),
            None,
            cx,
        );
        assert!(outbound.next().now_or_never().is_none());

        // Blurred and quiet: the lower bar clears, so the ping goes out at once.
        crate::app_presence::seed_for_test(
            AwaySignal::HERE.observe(false, Some(Duration::from_secs(300)), blurred_at),
            false,
            Some(Duration::from_secs(300)),
            cx,
        );
        ws.relay_when_presence_allows(
            pane,
            "chat".into(),
            crate::telegram::bridge::TelegramTail::Markdown("while away".into()),
            None,
            cx,
        );
        assert_eq!(
            expect_ping(outbound.next().now_or_never().flatten().unwrap())
                .pane
                .pane,
            pane
        );

        // Walked away with daruda still frontmost — no blur will ever come,
        // so the foreground bar is the only thing that can carry this case.
        crate::app_presence::seed_for_test(
            AwaySignal::HERE.observe(true, Some(Duration::from_secs(300)), blurred_at),
            true,
            Some(Duration::from_secs(300)),
            cx,
        );
        ws.relay_when_presence_allows(
            pane,
            "chat".into(),
            crate::telegram::bridge::TelegramTail::Markdown("away from a frontmost daruda".into()),
            None,
            cx,
        );
        assert_eq!(
            expect_ping(outbound.next().now_or_never().flatten().unwrap())
                .pane
                .pane,
            pane
        );

        // Returning to the window does not release what presence declined.
        crate::app_presence::seed_for_test(AwaySignal::HERE, true, Some(Duration::ZERO), cx);
        assert!(outbound.next().now_or_never().is_none());
    });
}

/// A turn the phone started is solicited, so presence does not decide its
/// relays: while the pane's `PhoneTurn` ledger is open, a completion or a
/// second permission wait reaches the phone even with the user at the desk.
/// Once the ledger closes, the same call is back under the presence rule —
/// a post-turn follow-up is unsolicited and stays gated.
#[gpui::test]
async fn a_phone_turn_relays_while_the_user_is_present(cx: &mut gpui::TestAppContext) {
    use crate::platform::presence::AwaySignal;
    use std::time::{Duration, Instant};

    let mut outbound =
        cx.update(|cx| crate::telegram::global::install_for_test(true, Some(42), cx));
    let mut config = daruda_config::Config::default();
    config.telegram.enabled = true;
    config.telegram.authorized_chat_id = Some(42);
    assert!(
        config.telegram.only_when_away,
        "the default gate must be on"
    );
    cx.update(crate::app_presence::init);
    let (handle, workspace) = make_window(cx, &config);
    let pane = handle
        .update(cx, |_, window, cx| {
            workspace.update(cx, |ws, cx| ws.open_agent_chat_pane_for_test(window, cx))
        })
        .unwrap();
    cx.run_until_parked();

    workspace.update(cx, |ws, cx| {
        crate::app_presence::seed_for_test(AwaySignal::HERE, true, Some(Duration::ZERO), cx);
        let view = ws.agent_chat_view(pane).cloned().expect("view");
        view.update(cx, |v, _| v.start_phone_turn_for_test(Instant::now()));

        assert!(
            ws.relay_when_presence_allows(
                pane,
                "h".into(),
                crate::telegram::bridge::TelegramTail::Plain("in turn".into()),
                None,
                cx,
            ),
            "an open ledger reports the ping as sent"
        );
        assert_eq!(
            expect_ping(outbound.next().now_or_never().flatten().unwrap())
                .pane
                .pane,
            pane
        );

        ws.close_phone_turn(pane, cx);
        assert!(
            !ws.relay_when_presence_allows(
                pane,
                "h".into(),
                crate::telegram::bridge::TelegramTail::Plain("after".into()),
                None,
                cx,
            ),
            "a closed ledger is back under presence"
        );
        assert!(outbound.next().now_or_never().is_none());
    });
}

/// A permission card is the phone's first sign of life, not the end of its
/// turn: folding the request answers the wait but keeps the ledger open, so
/// the completion and any later permission wait still reach a present user.
#[gpui::test]
async fn a_permission_wait_keeps_the_phone_turn_open(cx: &mut gpui::TestAppContext) {
    use crate::platform::presence::AwaySignal;
    use agent_client_protocol::schema::v1::RequestPermissionRequest;
    use daruda_acp::{PermissionOption, PermissionOptionKind, ToolCallUpdate};
    use std::time::{Duration, Instant};

    let mut outbound =
        cx.update(|cx| crate::telegram::global::install_for_test(true, Some(42), cx));
    let mut config = daruda_config::Config::default();
    config.telegram.enabled = true;
    config.telegram.authorized_chat_id = Some(42);
    cx.update(crate::app_presence::init);
    let (handle, workspace) = make_window(cx, &config);
    let pane = handle
        .update(cx, |_, window, cx| {
            workspace.update(cx, |ws, cx| ws.open_agent_chat_pane_for_test(window, cx))
        })
        .unwrap();
    cx.run_until_parked();

    workspace.update(cx, |ws, cx| {
        crate::app_presence::seed_for_test(AwaySignal::HERE, true, Some(Duration::ZERO), cx);
        let view = ws.agent_chat_view(pane).cloned().expect("view");
        view.update(cx, |v, cx| {
            v.start_phone_turn_for_test(Instant::now());
            v.apply_event(
                daruda_acp::session::AcpEvent::PermissionRequested {
                    id: 1,
                    request: Box::new(RequestPermissionRequest::new(
                        "s1",
                        ToolCallUpdate::new("t1", Default::default()),
                        vec![PermissionOption::new(
                            "allow",
                            "Allow",
                            PermissionOptionKind::AllowAlways,
                        )],
                    )),
                },
                "theme",
                false,
                cx,
            );
        });
        let v = view.read(cx);
        assert!(
            v.phone_turn().is_some() && !v.is_phone_turn_waiting(),
            "the card answered the wait without closing the turn"
        );

        assert!(
            ws.relay_when_presence_allows(
                pane,
                "h".into(),
                crate::telegram::bridge::TelegramTail::Plain("after the card".into()),
                None,
                cx,
            ),
            "a later relay in the same turn is still the phone's"
        );
        assert_eq!(
            expect_ping(outbound.next().now_or_never().flatten().unwrap())
                .pane
                .pane,
            pane
        );
    });
}

/// `only_when_away = false` is the opt-out: every ping goes to the phone,
/// presence notwithstanding. Still one decision, still no queue.
#[gpui::test]
async fn opting_out_of_the_presence_gate_sends_while_the_user_is_present(
    cx: &mut gpui::TestAppContext,
) {
    use crate::platform::presence::AwaySignal;

    let mut outbound =
        cx.update(|cx| crate::telegram::global::install_for_test(true, Some(42), cx));
    let mut config = daruda_config::Config::default();
    config.telegram.enabled = true;
    config.telegram.authorized_chat_id = Some(42);
    config.telegram.only_when_away = false;
    cx.update(crate::app_presence::init);
    let (handle, workspace) = make_window(cx, &config);
    let pane = handle
        .update(cx, |_, window, cx| {
            workspace.update(cx, |ws, cx| ws.open_agent_chat_pane_for_test(window, cx))
        })
        .unwrap();
    cx.run_until_parked();

    workspace.update(cx, |ws, cx| {
        crate::app_presence::seed_for_test(
            AwaySignal::HERE,
            true,
            Some(std::time::Duration::ZERO),
            cx,
        );
        ws.relay_when_presence_allows(
            pane,
            "chat".into(),
            crate::telegram::bridge::TelegramTail::Markdown("sent regardless".into()),
            None,
            cx,
        );
        assert_eq!(
            expect_ping(outbound.next().now_or_never().flatten().unwrap())
                .pane
                .pane,
            pane
        );
    });
}

#[test]
fn permission_wait_tail_formats_title_summary_fallback_and_empty_values() {
    assert_eq!(
        permission_wait_tail(Some("Write /tmp/x.rs"), None),
        format!("{}\n{}", s::agent_notification_waiting(), "Write /tmp/x.rs")
    );

    assert_eq!(
        permission_wait_tail(Some("Run npm install"), Some("command: npm install")),
        format!(
            "{}\n{}\n{}",
            s::agent_notification_waiting(),
            "Run npm install",
            "command: npm install"
        )
    );

    assert_eq!(
        permission_wait_tail(None, Some("file: /tmp/x.rs")),
        format!("{}\n{}", s::agent_notification_waiting(), "file: /tmp/x.rs")
    );

    assert_eq!(
        permission_wait_tail(None, None),
        s::agent_notification_waiting()
    );

    assert_eq!(
        permission_wait_tail(Some(""), Some("")),
        s::agent_notification_waiting()
    );
}

#[test]
fn first_tool_ack_tail_formats_title_empty_and_absent_cases() {
    assert_eq!(
        super::first_tool_ack_tail(Some("Write /tmp/x.rs")),
        format!(
            "{}\n{}",
            s::agent_notification_telegram_first_tool_ack(),
            "Write /tmp/x.rs"
        )
    );

    assert_eq!(
        super::first_tool_ack_tail(Some("")),
        s::agent_notification_telegram_first_tool_ack()
    );

    assert_eq!(
        super::first_tool_ack_tail(None),
        s::agent_notification_telegram_first_tool_ack()
    );
}

fn choice(option_id: &str, name: &str, kind: PermissionKindView) -> PermissionChoice {
    PermissionChoice {
        option_id: option_id.to_string(),
        name: name.to_string(),
        kind,
    }
}

#[test]
fn preview_for_keeps_short_text_and_truncates_long_text_char_safely() {
    let text = "a".repeat(2000);
    assert_eq!(preview_for(&text, "…(marker)…"), text);

    // 1000 'a's + 1000 'b's = 2001 chars, one over the threshold.
    let text = format!("{}{}", "a".repeat(1000), "b".repeat(1001));
    let result = preview_for(&text, "…(marker)…");
    assert_eq!(
        result,
        format!("{}\n…(marker)…\n{}", "a".repeat(1000), "b".repeat(1000))
    );

    // Korean text well past the threshold — must not panic on a
    // byte-boundary split, and must actually keep 1000 *characters*
    // (not bytes) on each side.
    let text = "가".repeat(2500);
    let result = preview_for(&text, "…(중략)…");
    let expected = format!("{}\n…(중략)…\n{}", "가".repeat(1000), "가".repeat(1000));
    assert_eq!(result, expected);
}

#[test]
fn permission_buttons_exposes_every_option_and_maps_rejects() {
    // The real motivating case (codex-acp): more than one Allow-shaped
    // option (Once, session-scoped, an execpolicy amendment) alongside
    // Reject — all four must become their own button, in order, not
    // collapse to a single Allow/Reject pair.
    let options = vec![
        choice("allow_once", "Allow Once", PermissionKindView::AllowOnce),
        choice(
            "allow_always",
            "Allow for Session",
            PermissionKindView::AllowAlways,
        ),
        choice(
            "accept_execpolicy_amendment",
            "Allow Commands Starting With …",
            PermissionKindView::AllowAlways,
        ),
        choice("reject_once", "Reject", PermissionKindView::RejectOnce),
    ];

    let buttons = permission_buttons(&options);

    assert_eq!(
        buttons,
        vec![
            (
                "Allow Once".to_string(),
                PermissionDecision::Allow("allow_once".to_string())
            ),
            (
                "Allow for Session".to_string(),
                PermissionDecision::Allow("allow_always".to_string())
            ),
            (
                "Allow Commands Starting With …".to_string(),
                PermissionDecision::Allow("accept_execpolicy_amendment".to_string())
            ),
            (
                "Reject".to_string(),
                PermissionDecision::Reject("reject_once".to_string())
            ),
        ]
    );

    let options = vec![choice(
        "reject_always",
        "Always Reject",
        PermissionKindView::RejectAlways,
    )];
    assert_eq!(
        permission_buttons(&options),
        vec![(
            "Always Reject".to_string(),
            PermissionDecision::Reject("reject_always".to_string())
        )]
    );

    assert!(permission_buttons(&[]).is_empty());
}

/// The regression the turn ledger fixes, end to end through the real pane.
///
/// `/usage` from the phone produces exactly one assistant message and the turn
/// ends. The first-response relay sent it; the completion relay would report
/// the same message, so the sender got the identical text twice. With a second
/// message the completion has news again and must still report.
#[gpui::test]
async fn a_turn_whose_only_answer_was_acked_is_not_reported_twice(cx: &mut gpui::TestAppContext) {
    let _outbound = cx.update(|cx| crate::telegram::global::install_for_test(true, Some(42), cx));
    let mut config = daruda_config::Config::default();
    config.telegram.enabled = true;
    config.telegram.authorized_chat_id = Some(42);
    let (handle, workspace) = make_window(cx, &config);
    cx.run_until_parked();

    let pane_id = cx
        .update_window(handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let pane = ws.create_agent_chat_pane(
                    Some(PaneCwd::Local(std::env::temp_dir())),
                    None,
                    daruda_config::AgentDefinition::claude_default().id,
                    None,
                    window,
                    cx,
                );
                let id = pane.id;
                ws.active_runtime_mut().panes.push(pane);
                id
            })
        })
        .unwrap();
    cx.run_until_parked();

    let answer = |id: &str, text: &str| daruda_acp::ChatItem::AssistantText {
        text: text.to_string(),
        streaming: false,
        message_id: Some(id.to_string()),
        phase: Default::default(),
    };

    // A phone turn that said one thing, already relayed as the first response.
    workspace.update(cx, |ws, cx| {
        let view = ws.agent_chat_view(pane_id).cloned().expect("view");
        view.update(cx, |v, _| {
            v.start_phone_turn_for_test(std::time::Instant::now());
            v.items.push(answer("m1", "## Usage\n48% used"));
            assert!(
                v.take_phone_first_response_for_test(),
                "the ack goes out for the turn's first message"
            );
        });
        assert_eq!(
            ws.telegram_completion_parts(pane_id, cx),
            None,
            "the sender already has this turn's only message"
        );
    });

    // Same pane, a turn that said something after the acked message.
    workspace.update(cx, |ws, cx| {
        let view = ws.agent_chat_view(pane_id).cloned().expect("view");
        view.update(cx, |v, _| {
            v.start_phone_turn_for_test(std::time::Instant::now());
            v.items.push(answer("m2", "working on it"));
            assert!(v.take_phone_first_response_for_test());
            v.items.push(answer("m3", "here is the answer"));
        });
        let (_, tail) = ws
            .telegram_completion_parts(pane_id, cx)
            .expect("a later message is news the sender does not have");
        assert_eq!(
            tail,
            super::TelegramTail::Markdown("here is the answer".to_string())
        );
    });
}

/// One turn, one ack. `first_response` is a pure query, so the answering
/// transition is the only thing stopping the next event from resolving the
/// same response again — which the phone receives as a duplicate ack.
#[gpui::test]
async fn a_turns_first_response_answers_it_once(cx: &mut gpui::TestAppContext) {
    let fixture = crate::test_support::workspace_with_agent_chat(cx);
    let pane_id = fixture.pane();

    fixture.workspace.update(cx, |ws, cx| {
        let view = ws.agent_chat_view(pane_id).cloned().expect("view");
        view.update(cx, |v, _| {
            v.start_phone_turn_for_test(std::time::Instant::now());
            v.items.push(daruda_acp::ChatItem::AssistantText {
                text: "on it".to_string(),
                streaming: false,
                message_id: Some("m1".to_string()),
                phase: Default::default(),
            });
            assert!(
                v.take_phone_first_response_for_test(),
                "the turn's first response is the one ack that goes out"
            );
            assert!(
                !v.take_phone_first_response_for_test(),
                "a later event on the same turn must not resolve a second ack"
            );
        });
    });
}

/// A turn that did not complete still ends its phone conversation.
///
/// The leak this guards: the ledger was retired only on `Completed`, so an
/// errored phone turn left `Answered{m1}` behind. The pane's *next* turn is
/// an in-app one, which never re-arms the ledger — and its completion was
/// then measured against `m1`, a message from a turn that was already over,
/// and suppressed outright.
#[gpui::test]
async fn a_turn_that_errored_does_not_leave_its_ledger_behind(cx: &mut gpui::TestAppContext) {
    let _outbound = cx.update(|cx| crate::telegram::global::install_for_test(true, Some(42), cx));
    let mut config = daruda_config::Config::default();
    config.telegram.enabled = true;
    config.telegram.authorized_chat_id = Some(42);
    let (handle, workspace) = make_window(cx, &config);
    cx.run_until_parked();

    let pane_id = cx
        .update_window(handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let pane = ws.create_agent_chat_pane(
                    Some(PaneCwd::Local(std::env::temp_dir())),
                    None,
                    daruda_config::AgentDefinition::claude_default().id,
                    None,
                    window,
                    cx,
                );
                let id = pane.id;
                ws.active_runtime_mut().panes.push(pane);
                id
            })
        })
        .unwrap();
    cx.run_until_parked();

    // A phone turn that said one thing, already relayed, then errored.
    workspace.update(cx, |ws, cx| {
        let view = ws.agent_chat_view(pane_id).cloned().expect("view");
        view.update(cx, |v, _| {
            v.start_phone_turn_for_test(std::time::Instant::now());
            v.items.push(daruda_acp::ChatItem::AssistantText {
                text: "partial answer".to_string(),
                streaming: false,
                message_id: Some("m1".to_string()),
                phase: Default::default(),
            });
            assert!(v.take_phone_first_response_for_test());
        });
        ws.fire_activity_completion(pane_id, super::super::view::TurnOutcome::Errored, cx);
        assert!(
            ws.agent_chat_view(pane_id)
                .expect("view")
                .read(cx)
                .phone_turn()
                .is_none(),
            "an errored turn ends its phone conversation too"
        );
    });

    // The next turn is nobody's phone turn, so its answer is news.
    workspace.update(cx, |ws, cx| {
        let (_, tail) = ws
            .telegram_completion_parts(pane_id, cx)
            .expect("a turn with no ledger owes the sender its answer");
        assert_eq!(
            tail,
            super::TelegramTail::Markdown("partial answer".to_string()),
            "with the stale ledger gone, nothing suppresses this"
        );
    });
}

/// Telegram reply acknowledgement paths on a pane with no live handle:
/// queued replies get the queued notice; overdue first-response watches send a
/// one-shot fallback ack; and a permission wait with no buttons also falls back
/// to the same ack instead of leaving the phone silent.
#[gpui::test]
async fn telegram_reply_ack_paths_cover_queue_overdue_and_empty_permission(
    cx: &mut gpui::TestAppContext,
) {
    let mut outbound =
        cx.update(|cx| crate::telegram::global::install_for_test(true, Some(42), cx));
    let mut config = daruda_config::Config::default();
    config.telegram.enabled = true;
    config.telegram.authorized_chat_id = Some(42);
    let (handle, workspace) = make_window(cx, &config);
    cx.run_until_parked();

    let tmp = std::env::temp_dir();
    let pane_id = cx
        .update_window(handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let pane = ws.create_agent_chat_pane(
                    Some(PaneCwd::Local(tmp.clone())),
                    None,
                    daruda_config::AgentDefinition::claude_default().id,
                    None,
                    window,
                    cx,
                );
                let id = pane.id;
                ws.active_runtime_mut().panes.push(pane);
                id
            })
        })
        .unwrap();
    cx.run_until_parked();

    workspace.update(cx, |ws, cx| {
        ws.inject_bot_reply(pane_id, "hello from telegram".to_string(), cx);
    });
    cx.run_until_parked();

    workspace.read_with(cx, |ws, cx| {
        let view = ws.agent_chat_view(pane_id).cloned().expect("pane present");
        assert_eq!(
            view.read(cx)
                .queue
                .pending_prompts
                .iter()
                .map(|q| q.text.as_str())
                .collect::<Vec<_>>(),
            vec!["hello from telegram"],
            "the reply queues (never connected)"
        );
        assert!(
            !view.read(cx).is_phone_turn_waiting(),
            "queuing alone must not arm the watch"
        );
    });

    let sent = expect_ping(
        outbound
            .next()
            .await
            .expect("the queued notice should be sent"),
    );
    assert_eq!(
        sent.tail,
        super::TelegramTail::Plain(s::agent_notification_telegram_reply_queued())
    );

    let started = std::time::Instant::now()
        - std::time::Duration::from_secs(super::FIRST_RESPONSE_FALLBACK_SECS + 1);
    workspace.update(cx, |ws, cx| {
        let view = ws.agent_chat_view(pane_id).cloned().expect("view present");
        view.update(cx, |v, _| v.start_phone_turn_for_test(started));
        ws.flush_telegram_first_response_fallbacks(cx);
    });
    cx.run_until_parked();

    workspace.read_with(cx, |ws, cx| {
        let view = ws.agent_chat_view(pane_id).cloned().unwrap();
        assert!(
            !view.read(cx).is_phone_turn_waiting(),
            "the overdue fallback consumes the watch"
        );
    });
    let sent = expect_ping(
        outbound
            .next()
            .await
            .expect("the fallback ack should be sent"),
    );
    assert_eq!(sent.pane.pane, pane_id);
    assert_eq!(
        sent.tail,
        super::TelegramTail::Plain(s::agent_notification_telegram_reply_ack())
    );

    workspace.update(cx, |ws, cx| {
        ws.flush_telegram_first_response_fallbacks(cx);
    });
    cx.run_until_parked();
    assert!(outbound.next().now_or_never().is_none());

    workspace.update(cx, |ws, cx| {
        let view = ws.agent_chat_view(pane_id).cloned().expect("view present");
        view.update(cx, |v, _| {
            v.start_phone_turn_for_test(std::time::Instant::now());
        });
        ws.relay_permission_wait_to_telegram(
            pane_id,
            7,
            &[],
            Some("Tool without choices"),
            None,
            cx,
        );
    });
    cx.run_until_parked();

    let sent = expect_ping(
        outbound
            .next()
            .await
            .expect("empty-button first response should still ack the phone"),
    );
    assert_eq!(sent.pane.pane, pane_id);
    assert_eq!(
        sent.tail,
        super::TelegramTail::Plain(s::agent_notification_telegram_reply_ack())
    );
}

/// Cross-workspace routing coverage: `crate::telegram::global`'s
/// `dispatch_action` matches a `PaneRef.workspace` against every open
/// `Workspace` via `WindowRegistry::for_each_workspace` + `ws.uuid() ==
/// pane.workspace`, then mutates only the matching one. That guard —
/// the entire reason `Workspace::uuid` and `for_each_workspace` exist
/// for this feature — had zero test coverage; the pure pieces
/// (`permission_buttons`, `BridgeCore`) are covered above and in
/// `telegram::bridge`, but not the multi-window dispatch itself.
///
/// This drives the same shape `InboundAction::InjectPrompt`'s dispatch
/// arm uses: two real `Workspace` windows (mirroring
/// `window_registry.rs`'s `make_window` test helper), one AgentChat pane
/// per workspace, `for_each_workspace` + a `uuid()` guard, and
/// `inject_bot_reply` as the observable mutation. Only the pane in the
/// targeted workspace should receive the injected queued prompt.
#[gpui::test]
async fn for_each_workspace_uuid_guard_dispatches_to_only_the_matching_pane(
    cx: &mut gpui::TestAppContext,
) {
    use crate::window_registry::WindowRegistry;

    let config = daruda_config::Config::default();
    let (handle_a, workspace_a) = make_window(cx, &config);
    let (handle_b, workspace_b) = make_window(cx, &config);
    cx.run_until_parked();

    let tmp = std::env::temp_dir();
    let pane_a = cx
        .update_window(handle_a.into(), |_, window, cx| {
            workspace_a.update(cx, |ws, cx| {
                let pane = ws.create_agent_chat_pane(
                    Some(PaneCwd::Local(tmp.clone())),
                    None,
                    daruda_config::AgentDefinition::claude_default().id,
                    None,
                    window,
                    cx,
                );
                let id = pane.id;
                ws.active_runtime_mut().panes.push(pane);
                id
            })
        })
        .unwrap();
    let pane_b = cx
        .update_window(handle_b.into(), |_, window, cx| {
            workspace_b.update(cx, |ws, cx| {
                let pane = ws.create_agent_chat_pane(
                    Some(PaneCwd::Local(tmp.clone())),
                    None,
                    daruda_config::AgentDefinition::claude_default().id,
                    None,
                    window,
                    cx,
                );
                let id = pane.id;
                ws.active_runtime_mut().panes.push(pane);
                id
            })
        })
        .unwrap();
    cx.run_until_parked();

    let target_uuid = workspace_a.read_with(cx, |ws, _| ws.uuid());
    assert_ne!(
        target_uuid,
        workspace_b.read_with(cx, |ws, _| ws.uuid()),
        "each window has a distinct workspace uuid"
    );

    // Exactly what `dispatch_action`'s `InjectPrompt` arm does.
    cx.update(|cx| {
        WindowRegistry::for_each_workspace(cx, |ws, _window, cx| {
            if ws.uuid() == target_uuid {
                ws.inject_bot_reply(pane_a, "hello from telegram".to_string(), cx);
            }
        });
    });
    cx.run_until_parked();

    workspace_a.read_with(cx, |ws, cx| {
        let view = ws.agent_chat_view(pane_a).cloned().expect("pane a present");
        assert_eq!(
            view.read(cx)
                .queue
                .pending_prompts
                .iter()
                .map(|q| q.text.as_str())
                .collect::<Vec<_>>(),
            vec!["hello from telegram"],
            "the targeted workspace's pane received the injected queued prompt"
        );
    });
    workspace_b.read_with(cx, |ws, cx| {
        let view = ws.agent_chat_view(pane_b).cloned().expect("pane b present");
        assert!(
            view.read(cx).queue.pending_prompts.is_empty(),
            "the non-matching workspace's pane must not be touched"
        );
    });
}

/// A permission request is a live blocking state, not a report of a past
/// event: the agent stays stopped until someone answers. So when the gate
/// declines one because the user was present, and the user then leaves, the
/// periodic sweep offers it — the request is as true then as when it fired.
/// Exactly once, and never after it has been answered.
#[gpui::test]
async fn an_outstanding_permission_declined_while_present_is_offered_once_the_user_leaves(
    cx: &mut gpui::TestAppContext,
) {
    use crate::platform::presence::AwaySignal;
    use daruda_acp::{ChatItem, PermissionItem, PermissionResolution};
    use std::time::{Duration, Instant};

    let mut outbound =
        cx.update(|cx| crate::telegram::global::install_for_test(true, Some(42), cx));
    let mut config = daruda_config::Config::default();
    config.telegram.enabled = true;
    config.telegram.authorized_chat_id = Some(42);
    cx.update(crate::app_presence::init);
    let (handle, workspace) = make_window(cx, &config);
    let pane = handle
        .update(cx, |_, window, cx| {
            workspace.update(cx, |ws, cx| ws.open_agent_chat_pane_for_test(window, cx))
        })
        .unwrap();
    cx.run_until_parked();

    let options = vec![
        choice("allow_once", "Allow", PermissionKindView::AllowOnce),
        choice("reject_once", "Reject", PermissionKindView::RejectOnce),
    ];

    workspace.update(cx, |ws, cx| {
        let view = ws.agent_chat_view(pane).cloned().expect("view present");
        view.update(cx, |v, _| {
            v.items = vec![ChatItem::Permission(PermissionItem {
                id: 7,
                tool_title: Some("Write /tmp/x.rs".to_string()),
                raw_input_summary: None,
                options: options.clone(),
                resolved: None,
            })];
            v.pending_permissions.insert(7);
        });

        // Fires while the user is at the desk: declined, nothing queued.
        crate::app_presence::seed_for_test(AwaySignal::HERE, true, Some(Duration::ZERO), cx);
        ws.relay_permission_wait_to_telegram(pane, 7, &options, Some("Write /tmp/x.rs"), None, cx);
        assert!(outbound.next().now_or_never().is_none());
        // A sweep while still present must not change that.
        ws.relay_outstanding_permissions(cx);
        assert!(outbound.next().now_or_never().is_none());

        // The user leaves. The request is still outstanding, so it goes now.
        crate::app_presence::seed_for_test(
            AwaySignal::HERE.observe(
                false,
                Some(Duration::from_secs(300)),
                Instant::now() - Duration::from_secs(30),
            ),
            false,
            Some(Duration::from_secs(300)),
            cx,
        );
        ws.relay_outstanding_permissions(cx);
        let sent = expect_ping(outbound.next().now_or_never().flatten().unwrap());
        assert_eq!(sent.pane.pane, pane);
        assert_eq!(
            sent.permission.as_ref().map(|p| p.perm_id),
            Some(7),
            "the offer carries the buttons for this request"
        );

        // Still outstanding, still away — but the phone already has it, so
        // the sweep must not repeat every tick.
        ws.relay_outstanding_permissions(cx);
        assert!(outbound.next().now_or_never().is_none());
    });

    // Answered in-app: the request leaves `pending_permissions`, and with it
    // every reason to mention it again.
    workspace.update(cx, |ws, cx| {
        let view = ws.agent_chat_view(pane).cloned().expect("view present");
        view.update(cx, |v, _| {
            v.pending_permissions.remove(&7);
            if let Some(ChatItem::Permission(p)) = v.items.first_mut() {
                p.resolved = Some(PermissionResolution::Chosen("allow_once".to_string()));
            }
        });
        ws.relay_outstanding_permissions(cx);
        assert!(outbound.next().now_or_never().is_none());
        // The bookkeeping entry went with it rather than accumulating.
        assert!(
            view.read(cx).permissions_told_to_phone.is_empty(),
            "a resolved request must not leave its id behind"
        );
    });
}

#[gpui::test]
async fn permission_delivery_history_tracks_recipient_and_connection(
    cx: &mut gpui::TestAppContext,
) {
    use crate::platform::presence::AwaySignal;
    use daruda_acp::{ChatItem, PermissionItem};
    use std::time::{Duration, Instant};

    let mut outbound =
        cx.update(|cx| crate::telegram::global::install_for_test(true, Some(42), cx));
    let mut config = daruda_config::Config::default();
    config.telegram.enabled = true;
    config.telegram.authorized_chat_id = Some(42);
    config.telegram.only_when_away = false;
    cx.update(crate::app_presence::init);
    let (handle, workspace) = make_window(cx, &config);
    let pane = handle
        .update(cx, |_, window, cx| {
            workspace.update(cx, |ws, cx| ws.open_agent_chat_pane_for_test(window, cx))
        })
        .unwrap();
    cx.run_until_parked();

    let options = vec![choice("allow_once", "Allow", PermissionKindView::AllowOnce)];
    let add_request = |view: &mut super::super::view::AgentChatView| {
        view.items.push(ChatItem::Permission(PermissionItem {
            id: 0,
            tool_title: Some("Write /tmp/x.rs".into()),
            raw_input_summary: None,
            options: options.clone(),
            resolved: None,
        }));
        view.pending_permissions.insert(0);
    };

    workspace.update(cx, |ws, cx| {
        let view = ws.agent_chat_view(pane).cloned().unwrap();
        view.update(cx, |v, _| add_request(v));
        ws.relay_outstanding_permissions(cx);
        assert_eq!(
            expect_ping(outbound.next().now_or_never().flatten().unwrap())
                .permission
                .unwrap()
                .perm_id,
            0
        );

        // Ordinary reloads keep deduplication; re-pairing allows a fresh offer,
        // including unpairing and pairing the same chat again.
        for (recipient, should_send) in [
            (Some(42), false),
            (Some(84), true),
            (None, false),
            (Some(84), true),
        ] {
            config.telegram.authorized_chat_id = recipient;
            ws.apply_config(&config, cx);
            ws.relay_outstanding_permissions(cx);
            assert_eq!(
                outbound.next().now_or_never().flatten().is_some(),
                should_send,
                "recipient={recipient:?}"
            );
            ws.relay_outstanding_permissions(cx);
            assert!(outbound.next().now_or_never().is_none());
        }

        config.telegram.only_when_away = true;
        ws.apply_config(&config, cx);
        for reconnect in [false, true] {
            view.update(cx, |v, cx| {
                if reconnect {
                    v.retry_for_reconnect(Some("saved-session".into()), cx);
                } else {
                    v.reset_for_new_session(cx);
                }
            });
            // A new connection reuses id 0 before any cleanup sweep. Its first
            // offer is declined while present and must remain eligible later.
            crate::app_presence::seed_for_test(AwaySignal::HERE, true, Some(Duration::ZERO), cx);
            ws.relay_permission_wait_to_telegram(
                pane,
                0,
                &options,
                Some("Write /tmp/x.rs"),
                None,
                cx,
            );
            view.update(cx, |v, _| add_request(v));
            ws.relay_outstanding_permissions(cx);
            assert!(outbound.next().now_or_never().is_none());

            crate::app_presence::seed_for_test(
                AwaySignal::HERE.observe(
                    false,
                    Some(Duration::from_secs(300)),
                    Instant::now() - Duration::from_secs(30),
                ),
                false,
                Some(Duration::from_secs(300)),
                cx,
            );
            ws.relay_outstanding_permissions(cx);
            let ping = expect_ping(
                outbound
                    .next()
                    .now_or_never()
                    .flatten()
                    .expect("new connection's permission must be relayed"),
            );
            assert_eq!(ping.permission.unwrap().perm_id, 0);
            ws.relay_outstanding_permissions(cx);
            assert!(outbound.next().now_or_never().is_none());
        }
    });
}

/// A phone tap routes by permission id, not by position: with two
/// permissions outstanding, tapping the button for id A resolves *A* and
/// leaves B live; a second tap for an already-answered id is a clean no-op.
/// Mirrors the in-app `concurrent_permissions_resolve_independently_by_id`
/// for the Telegram `respond_bot_permission` path.
#[gpui::test]
async fn respond_bot_permission_routes_by_id_under_concurrency(cx: &mut gpui::TestAppContext) {
    use daruda_acp::{
        ChatItem, PermissionChoice, PermissionItem, PermissionKindView, PermissionResolution,
    };

    let config = daruda_config::Config::default();
    let (handle, workspace) = make_window(cx, &config);
    cx.run_until_parked();

    let tmp = std::env::temp_dir();
    let card = |id: u64| {
        ChatItem::Permission(PermissionItem {
            id,
            tool_title: Some(format!("Write /tmp/{id}")),
            raw_input_summary: None,
            options: vec![
                PermissionChoice {
                    option_id: "allow_once".to_string(),
                    name: "Allow".to_string(),
                    kind: PermissionKindView::AllowOnce,
                },
                PermissionChoice {
                    option_id: "reject_once".to_string(),
                    name: "Reject".to_string(),
                    kind: PermissionKindView::RejectOnce,
                },
            ],
            resolved: None,
        })
    };

    let pane = cx
        .update_window(handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let pane = ws.create_agent_chat_pane(
                    Some(PaneCwd::Local(tmp.clone())),
                    None,
                    daruda_config::AgentDefinition::claude_default().id,
                    None,
                    window,
                    cx,
                );
                let id = pane.id;
                ws.active_runtime_mut().panes.push(pane);
                let view = ws.agent_chat_view(id).cloned().expect("view present");
                view.update(cx, |v, _| {
                    v.items = vec![card(100), card(200)];
                    v.pending_permissions.insert(100);
                    v.pending_permissions.insert(200);
                });
                id
            })
        })
        .unwrap();
    cx.run_until_parked();

    // Phone-tap the button for id 100, then a stale re-tap for 100.
    workspace.update(cx, |ws, cx| {
        ws.respond_bot_permission(
            pane,
            100,
            PermissionDecision::Allow("allow_once".into()),
            cx,
        );
        // Already answered → is_permission_outstanding(100) is false → no-op.
        ws.respond_bot_permission(
            pane,
            100,
            PermissionDecision::Reject("reject_once".into()),
            cx,
        );
    });
    cx.run_until_parked();

    workspace.read_with(cx, |ws, cx| {
        let view = ws.agent_chat_view(pane).cloned().unwrap();
        let view = view.read(cx);
        let ChatItem::Permission(first) = &view.items[0] else {
            panic!("expected first permission card");
        };
        let ChatItem::Permission(second) = &view.items[1] else {
            panic!("expected second permission card");
        };
        assert_eq!(
            first.resolved,
            Some(PermissionResolution::Chosen("allow_once".to_string())),
            "the tapped id resolves to its own option; the stale re-tap is a no-op"
        );
        assert_eq!(second.resolved, None, "the other permission stays live");
        assert!(view.is_permission_outstanding(200));
        assert!(!view.is_permission_outstanding(100));
    });

    // Phone-tap id 200 → the pane drains fully.
    workspace.update(cx, |ws, cx| {
        ws.respond_bot_permission(
            pane,
            200,
            PermissionDecision::Reject("reject_once".into()),
            cx,
        );
    });
    cx.run_until_parked();

    workspace.read_with(cx, |ws, cx| {
        let view = ws.agent_chat_view(pane).cloned().unwrap();
        let view = view.read(cx);
        let ChatItem::Permission(second) = &view.items[1] else {
            panic!("expected second permission card");
        };
        assert_eq!(
            second.resolved,
            Some(PermissionResolution::Chosen("reject_once".to_string())),
        );
        assert!(!view.has_pending_permission());
    });
}

/// Construct a Workspace wrapped in `gpui_component::Root` — matches the
/// production windowing path so APIs that walk the window root don't
/// panic during construction. Local adaptation of
/// `window_registry.rs`'s test-only `make_window` helper (that helper is
/// private to its own module, so it isn't reachable from here).
fn make_window(
    cx: &mut gpui::TestAppContext,
    config: &daruda_config::Config,
) -> (
    gpui::WindowHandle<gpui_component::Root>,
    gpui::Entity<super::Workspace>,
) {
    crate::test_support::init_gpui_component(cx);
    let workspace_for_root = std::cell::RefCell::new(None);
    let wh = cx.add_window(|window, cx| {
        let workspace = cx.new(|cx| super::Workspace::new(config, test_data_dir(), window, cx));
        *workspace_for_root.borrow_mut() = Some(workspace.clone());
        gpui_component::Root::new(workspace, window, cx)
    });
    let workspace = workspace_for_root.borrow().clone().unwrap();
    (wh, workspace)
}

fn test_data_dir() -> std::path::PathBuf {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    std::env::temp_dir().join(format!("daruda_telegram_ops_test_{id}"))
}
