use super::*;
use daruda_store::project::WorkspaceUuid;

fn pane(n: u64) -> PaneRef {
    PaneRef {
        workspace: WorkspaceUuid::default(),
        pane: n,
    }
}

#[test]
fn opaque_ids_preserve_slack_precision_and_discord_snowflakes() {
    let mut core = RoutingCore::<String>::default();
    let slack = "1700000000.000001".to_owned();
    let discord = "18446744073709551615".to_owned();
    core.record_sent(slack.clone(), pane(1));
    core.record_sent(discord.clone(), pane(2));
    let first = core.route_text("reply".into(), Some(slack)).ready();
    let second = core.route_text("reply".into(), Some(discord)).ready();
    assert!(matches!(
        first,
        InboundAction::InjectPrompt {
            pane: PaneRef { pane: 1, .. },
            ..
        }
    ));
    assert!(matches!(
        second,
        InboundAction::InjectPrompt {
            pane: PaneRef { pane: 2, .. },
            ..
        }
    ));
}

#[test]
fn channel_instances_do_not_share_targets_or_permission_tokens() {
    let mut first = RoutingCore::<String>::default();
    let mut second = RoutingCore::<String>::default();
    let prepared = first.build_ping(BridgePing {
        pane: pane(1),
        header: "permission".into(),
        tail: super::super::MessageTail::Plain("write".into()),
        prompt: Some(crate::remote_channel::bridge::PingPrompt::Permission(
            super::super::PermissionPromptRef {
                session: 1,
                perm_id: 7,
                buttons: vec![("Allow".into(), PermissionDecision::Allow("once".into()))],
            },
        )),
    });
    let token = prepared.keyboard.unwrap().rows[0][0].1.clone();
    assert_eq!(second.route_callback(token.clone()), InboundAction::Ignore);
    assert!(matches!(
        first.route_callback(token.clone()),
        InboundAction::RespondPermission { perm_id: 7, .. }
    ));
    assert_eq!(first.route_callback(token), InboundAction::Ignore);
    assert!(matches!(
        second.route_text("hello".into(), None),
        Routed::NeedsTarget(_)
    ));
}

fn injected_into(core: &mut RoutingCore<String>) -> Option<PaneRef> {
    match core.route_text("carry on".into(), None).ready() {
        InboundAction::InjectPrompt { pane, .. } => Some(pane),
        _ => None,
    }
}

/// A pane chosen with `/use` keeps plain text only until another pane speaks:
/// the user is answering whoever spoke last.
#[test]
fn a_pane_that_speaks_after_the_selection_gets_the_plain_text() {
    let mut core = RoutingCore::<String>::default();
    core.command_state_mut().select(Some(pane(1)));
    core.record_sent("m1".into(), pane(2));
    assert_eq!(injected_into(&mut core), Some(pane(2)));
}

/// The orchestrator reporting on a chat it opened must not pull the phone
/// back to itself; the announced chat stays the target.
#[test]
fn a_bystander_ping_leaves_the_target_alone() {
    let mut core = RoutingCore::<String>::default();
    core.command_state_mut().select(Some(pane(1)));
    core.record_sent("announce".into(), pane(1));
    core.record_bystander_sent("report".into(), pane(9));
    assert_eq!(injected_into(&mut core), Some(pane(1)));
    // Still answerable by replying to it.
    let replied = core.route_text("ok".into(), Some("report".into())).ready();
    assert!(matches!(
        replied,
        InboundAction::InjectPrompt {
            pane: PaneRef { pane: 9, .. },
            ..
        }
    ));
}

/// With nothing chosen and only a bystander heard from, plain text still has
/// somewhere to go rather than none.
#[test]
fn a_bystander_is_the_last_resort() {
    let mut core = RoutingCore::<String>::default();
    core.record_bystander_sent("report".into(), pane(9));
    assert_eq!(injected_into(&mut core), Some(pane(9)));
}

#[test]
fn only_the_orchestrators_ping_is_a_bystander() {
    let mut core = RoutingCore::<String>::default();
    let orchestrator = Some(pane(9));
    core.record_delivered("worker".into(), pane(1), orchestrator);
    core.record_delivered("report".into(), pane(9), orchestrator);
    assert_eq!(injected_into(&mut core), Some(pane(1)));
    core.record_delivered("worker2".into(), pane(2), orchestrator);
    assert_eq!(injected_into(&mut core), Some(pane(2)));
}

/// A pane that spoke and then closed hands plain text back to the pick, not
/// to itself through `last_pinged` — which would fail the same way forever.
#[test]
fn a_gone_pane_that_spoke_hands_plain_text_back_to_the_pick() {
    use crate::remote_channel::command::Absorbed;
    let mut core = RoutingCore::<String>::default();
    core.command_state_mut().select(Some(pane(2)));
    core.record_sent("m1".into(), pane(3));
    assert_eq!(core.forget(pane(3)), Absorbed::SelectionRestored);
    assert_eq!(injected_into(&mut core), Some(pane(2)));
    assert_eq!(core.command_state_mut().selected(), Some(pane(2)));
}

/// With no pick to fall back to, nothing is the target — not the gone pane.
#[test]
fn a_gone_pane_with_no_pick_leaves_plain_text_unaimed() {
    use crate::remote_channel::command::Absorbed;
    let mut core = RoutingCore::<String>::default();
    core.record_sent("m1".into(), pane(3));
    assert_eq!(core.forget(pane(3)), Absorbed::SelectionDropped);
    assert!(matches!(
        core.route_text("carry on".into(), None),
        Routed::NeedsTarget(_)
    ));
}

/// A gone pane the plain text was not going to changes nothing worth saying.
#[test]
fn forgetting_a_pane_plain_text_was_not_aimed_at_moves_nothing() {
    use crate::remote_channel::command::Absorbed;
    let mut core = RoutingCore::<String>::default();
    core.record_sent("m1".into(), pane(3));
    core.record_sent("m2".into(), pane(4));
    assert_eq!(core.forget(pane(3)), Absorbed::Nothing);
    assert_eq!(injected_into(&mut core), Some(pane(4)));
}

#[test]
fn settlement_keeps_late_flow_routing_separate_from_live_tokens_and_bounded() {
    use crate::remote_channel::{bridge::FlowAskPrompt, cards::CardId};
    let mut core = RoutingCore::<i64>::default();
    let mut oldest = None;
    let mut latest = String::new();
    for ask_id in 0..=super::SETTLED_CALLBACKS_CAP as u64 {
        let message = core.build_flow_ask(FlowAskPrompt {
            worktree: "/tree".into(),
            run_dir: "/tree/run".into(),
            ask_id,
            summary: "Question".into(),
            buttons: vec![("Allow".into(), PermissionDecision::Allow("once".into()))],
        });
        latest = message.keyboard.unwrap().rows[0][0].1.clone();
        oldest.get_or_insert(latest.clone());
        core.settle_card(&CardId::FlowAsk {
            run_dir: "/tree/run".into(),
            ask_id,
        });
        assert!(core.pending_flow_asks.is_empty());
    }
    assert!(!core.is_settled_callback(&oldest.unwrap()));
    assert!(core.is_settled_callback(&latest));
    assert!(
        matches!(core.route_callback(latest.clone()), InboundAction::ResolveFlowAsk { ask_id, .. } if ask_id == super::SETTLED_CALLBACKS_CAP as u64)
    );
    core.clear_settled_callbacks();
    assert_eq!(core.route_callback(latest), InboundAction::Ignore);
}

fn question_ping(core: &mut RoutingCore<String>) -> (QuestionPromptRef, Vec<String>) {
    use super::super::QuestionPrompt;
    let question = QuestionPromptRef {
        session: 1,
        question_id: 4,
        field: 0,
    };
    let prepared = core.build_ping(BridgePing {
        pane: pane(1),
        header: "question".into(),
        tail: super::super::MessageTail::Plain("which?".into()),
        prompt: Some(PingPrompt::Question(QuestionPrompt {
            question,
            step: 9,
            rows: vec![
                vec![("A".into(), QuestionButton::Option(0))],
                vec![("Skip".into(), QuestionButton::Skip)],
            ],
        })),
    });
    let tokens = prepared
        .keyboard
        .unwrap()
        .rows
        .into_iter()
        .map(|row| row[0].1.clone())
        .collect();
    (question, tokens)
}

fn question_card(question: QuestionPromptRef) -> crate::remote_channel::cards::CardId {
    crate::remote_channel::cards::CardId::Question {
        pane: pane(1),
        question,
        step: 9,
    }
}

/// A question button names its field and is spent once tapped; the card's
/// settlement retires the rest, so neither can answer twice.
#[test]
fn a_question_button_answers_its_field_once() {
    let mut core = RoutingCore::<String>::default();
    let (question, tokens) = question_ping(&mut core);
    assert_eq!(
        core.route_callback(tokens[0].clone()),
        InboundAction::RespondQuestion {
            pane: pane(1),
            question,
            reply: QuestionReply::Button(QuestionButton::Option(0)),
        }
    );
    assert_eq!(
        core.route_callback(tokens[0].clone()),
        InboundAction::Ignore
    );
    core.settle_card(&question_card(question));
    assert_eq!(
        core.route_callback(tokens[1].clone()),
        InboundAction::Ignore
    );
}

/// Text replying to a live question card answers that field. Once the card
/// is settled or superseded by a later step, the reply is stale: refused,
/// never written into the field, and never sent as a prompt.
#[test]
fn a_reply_to_a_question_card_answers_it_while_the_card_is_live() {
    let mut core = RoutingCore::<String>::default();
    let (question, _) = question_ping(&mut core);
    core.record_sent("m1".into(), pane(1));
    core.record_question_sent("m1".into(), pane(1), question, 9);
    assert_eq!(
        core.route_text("teal".into(), Some("m1".into())).ready(),
        InboundAction::RespondQuestion {
            pane: pane(1),
            question,
            reply: QuestionReply::Text("teal".into()),
        }
    );
    // The next card for the same field supersedes this one.
    core.record_sent("m2".into(), pane(1));
    core.record_question_sent("m2".into(), pane(1), question, 10);
    core.settle_card(&question_card(question));
    assert_eq!(
        core.route_text("late".into(), Some("m1".into())).ready(),
        InboundAction::StaleQuestion
    );
    assert!(matches!(
        core.route_text("live".into(), Some("m2".into())).ready(),
        InboundAction::RespondQuestion { .. }
    ));
    assert!(matches!(
        core.route_text("next".into(), None).ready(),
        InboundAction::InjectPrompt { .. }
    ));
}
