//! End-to-end shape of the external control surface: a `/list` fills the
//! adapter's ordinal table, `/use` makes one of those rows the target, and a
//! plain message afterwards reaches that pane with no reply-to.
//!
//! The bridge core is driven directly rather than through the poll loop —
//! `global.rs`'s loop is HTTP on both ends, and every decision it makes is
//! already in `BridgeCore` and `telegram::command`.

use gpui::TestAppContext;

use crate::control::spec::{ControlCommand, Ordinal, ResolvedCommand, UseTarget};
use crate::telegram::bridge::{BridgeCore, InboundAction, PaneRef, Routed, Unaimed};
use crate::telegram::client::{Update, UpdateKind};
use crate::telegram::command::{Resolution, absorb, resolve_command};
use crate::test_support::{ControlFixture, workspace_for_control, workspace_with_agent_chat};
use crate::workspace::SlashClaim;
use crate::workspace::main_area::pane_tree::PaneId;
use gpui::AppContext as _;

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
    let action = core.route(message(1, 42, "/list")).action.ready();
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
    let action = core.route(message(2, 42, "add tests too")).action.ready();
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

    let InboundAction::RunCommand { command } =
        core.route(message(1, 42, "/say 1 go on")).action.ready()
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
        Routed::NeedsTarget(Unaimed::Text {
            text: "add tests too".into()
        })
    );

    // Act on `/use 1`, exactly as the loop would between two routes.
    let InboundAction::RunCommand { command } = core.route(message(2, 42, "/use 1")).action.ready()
    else {
        panic!("/use routes as a command");
    };
    let _ = resolve_command(command, core.command_state_mut());

    // After: the identical text now resolves, without any reply-to.
    assert_eq!(
        core.route(message(3, 42, "add tests too")).action.ready(),
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
    let InboundAction::RunCommand { command } = core.route(message(1, 42, "/list")).action.ready()
    else {
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
        core.route(message(2, 42, "carry on")).action.ready(),
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

    let InboundAction::RunCommand { command } =
        core.route(message(1, 42, "/say 1 go on")).action.ready()
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

/// The settle edge is what answers a `daruda_chat_ask`, so the wait and the
/// completion signals cannot disagree about when a turn is done. Driven
/// through the real pulse tick — the time-based settle driver — rather than by
/// calling the tee, because the edge detection is the part under test.
mod ask {
    use super::*;
    use crate::control::result::PaneAnswer;
    use daruda_acp::ChatItem;

    /// How the turn under test ended. Three outcomes, and `completed_normally`
    /// alone cannot name the third.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum TurnEnd {
        Completed,
        /// A Stop — from the desk or from another surface.
        Cancelled,
        Failed,
    }

    fn said(text: &str) -> ChatItem {
        ChatItem::AssistantText {
            text: text.into(),
            streaming: false,
            message_id: None,
            phase: Default::default(),
        }
    }

    /// Put a pane through busy → idle with `items` in its transcript, and
    /// return what the waiting call was answered with.
    ///
    /// The turn is ended with a real `TurnEnded`, not by parking the turn
    /// field: the settle edge reports the outcome that event *stashes*, so a
    /// hand-parked turn produces no edge at all and would make this pass
    /// vacuously.
    fn answer_after_a_turn(
        items: Vec<ChatItem>,
        end: TurnEnd,
        cx: &mut TestAppContext,
    ) -> Option<PaneAnswer> {
        let fixture = workspace_with_agent_chat(cx);
        let target = fixture
            .workspace
            .read_with(cx, |ws, cx| ws.control_snapshot(cx)[0].1.target);
        let waiter = cx.update(|cx| {
            let (_id, _deadline, rx) = crate::control::ask::wait_for(target, cx);
            rx
        });

        cx.update_window(fixture.window.into(), |_, _window, cx| {
            fixture.workspace.update(cx, |ws, cx| {
                let view = ws.agent_chat_view(target.pane).expect("pane").clone();
                view.update(cx, |v, _| {
                    v.set_turn_in_flight();
                    v.items = items;
                });
                // Busy is observed first, so the tick has an edge to detect.
                ws.pulse_agent_chats(cx);
                match end {
                    TurnEnd::Completed | TurnEnd::Cancelled => {
                        let completed_normally = end == TurnEnd::Completed;
                        view.update(cx, |v, cx| {
                            v.apply_event(
                                daruda_acp::AcpEvent::TurnEnded {
                                    stop_reason: "end_turn".into(),
                                    completed_normally,
                                    usage: None,
                                },
                                "base16-ocean.dark",
                                false,
                                cx,
                            );
                        });
                    }
                    // The error arm stashes its own outcome, so it is reached
                    // by the failure event rather than by `TurnEnded`.
                    TurnEnd::Failed => {
                        view.update(cx, |v, cx| {
                            v.apply_event(
                                daruda_acp::AcpEvent::TurnFailed(
                                    daruda_acp::AcpFailure::Unclassified {
                                        message: "the adapter gave up".into(),
                                    },
                                ),
                                "base16-ocean.dark",
                                false,
                                cx,
                            );
                        });
                    }
                }
                ws.pulse_agent_chats(cx);
            });
        })
        .expect("window is live");

        waiter.try_recv().ok().flatten()
    }

    #[gpui::test]
    async fn an_ask_answers_with_the_text_the_turn_produced(cx: &mut TestAppContext) {
        assert_eq!(
            answer_after_a_turn(vec![said("here is the summary")], TurnEnd::Completed, cx),
            Some(PaneAnswer::Text {
                text: "here is the summary".into()
            })
        );
    }

    /// A turn that only ran tools said nothing, which is a different answer
    /// from an empty string — a caller told `""` cannot tell the two apart.
    #[gpui::test]
    async fn a_tool_only_turn_answers_no_answer(cx: &mut TestAppContext) {
        assert_eq!(
            answer_after_a_turn(Vec::new(), TurnEnd::Completed, cx),
            Some(PaneAnswer::NoAnswer)
        );
    }

    /// A message still streaming when the turn ends is *finished* by
    /// `settle_turn`, so by the settle edge it is what the agent said. Pinned
    /// because the reader skips streaming messages, and that skip must not be
    /// read as "an interrupted answer is lost".
    #[gpui::test]
    async fn text_still_streaming_when_the_turn_ends_is_the_answer(cx: &mut TestAppContext) {
        let streaming = ChatItem::AssistantText {
            text: "partial".into(),
            streaming: true,
            message_id: None,
            phase: Default::default(),
        };
        assert_eq!(
            answer_after_a_turn(vec![streaming], TurnEnd::Completed, cx),
            Some(PaneAnswer::Text {
                text: "partial".into()
            })
        );
    }

    /// Where the skip does matter: a read taken *while* a turn is running must
    /// not present a half-written sentence as the agent's answer. This is what
    /// separates `daruda_chat_read` from `daruda_chat_ask`.
    #[gpui::test]
    async fn a_mid_turn_read_skips_a_streaming_message(cx: &mut TestAppContext) {
        let fixture = workspace_with_agent_chat(cx);
        let target = fixture
            .workspace
            .read_with(cx, |ws, cx| ws.control_snapshot(cx)[0].1.target);
        fixture.workspace.update(cx, |ws, cx| {
            let view = ws.agent_chat_view(target.pane).expect("pane").clone();
            view.update(cx, |v, _| {
                v.set_turn_in_flight();
                v.items = vec![
                    said("the previous turn's answer"),
                    ChatItem::AssistantText {
                        text: "half a sen".into(),
                        streaming: true,
                        message_id: None,
                        phase: Default::default(),
                    },
                ];
            });
            assert_eq!(
                ws.control_read(target.pane, cx).expect("pane is there"),
                Some("the previous turn's answer".to_string()),
                "a fragment must not be reported as what it said"
            );
        });
    }

    /// A turn somebody stopped is not an answer. `settle_run_state` finalises
    /// whatever was streaming, so the transcript *does* hold text — reporting
    /// that as the reply would hand the caller a cut-off sentence as though
    /// the agent had meant it.
    #[gpui::test]
    async fn a_stopped_turn_answers_interrupted_not_its_partial_text(cx: &mut TestAppContext) {
        assert_eq!(
            answer_after_a_turn(vec![said("half an ans")], TurnEnd::Cancelled, cx),
            Some(PaneAnswer::Interrupted)
        );
    }

    /// The error state the tool advertises to the model, which nothing
    /// exercised end to end.
    #[gpui::test]
    async fn an_errored_turn_answers_failed(cx: &mut TestAppContext) {
        assert_eq!(
            answer_after_a_turn(vec![said("got this far")], TurnEnd::Failed, cx),
            Some(PaneAnswer::Failed)
        );
    }

    /// A pane nobody asked about must not be charged for the lookup with a
    /// spurious answer going nowhere.
    #[gpui::test]
    async fn a_turn_nobody_asked_about_leaves_no_waiter(cx: &mut TestAppContext) {
        let fixture = workspace_with_agent_chat(cx);
        let target = fixture
            .workspace
            .read_with(cx, |ws, cx| ws.control_snapshot(cx)[0].1.target);
        cx.update_window(fixture.window.into(), |_, _window, cx| {
            fixture.workspace.update(cx, |ws, cx| {
                let view = ws.agent_chat_view(target.pane).expect("pane").clone();
                view.update(cx, |v, _| {
                    v.set_turn_in_flight();
                    v.items = vec![said("nobody is listening")];
                });
                ws.pulse_agent_chats(cx);
                view.update(cx, |v, _| v.set_turn_idle());
                ws.pulse_agent_chats(cx);
            });
        })
        .expect("window is live");
        cx.update(|cx| {
            assert_eq!(crate::control::ask::waiting_count_for_test(cx), 0);
        });
    }
}

/// Where a phone message goes when nothing names a target.
///
/// The bridge's selection and last-pinged pane are in-memory, so every app
/// restart drops them and the phone had to be re-aimed by hand before it could
/// say anything. The app's own active lane answers the question without a
/// round trip; it is deliberately narrow, because guessing between two agents
/// is worse than asking.
/// Open one more chat in the lane the fixture already set up, and return its
/// pane id. The fixture leaves focus on its terminal pane, so this makes the
/// lane hold two chats with neither of them focused.
fn second_chat(fixture: &ControlFixture, cx: &mut TestAppContext) -> PaneId {
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture
            .workspace
            .update(cx, |ws, cx| ws.open_agent_chat_pane_for_test(window, cx))
    })
    .expect("window is live")
}

/// Give the fixture the hidden orchestrator chat a real session grows, so a
/// test can check that a pane the phone is never shown stays out of the
/// answers about panes it could be.
fn seed_orchestrator(fixture: &ControlFixture, cx: &mut TestAppContext) -> PaneId {
    let cwd = std::env::temp_dir();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.workspace.update(cx, |ws, cx| {
            ws.seed_orchestrator_chat_pane_unrevealed_for_test(
                ws.agents[0].id.clone(),
                cwd,
                daruda_store::accounts::AccountSelection::SystemDefault,
                None,
                window,
                cx,
            )
            .expect("seeded")
        })
    })
    .expect("window is live")
}

mod fallback_target {
    use super::*;

    /// The fixture focuses its terminal, so this is the "only chat in the
    /// lane" branch — the one that actually fires after a restart, when the
    /// restored focus is on whatever the user last clicked.
    #[gpui::test]
    async fn the_active_lanes_only_chat_is_the_fallback(cx: &mut TestAppContext) {
        let fixture = workspace_with_agent_chat(cx);
        let expected = fixture.pane_ref(cx);
        assert_eq!(
            fixture
                .workspace
                .read_with(cx, |ws, _| ws.fallback_agent_chat()),
            Some(expected),
            "one chat in the lane the user is in is not a guess"
        );
    }

    /// Nothing to fall back to is still an honest answer — the phone hears
    /// "no target" rather than reaching a pane that is not a chat.
    #[gpui::test]
    async fn a_lane_with_no_chat_has_no_fallback(cx: &mut TestAppContext) {
        let fixture = workspace_for_control(cx);
        assert_eq!(
            fixture
                .workspace
                .read_with(cx, |ws, _| ws.fallback_agent_chat()),
            None
        );
    }

    /// Two chats and focus on neither: a pick between two agents would start
    /// a turn in the wrong one, so the phone is told to choose instead.
    #[gpui::test]
    async fn two_chats_with_neither_focused_refuse_to_guess(cx: &mut TestAppContext) {
        let fixture = workspace_with_agent_chat(cx);
        second_chat(&fixture, cx);
        assert_eq!(
            fixture
                .workspace
                .read_with(cx, |ws, _| ws.fallback_agent_chat()),
            None,
            "two chats and no focus is ambiguous, not a coin flip"
        );
    }

    /// Focus settles that ambiguity: the chat the user is actually in wins
    /// over the other one, rather than whichever the pane list holds first.
    #[gpui::test]
    async fn a_focused_chat_wins_over_the_other_one(cx: &mut TestAppContext) {
        let fixture = workspace_with_agent_chat(cx);
        let second = second_chat(&fixture, cx);
        fixture.workspace.update(cx, |ws, _| {
            ws.active_runtime_mut().focused_pane_id = second;
        });
        assert_eq!(
            fixture
                .workspace
                .read_with(cx, |ws, _| ws.fallback_agent_chat())
                .map(|p| p.pane),
            Some(second)
        );
    }
}

/// Which vocabulary a `/name` from the phone belongs to.
///
/// daruda owns seven names and the agent's set is open, so they share the `/`
/// namespace. The regression: daruda answered `/usage` — Claude's — with a
/// suggestion for `/use`, its own, and the agent never saw it. The agent's
/// advertised list is the arbiter, and the in-app completion menu already
/// reads the same field.
mod unowned_slash {
    use super::*;

    #[gpui::test]
    async fn a_command_the_agent_advertises_goes_to_the_agent(cx: &mut TestAppContext) {
        let fixture = workspace_with_agent_chat(cx);
        let pane = fixture.pane();
        fixture.workspace.update(cx, |ws, cx| {
            ws.advertise_slash_commands_for_test(pane, &["usage", "cost", "model"], cx);
            assert!(
                ws.agent_takes_slash_command(pane, "usage", cx),
                "the agent advertises /usage, so /usage is the agent's"
            );
        });
    }

    /// The other side: with a list that lacks the name, daruda really does
    /// know better, and the typo answer is the useful one.
    #[gpui::test]
    async fn a_name_the_agent_does_not_have_stays_ours(cx: &mut TestAppContext) {
        let fixture = workspace_with_agent_chat(cx);
        let pane = fixture.pane();
        fixture.workspace.update(cx, |ws, cx| {
            ws.advertise_slash_commands_for_test(pane, &["usage", "cost"], cx);
            assert!(
                !ws.agent_takes_slash_command(pane, "lst", cx),
                "a typo of /list is not one of the agent's, so daruda answers it"
            );
        });
    }

    /// An empty list is "the session has not said yet", not "it does not have
    /// it". Forwarding is the safe reading — a cold pane must not turn every
    /// agent command into a typo answer.
    #[gpui::test]
    async fn an_agent_that_has_advertised_nothing_still_gets_it(cx: &mut TestAppContext) {
        let fixture = workspace_with_agent_chat(cx);
        let pane = fixture.pane();
        fixture.workspace.update(cx, |ws, cx| {
            ws.advertise_slash_commands_for_test(pane, &[], cx);
            assert!(ws.agent_takes_slash_command(pane, "usage", cx));
        });
    }

    /// A pane that is gone forwards too, so the answer comes from the delivery
    /// attempt — which can say `target_gone` — rather than from a guess.
    #[gpui::test]
    async fn a_pane_that_is_gone_is_not_answered_as_a_typo(cx: &mut TestAppContext) {
        let fixture = workspace_with_agent_chat(cx);
        fixture.workspace.update(cx, |ws, cx| {
            assert!(ws.agent_takes_slash_command(9999, "usage", cx));
        });
    }

    /// The same question with no target to ask it of. Having nowhere to send
    /// `/usage` never made it daruda's, so an agent that advertises the name
    /// still keeps the typo answer off it — the phone hears the real reason
    /// (no target) instead of a suggestion for a command nobody wanted.
    #[gpui::test]
    async fn an_advertised_name_is_not_ruled_out_when_nothing_names_a_target(
        cx: &mut TestAppContext,
    ) {
        let fixture = workspace_with_agent_chat(cx);
        let pane = fixture.pane();
        fixture.workspace.update(cx, |ws, cx| {
            ws.advertise_slash_commands_for_test(pane, &["usage", "cost", "model"], cx);
            assert_eq!(ws.slash_claim("usage", cx), SlashClaim::Claims);
            assert!(!ws.slash_claim("usage", cx).rules_out());
        });
    }

    /// The other side: an agent that has advertised its list and does not name
    /// it really does rule it out, and the suggestion is the useful answer.
    #[gpui::test]
    async fn a_name_an_advertised_agent_lacks_is_ruled_out(cx: &mut TestAppContext) {
        let fixture = workspace_with_agent_chat(cx);
        let pane = fixture.pane();
        fixture.workspace.update(cx, |ws, cx| {
            ws.advertise_slash_commands_for_test(pane, &["usage", "cost"], cx);
            assert!(ws.slash_claim("lst", cx).rules_out());
        });
    }

    /// A cold pane abstains rather than claiming. The regression this guards:
    /// as a claim it vetoed every name for the whole window, and a restored
    /// chat stays `Idle` until first focus — so after a restart, the state
    /// this whole path exists for, almost every pane is cold.
    #[gpui::test]
    async fn a_cold_pane_abstains_instead_of_vetoing_an_advertised_one(cx: &mut TestAppContext) {
        let fixture = workspace_with_agent_chat(cx);
        let advertised = fixture.pane();
        let cold = second_chat(&fixture, cx);
        fixture.workspace.update(cx, |ws, cx| {
            ws.advertise_slash_commands_for_test(advertised, &["usage", "cost"], cx);
            ws.advertise_slash_commands_for_test(cold, &[], cx);
            assert_eq!(ws.slash_claim("lst", cx), SlashClaim::Disclaims);
            assert!(
                ws.slash_claim("lst", cx).rules_out(),
                "one silent pane must not overrule one that answered"
            );
            assert_eq!(
                ws.slash_claim("usage", cx),
                SlashClaim::Claims,
                "a claim still settles it over both"
            );
        });
    }

    /// Silence on its own earns no conclusion in either direction: no agent
    /// chat at all, and every session still cold, both answer `Unsaid`, and
    /// the phone hears the missing target rather than a typo claim daruda
    /// cannot support.
    #[gpui::test]
    async fn silence_alone_rules_nothing_out(cx: &mut TestAppContext) {
        let bare = workspace_for_control(cx);
        bare.workspace.update(cx, |ws, cx| {
            assert_eq!(ws.slash_claim("usage", cx), SlashClaim::Unsaid);
            assert!(!ws.slash_claim("usage", cx).rules_out());
        });

        let cold = workspace_with_agent_chat(cx);
        let pane = cold.pane();
        cold.workspace.update(cx, |ws, cx| {
            ws.advertise_slash_commands_for_test(pane, &[], cx);
            assert_eq!(ws.slash_claim("usage", cx), SlashClaim::Unsaid);
        });
    }

    /// The orchestrator's vocabulary must not decide an answer about panes it
    /// is not one of: an unowned slash can only be delivered to a chat
    /// `/list` offers, and the hidden orchestrator is never one. Left in, and
    /// usually cold, it would answer `Unsaid` for every name.
    #[gpui::test]
    async fn the_orchestrators_vocabulary_is_not_consulted(cx: &mut TestAppContext) {
        let fixture = workspace_with_agent_chat(cx);
        let pane = fixture.pane();
        let orchestrator = seed_orchestrator(&fixture, cx);
        fixture.workspace.update(cx, |ws, cx| {
            ws.advertise_slash_commands_for_test(pane, &["usage", "cost"], cx);
            ws.advertise_slash_commands_for_test(orchestrator, &[], cx);
            assert!(
                ws.slash_claim("lst", cx).rules_out(),
                "the lane chat answered; the hidden orchestrator does not get a vote"
            );
        });
    }
}
