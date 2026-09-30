//! What a phone is told: who is asking, that a turn failed, and what a
//! finished run did.

use futures::{FutureExt as _, StreamExt as _};

use super::tests::{expect_ping, make_window, prompt};
use crate::surface::strings as s;
use crate::telegram::bridge::{Outbound, TelegramTail};
use crate::workspace::main_area::pane_tree::PaneId;
use daruda_acp::{ChatItem, PermissionChoice, PermissionKindView};

type Outbox = futures::channel::mpsc::UnboundedReceiver<Outbound>;

/// A paired bridge that sends whatever the presence gate would, and one agent
/// chat pane.
fn bridged_pane(cx: &mut gpui::TestAppContext) -> (Outbox, gpui::Entity<super::Workspace>, PaneId) {
    let outbound = cx.update(|cx| crate::telegram::global::install_for_test(true, Some(42), cx));
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
    (outbound, workspace, pane)
}

fn sent(outbound: &mut Outbox) -> crate::telegram::bridge::BridgePing {
    expect_ping(
        outbound
            .next()
            .now_or_never()
            .flatten()
            .expect("a ping was sent"),
    )
}

fn name_tab(
    ws: &mut super::Workspace,
    pane: PaneId,
    name: &str,
    cx: &mut gpui::Context<super::Workspace>,
) {
    let tab_id = ws
        .active_runtime()
        .tabs
        .iter()
        .find(|t| t.layout.contains(pane))
        .map(|t| t.id)
        .expect("the pane sits in a tab");
    ws.rename_tab(tab_id, Some(name.into()), cx);
}

/// The one ping that asks the user to decide carries the same who-is-this
/// lines as every other: project, agent and the tab's name.
#[gpui::test]
async fn a_permission_wait_says_which_project_agent_and_tab_asks(cx: &mut gpui::TestAppContext) {
    let (mut outbound, workspace, pane) = bridged_pane(cx);
    let options = vec![PermissionChoice {
        option_id: "allow_once".into(),
        name: "Allow".into(),
        kind: PermissionKindView::AllowOnce,
    }];
    workspace.update(cx, |ws, cx| {
        name_tab(ws, pane, "review", cx);
        ws.relay_permission_wait_to_telegram(pane, &prompt(7, &options, Some("Write x.rs")), cx);
        let ping = sent(&mut outbound);
        let project = ws.project_name_for_pane(pane).expect("an owning project");
        let agent = ws
            .agent_chat_view(pane)
            .unwrap()
            .read(cx)
            .agent_name
            .clone();
        assert_eq!(
            ping.header,
            format!("{project}\n{}", s::remote_agent_with_tab(&agent, "review"))
        );
    });
}

/// A turn that ended in an error is news to a phone waiting on it: without
/// this the phone hears nothing, ever.
#[gpui::test]
async fn a_failed_turn_tells_the_phone_why(cx: &mut gpui::TestAppContext) {
    let (mut outbound, workspace, pane) = bridged_pane(cx);
    workspace.update(cx, |ws, cx| {
        let view = ws.agent_chat_view(pane).cloned().unwrap();
        view.update(cx, |v, _| {
            v.items.push(ChatItem::UserText("go".into()));
            v.items
                .push(ChatItem::Failure(daruda_acp::AcpFailure::TransportClosed {
                    message: "adapter exited".into(),
                }));
        });
        ws.fire_activity_completion(pane, super::super::view::TurnOutcome::Errored, cx);
        let ping = sent(&mut outbound);
        assert_eq!(ping.header, ws.telegram_header(pane, cx));
        let TelegramTail::Plain(tail) = ping.tail else {
            panic!("a failure is our own copy, not agent markdown");
        };
        assert!(tail.starts_with(&s::remote_turn_failed()), "{tail}");
        assert!(tail.contains(&s::agent_chat_transport_closed()), "{tail}");
    });
}

/// A session that died without a failure row still says what it died of.
#[gpui::test]
async fn a_dead_session_tells_the_phone_its_error(cx: &mut gpui::TestAppContext) {
    let (mut outbound, workspace, pane) = bridged_pane(cx);
    workspace.update(cx, |ws, cx| {
        let view = ws.agent_chat_view(pane).cloned().unwrap();
        view.update(cx, |v, _| {
            v.status = super::super::view::AgentSessionStatus::Error {
                message: "adapter exited with 137".into(),
                remedy: daruda_acp::Remedy::NoneAvailable,
            };
        });
        ws.fire_activity_completion(pane, super::super::view::TurnOutcome::Errored, cx);
        let TelegramTail::Plain(tail) = sent(&mut outbound).tail else {
            panic!("plain");
        };
        assert!(tail.contains("adapter exited with 137"), "{tail}");
    });
}

/// A Stop is the user's own doing; nothing is sent for it.
#[gpui::test]
async fn a_stopped_turn_sends_nothing(cx: &mut gpui::TestAppContext) {
    let (mut outbound, workspace, pane) = bridged_pane(cx);
    workspace.update(cx, |ws, cx| {
        ws.fire_activity_completion(pane, super::super::view::TurnOutcome::Stopped, cx);
    });
    assert!(outbound.next().now_or_never().is_none());
}

fn tool(id: &str, kind: daruda_acp::ToolKindView) -> ChatItem {
    ChatItem::ToolCall(daruda_acp::ToolCallItem {
        id: id.into(),
        title: id.into(),
        kind,
        tool_name: None,
        status: daruda_acp::ToolStatusView::Completed,
        diffs: Vec::new(),
        output: Vec::new(),
        raw_input: None,
        parent_tool_id: None,
        locations: Vec::new(),
        exit: None,
    })
}

/// A finished run's ping says how long it took and what it did, so the size
/// of the work reads without opening the answer.
#[gpui::test]
async fn a_completed_run_is_summarised_in_one_line(cx: &mut gpui::TestAppContext) {
    use daruda_acp::ToolKindView as K;
    let (mut outbound, workspace, pane) = bridged_pane(cx);
    workspace.update(cx, |ws, cx| {
        let view = ws.agent_chat_view(pane).cloned().unwrap();
        view.update(cx, |v, _| {
            let mut nested = tool("nested", K::Execute);
            let ChatItem::ToolCall(nested_call) = &mut nested else {
                unreachable!()
            };
            nested_call.parent_tool_id = Some("e1".into());
            v.items = vec![
                ChatItem::UserText("go".into()),
                tool("e1", K::Edit),
                tool("e2", K::Edit),
                tool("x1", K::Execute),
                nested,
                ChatItem::AssistantText {
                    text: "done".into(),
                    streaming: false,
                    message_id: None,
                    phase: Default::default(),
                },
            ];
            v.record_turn_for_test(std::time::Duration::from_secs(125));
        });
        ws.fire_activity_completion(pane, super::super::view::TurnOutcome::Completed, cx);
        let ping = sent(&mut outbound);
        let summary = ping.header.lines().last().expect("a header").to_string();
        assert!(ping.header.starts_with(&ws.telegram_header(pane, cx)));
        let took = s::format_duration_compact(std::time::Duration::from_secs(125));
        assert!(summary.contains(&took), "{summary}");
        assert!(
            summary.contains(&s::agent_chat_group_category("edit", 2)),
            "{summary}"
        );
        assert!(
            summary.contains(&s::agent_chat_group_category("run", 1)),
            "{summary}"
        );
        assert!(!summary.contains(&s::agent_chat_group_category("run", 2)));
    });
}

/// A request to change a file shows the change, so it can be judged on the
/// phone rather than approved blind.
#[gpui::test]
async fn a_permission_to_edit_a_file_shows_the_edit(cx: &mut gpui::TestAppContext) {
    let (mut outbound, workspace, pane) = bridged_pane(cx);
    let prompt = daruda_acp::PermissionItem {
        id: 7,
        tool_call_id: "t1".into(),
        tool_title: Some("Edit x.rs".into()),
        raw_input_summary: None,
        options: vec![PermissionChoice {
            option_id: "allow_once".into(),
            name: "Allow".into(),
            kind: PermissionKindView::AllowOnce,
        }],
        resolved: None,
    };
    workspace.update(cx, |ws, cx| {
        let view = ws.agent_chat_view(pane).cloned().unwrap();
        view.update(cx, |v, _| {
            let ChatItem::ToolCall(mut call) = tool("t1", daruda_acp::ToolKindView::Edit) else {
                unreachable!()
            };
            call.diffs = vec![daruda_acp::DiffView {
                path: "/repo/x.rs".into(),
                old_text: Some("old line\n".into()),
                new_text: "new line\n".into(),
            }];
            v.items.push(ChatItem::ToolCall(call));
        });
        ws.relay_permission_wait_to_telegram(pane, &prompt, cx);
        let TelegramTail::Markdown(tail) = sent(&mut outbound).tail else {
            panic!("a diff preview is intentional markdown");
        };
        assert!(
            tail.contains("```diff\nx.rs\n- old line\n+ new line\n```"),
            "{tail}"
        );
    });
}

/// A terminal hook cannot carry a response button, but its blocking notice
/// still reaches the paired phone as an unattributed notice.
#[gpui::test]
async fn a_terminal_hook_notice_reaches_the_phone_without_buttons(cx: &mut gpui::TestAppContext) {
    let (mut outbound, workspace, _pane) = bridged_pane(cx);
    workspace.update(cx, |ws, cx| {
        ws.relay_presence_notice_to_phone("Claude Code needs permission\n/repo".into(), cx);
    });
    let message = outbound
        .next()
        .now_or_never()
        .flatten()
        .expect("a notice was sent");
    let Outbound::Notice(text) = message else {
        panic!("a terminal hook must not target an agent pane");
    };
    assert!(text.contains("needs permission"), "{text}");
}
