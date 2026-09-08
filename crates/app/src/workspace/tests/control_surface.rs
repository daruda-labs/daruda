//! End-to-end shape of the external control surface: a `/list` fills the
//! adapter's ordinal table, `/use` makes one of those rows the target, and a
//! plain message afterwards reaches that pane with no reply-to.
//!
//! The bridge core is driven directly rather than through the poll loop —
//! `global.rs`'s loop is HTTP on both ends, and every decision it makes is
//! already in `BridgeCore` and `telegram::command`.

use gpui::TestAppContext;

use crate::control::spec::{ControlCommand, Ordinal, ResolvedCommand, UseTarget};
use crate::telegram::bridge::{BridgeCore, InboundAction, PaneRef};
use crate::telegram::client::{Update, UpdateKind};
use crate::telegram::command::{Resolution, absorb, resolve_command};
use crate::test_support::workspace_with_agent_chat;

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

#[gpui::test]
async fn list_then_use_then_plain_text_reaches_the_selected_pane(cx: &mut TestAppContext) {
    let fixture = workspace_with_agent_chat(cx);
    let mut core = BridgeCore::new(true, Some(42), 0);

    // `/list` routes as a command, and its result fills the ordinal table.
    let action = core.route(message(1, 42, "/list")).action;
    assert_eq!(
        action,
        InboundAction::RunCommand {
            command: ControlCommand::List
        }
    );
    let listing = cx.update(crate::control::exec::listing);
    absorb(
        &Ok(crate::control::result::ControlResult::Listing(listing)),
        None,
        core.command_state_mut(),
    );

    // `/use 1` selects, without ever reaching the executor.
    let resolution = resolve_command(
        ControlCommand::Use(UseTarget::Select(Ordinal(1))),
        core.command_state_mut(),
    );
    assert!(matches!(resolution, Resolution::Answer(Ok(_))));
    let target = core.command_state_mut().selected().expect("a target");
    assert_eq!(target.pane, fixture.pane());

    // Plain text now names that pane without a reply-to.
    let action = core.route(message(2, 42, "add tests too")).action;
    assert_eq!(
        action,
        InboundAction::InjectPrompt {
            pane: target,
            text: "add tests too".into(),
        }
    );
}

#[gpui::test]
async fn say_by_ordinal_resolves_to_the_listed_pane(cx: &mut TestAppContext) {
    let fixture = workspace_with_agent_chat(cx);
    let mut core = BridgeCore::new(true, Some(42), 0);

    let listing = cx.update(crate::control::exec::listing);
    absorb(
        &Ok(crate::control::result::ControlResult::Listing(listing)),
        None,
        core.command_state_mut(),
    );

    let InboundAction::RunCommand { command } = core.route(message(1, 42, "/say 1 go on")).action
    else {
        panic!("/say routes as a command");
    };
    let Resolution::Run(ResolvedCommand::Say { target, text }, addressed) =
        resolve_command(command, core.command_state_mut())
    else {
        panic!("an ordinal that is listed resolves");
    };
    assert_eq!(target.pane, fixture.pane());
    assert_eq!(text, "go on");
    assert_eq!(addressed, Some(target));

    let outcome =
        cx.update(|cx| crate::control::exec::run(ResolvedCommand::Say { target, text }, cx));
    assert!(outcome.is_ok(), "the pane accepted the prompt: {outcome:?}");
}

/// The property `global.rs`'s poll loop depends on, and the reason it routes
/// and acts on one update at a time instead of routing a whole `getUpdates`
/// batch first: routing must observe state that acting on an *earlier* update
/// changed. A batch holding `/use 1` and the message that means it is the case
/// that broke when the loop collected first.
#[gpui::test]
async fn routing_observes_state_an_earlier_update_changed(cx: &mut TestAppContext) {
    let fixture = workspace_with_agent_chat(cx);
    let mut core = BridgeCore::new(true, Some(42), 0);

    let listing = cx.update(crate::control::exec::listing);
    absorb(
        &Ok(crate::control::result::ControlResult::Listing(listing)),
        None,
        core.command_state_mut(),
    );

    // Before: the same plain text has nowhere to go.
    assert_eq!(
        core.route(message(1, 42, "add tests too")).action,
        InboundAction::NoTarget
    );

    // Act on `/use 1`, exactly as the loop would between two routes.
    let InboundAction::RunCommand { command } = core.route(message(2, 42, "/use 1")).action else {
        panic!("/use routes as a command");
    };
    let _ = resolve_command(command, core.command_state_mut());

    // After: the identical text now resolves, without any reply-to.
    assert_eq!(
        core.route(message(3, 42, "add tests too")).action,
        InboundAction::InjectPrompt {
            pane: PaneRef {
                workspace: fixture.workspace.read_with(cx, |ws, _| ws.uuid()),
                pane: fixture.pane(),
            },
            text: "add tests too".into(),
        }
    );
}

/// A command reply must not become the next plain message's destination.
/// `send_command_reply` achieves that by never calling `record_sent`; this
/// pins the consequence, which is what a regression would actually change.
#[gpui::test]
async fn answering_a_command_does_not_steal_the_plain_text_target(cx: &mut TestAppContext) {
    let fixture = workspace_with_agent_chat(cx);
    let mut core = BridgeCore::new(true, Some(42), 0);

    // A ping established the fallback target the way a real relay would.
    let pinged = PaneRef {
        workspace: fixture.workspace.read_with(cx, |ws, _| ws.uuid()),
        pane: fixture.pane(),
    };
    core.record_sent(7, pinged);

    // Answering `/list` walks the whole command path — resolve, run, absorb.
    let InboundAction::RunCommand { command } = core.route(message(1, 42, "/list")).action else {
        panic!("/list routes as a command");
    };
    let Resolution::Run(resolved, addressed) = resolve_command(command, core.command_state_mut())
    else {
        panic!("/list reaches the executor");
    };
    let outcome = cx.update(|cx| crate::control::exec::run(resolved, cx));
    absorb(&outcome, addressed, core.command_state_mut());

    // The fallback is still the pinged pane, not anything the listing named.
    assert_eq!(
        core.route(message(2, 42, "carry on")).action,
        InboundAction::InjectPrompt {
            pane: pinged,
            text: "carry on".into(),
        },
        "a /list answer must not move last_pinged"
    );
}

/// A number the user typed against a listing they never asked for has nowhere
/// to point — and must say so rather than picking whatever is at that index.
#[gpui::test]
async fn an_ordinal_with_no_listing_behind_it_is_refused(cx: &mut TestAppContext) {
    let _fixture = workspace_with_agent_chat(cx);
    let mut core = BridgeCore::new(true, Some(42), 0);

    let InboundAction::RunCommand { command } = core.route(message(1, 42, "/say 1 go on")).action
    else {
        panic!("/say routes as a command");
    };
    assert!(matches!(
        resolve_command(command, core.command_state_mut()),
        Resolution::Answer(Err(crate::control::result::ControlError::OrdinalNotFound {
            ordinal: 1
        }))
    ));
}
