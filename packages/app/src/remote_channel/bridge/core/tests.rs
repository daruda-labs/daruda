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
        permission: Some(super::super::PermissionPromptRef {
            perm_id: 7,
            buttons: vec![("Allow".into(), PermissionDecision::Allow("once".into()))],
        }),
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
