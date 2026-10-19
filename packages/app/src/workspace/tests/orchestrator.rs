//! `/daruda` routing through a registered host with a hidden, disconnected slot.
//!
//! The invariant with the most to lose is that the orchestrator's own pane
//! never appears in `/list`: it is not the user's work, and offering it as an
//! ordinal would let a `/say` reach it by number.

use gpui::{BorrowAppContext as _, TestAppContext};

use crate::control::result::{AskDisposition, ControlError, ControlResult};
use crate::control::spec::{ControlCommand, ResolvedCommand};
use crate::telegram::bridge::{BridgeCore, InboundAction, PaneRef};
use crate::telegram::client::{Update, UpdateKind};
use crate::test_support::workspace_with_agent_chat;
use crate::window_registry::WindowRegistry;

fn message(update_id: i64, chat_id: i64, text: &str) -> Update {
    Update {
        update_id,
        kind: UpdateKind::Message {
            chat_id,
            text: text.to_string(),
            reply_to_message_id: None,
        },
    }
}

fn enable_orchestrator(cx: &mut TestAppContext) {
    cx.update(|cx| {
        crate::settings_store::SettingsStore::init(cx);
        cx.update_global::<crate::settings_store::SettingsStore, _>(|store, _| {
            store.set_user_for_testing(daruda_config::Config {
                orchestrator: daruda_config::OrchestratorConfig {
                    enabled: true,
                    ..Default::default()
                },
                ..daruda_config::Config::default()
            });
        });
    });
}

/// Every `PaneRef` a `/list` offers as an ordinal.
fn listed_targets(cx: &mut TestAppContext) -> Vec<PaneRef> {
    cx.update(|cx| {
        crate::control::exec::listing(cx)
            .windows
            .into_iter()
            .flat_map(|w| w.projects)
            .flat_map(|p| p.lanes)
            .flat_map(|l| l.chats)
            .map(|c| c.target)
            .collect()
    })
}

/// The router has to classify it as a command, not as plain text — plain text
/// is injected into the last-pinged pane, which would send the whole
/// `/daruda …` line to whichever agent happened to speak last.
#[test]
fn daruda_routes_as_a_command_with_its_text_verbatim() {
    let mut core = BridgeCore::new(true, Some(42), 0);
    let action = core
        .route(message(1, 42, "/daruda make me a pane"))
        .action
        .ready();
    assert_eq!(
        action,
        InboundAction::RunCommand {
            command: ControlCommand::Ask {
                text: "make me a pane".into()
            }
        }
    );
}

/// The regression this whole split exists for: the orchestrator's pane is a
/// real agent chat in a real `Workspace`, and it must still not be listable.
#[gpui::test]
async fn the_orchestrator_pane_stays_out_of_the_listing(cx: &mut TestAppContext) {
    let user = workspace_with_agent_chat(cx);
    let orchestrator = crate::test_support::register_test_orchestrator(cx);

    let targets = listed_targets(cx);
    assert!(
        targets.iter().any(|t| t.pane == user.pane()),
        "the user's pane is listed: {targets:?}"
    );
    assert!(
        !targets.contains(&orchestrator),
        "the orchestrator must stay out of /list: {targets:?}"
    );
    cx.update(|cx| {
        assert_eq!(crate::control::exec::brief(cx).total, 1, "/brief agrees");
    });
}

/// Per-window machinery still has to reach it — that is what carries the
/// answer back to the phone once the turn settles.
#[gpui::test]
async fn the_orchestrator_window_is_still_walked_by_the_pulse(cx: &mut TestAppContext) {
    let _user = workspace_with_agent_chat(cx);
    let _orchestrator = crate::test_support::register_test_orchestrator(cx);
    cx.update(|cx| {
        let mut seen = 0usize;
        WindowRegistry::for_each_workspace(cx, |_, _, _| seen += 1);
        assert_eq!(seen, 2, "the pulse reaches both");
    });
}

/// A live orchestrator takes the prompt without opening anything, and the
/// answer is an acceptance rather than the agent's reply.
#[gpui::test]
async fn a_daruda_prompt_reaches_the_live_orchestrator(cx: &mut TestAppContext) {
    let _orchestrator = crate::test_support::register_test_orchestrator(cx);
    // `destination` goes through `orchestrator::ensure`, which wants a control
    // surface before it will reuse a live pane — and a test must not bind the
    // profile's socket.
    cx.update(crate::orchestrator::seed_control_surface_for_test);
    enable_orchestrator(cx);
    cx.update(|cx| {
        let (destination, connecting) =
            crate::orchestrator::destination(cx).expect("the orchestrator is up");
        assert_eq!(
            crate::control::exec::run(
                ResolvedCommand::AskOrchestrator {
                    text: "make me a pane".into(),
                    destination,
                    connecting,
                },
                crate::control::exec::CommandOrigin::Orchestrator,
                cx,
            ),
            Ok(ControlResult::Accepted {
                disposition: AskDisposition::Queued
            })
        );
    });
}

/// Asking makes the orchestrator the plain-text target: its answer arrives as
/// a bystander ping, so without this a follow-up would reach whichever worker
/// spoke before the question.
#[gpui::test]
async fn asking_the_orchestrator_makes_it_the_conversation(cx: &mut TestAppContext) {
    let worker = workspace_with_agent_chat(cx);
    let orchestrator = crate::test_support::register_test_orchestrator(cx);
    cx.update(crate::orchestrator::seed_control_surface_for_test);
    enable_orchestrator(cx);
    let _outbound = cx.update(|cx| crate::telegram::global::install_for_test(true, Some(42), cx));
    let worker = PaneRef {
        workspace: worker.workspace.read_with(cx, |ws, _| ws.uuid()),
        pane: worker.pane(),
    };
    cx.update(|cx| {
        cx.global_mut::<crate::telegram::global::TelegramBridge>()
            .command_state_mut()
            .note_ping(worker)
    });

    let mut async_cx = cx.to_async();
    crate::remote_channel::dispatch::handle(
        crate::remote_channel::dispatch::target::Aimed {
            action: InboundAction::RunCommand {
                command: ControlCommand::Ask {
                    text: "what is left?".into(),
                },
            },
            adopted: None,
        },
        &crate::remote_channel::dispatch::Target::Telegram,
        &mut async_cx,
    );

    cx.update(|cx| {
        let state = cx
            .global_mut::<crate::telegram::global::TelegramBridge>()
            .command_state_mut();
        assert_eq!(state.plain_text_target(None, None), Some(orchestrator));
        assert_eq!(
            state.selected(),
            None,
            "asking does not change what /stop means"
        );
    });
}

/// With nothing configured, the prompt is refused up front — no window, and
/// nothing sent to any of the user's panes.
#[gpui::test]
async fn a_daruda_prompt_with_no_orchestrator_configured_is_refused(cx: &mut TestAppContext) {
    let _user = workspace_with_agent_chat(cx);
    cx.update(|cx| {
        crate::settings_store::SettingsStore::init(cx);
        assert_eq!(
            crate::orchestrator::destination(cx),
            Err(ControlError::OrchestratorDisabled)
        );
        assert!(WindowRegistry::orchestrator(cx).is_none());
    });
}

/// Closing the window ends the orchestrator, so the next `/daruda` starts a
/// fresh one instead of addressing a dead pane.
#[gpui::test]
async fn closing_the_window_takes_the_orchestrator_down(cx: &mut TestAppContext) {
    let _orchestrator = crate::test_support::register_test_orchestrator(cx);
    let handle = cx.update(|cx| WindowRegistry::orchestrator(cx).expect("up").0);
    cx.update(|cx| {
        crate::windows::try_update_workspace_window(handle, cx, "test.close", |window, _cx| {
            window.remove_window();
        });
    });
    // Two cycles: `cx.update` flushes effects when its closure *returns*, so
    // the workspace's release — and the hook that clears the slot — lands
    // after the update that removed the window, not inside it.
    cx.run_until_parked();
    cx.update(|_| {});
    cx.update(|cx| {
        assert!(WindowRegistry::orchestrator(cx).is_none());
        assert!(crate::orchestrator::pane(cx).is_none());
    });
}
