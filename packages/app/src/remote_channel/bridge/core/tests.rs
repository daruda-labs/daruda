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
