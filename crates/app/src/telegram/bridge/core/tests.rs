//! `BridgeCore` routing tests — the state machine driven by plain
//! function calls, with no network and no GPUI.

use super::*;
// Shapes only the tests build; `core` itself never names them.
use crate::telegram::bridge::{PermissionPromptRef, TelegramTail};
use daruda_store::project::WorkspaceUuid;

fn pane(workspace_seed: u128, id: u64) -> PaneRef {
    PaneRef {
        workspace: WorkspaceUuid(Uuid::from_u128(workspace_seed)),
        pane: id,
    }
}

fn message(update_id: i64, chat_id: i64, text: &str, reply_to: Option<i64>) -> Update {
    Update {
        update_id,
        kind: UpdateKind::Message {
            chat_id,
            text: text.to_string(),
            reply_to_message_id: reply_to,
        },
    }
}

fn callback(update_id: i64, chat_id: i64, callback_id: &str, data: &str) -> Update {
    Update {
        update_id,
        kind: UpdateKind::Callback {
            chat_id,
            callback_id: callback_id.to_string(),
            data: data.to_string(),
            message_id: update_id,
            message_text: "prompt".to_string(),
        },
    }
}

#[test]
fn commands_are_ignored_before_pairing() {
    let mut core = BridgeCore::new(true, None, 0);
    let action = core.route(message(1, 999, "/list", None)).action.ready();
    assert_eq!(
        action,
        InboundAction::Ignore,
        "unauthorized chats must not enumerate"
    );
}

#[test]
fn an_authorized_command_routes_to_run_command() {
    let mut core = BridgeCore::new(true, Some(42), 0);
    let action = core.route(message(1, 42, "/list", None)).action.ready();
    assert_eq!(
        action,
        InboundAction::RunCommand {
            command: crate::control::spec::ControlCommand::List
        }
    );
}

/// A typo is never swallowed. The answer is settled a layer up now — no
/// agent advertises `lst`, so the suggestion is daruda's to give — but the
/// suggestion has to reach that layer intact for it to be given at all.
#[test]
fn a_typo_carries_its_suggestion_instead_of_vanishing() {
    let mut core = BridgeCore::new(true, Some(42), 0);
    let action = core.route(message(1, 42, "/lst", None)).action;
    assert_eq!(
        action,
        Routed::NeedsTarget(Unaimed::Slash {
            name: "lst".into(),
            text: "/lst".into(),
            suggestion: Some("list"),
        })
    );
}

/// The regression this fixes. Claude owns `/usage`, daruda owns `/use`,
/// and they share the `/` namespace — so daruda answered a command it does
/// not have with a suggestion for one the user did not want, and the
/// agent never saw it. Every agent slash command was unreachable from the
/// phone this way; `/usage` is the one that has a near-miss to make it
/// look deliberate.
#[test]
fn a_slash_we_do_not_own_is_held_for_the_pane_to_claim() {
    let mut core = BridgeCore::new(true, Some(42), 0);
    let target = pane(1, 10);
    core.command_state_mut().select(Some(target));
    let action = core.route(message(1, 42, "/usage", None)).action.ready();
    assert_eq!(
        action,
        InboundAction::UnknownSlash {
            pane: target,
            name: "usage".into(),
            text: "/usage".into(),
            suggestion: Some("use"),
        },
        "an agent command must reach the agent, not a typo answer"
    );
}

/// The same slash with nothing to aim it at. daruda still cannot say the
/// name is a typo of one of its own — it never could — so the question is
/// handed on rather than answered with a suggestion for a command the
/// sender did not want.
#[test]
fn an_unowned_slash_with_no_target_is_still_not_ours_to_refuse() {
    let mut core = BridgeCore::new(true, Some(42), 0);
    let action = core.route(message(1, 42, "/usage", None)).action;
    assert_eq!(
        action,
        Routed::NeedsTarget(Unaimed::Slash {
            name: "usage".into(),
            text: "/usage".into(),
            suggestion: Some("use"),
        }),
        "with no target the reason is the missing target, not a typo"
    );
}

/// A command we *do* own, used wrongly, stays ours — the agent has no
/// `/say` and forwarding it would answer a daruda mistake with an agent's
/// confusion.
#[test]
fn our_own_command_used_wrongly_is_still_ours_to_answer() {
    let mut core = BridgeCore::new(true, Some(42), 0);
    core.command_state_mut().select(Some(pane(1, 10)));
    let action = core.route(message(1, 42, "/say", None)).action.ready();
    assert_eq!(
        action,
        InboundAction::ReportParseError {
            error: crate::control::spec::ParseError::MissingArgument { command: "say" }
        }
    );
}

#[test]
fn plain_text_with_no_target_reports_instead_of_vanishing() {
    let mut core = BridgeCore::new(true, Some(42), 0);
    let action = core.route(message(1, 42, "hello", None)).action;
    assert_eq!(
        action,
        Routed::NeedsTarget(Unaimed::Text {
            text: "hello".into()
        })
    );
}

#[test]
fn a_selected_target_takes_plain_text_without_a_reply_to() {
    let mut core = BridgeCore::new(true, Some(42), 0);
    let target = pane(1, 10);
    core.command_state_mut().select(Some(target));
    let action = core
        .route(message(1, 42, "add tests too", None))
        .action
        .ready();
    assert_eq!(
        action,
        InboundAction::InjectPrompt {
            pane: target,
            text: "add tests too".to_string(),
        }
    );
}

#[test]
fn tapping_a_list_button_selects_that_target() {
    use crate::control::spec::Ordinal;

    let mut core = BridgeCore::new(true, Some(42), 0);
    let a = pane(1, 10);
    core.command_state_mut()
        .record_listing(&crate::telegram::command::tests::listing_of(&[a]));
    let token = core
        .command_state_mut()
        .listing_token(Ordinal(1))
        .expect("token");
    let action = core.route(callback(1, 42, "cb", &token)).action.ready();
    assert_eq!(action, InboundAction::SelectTarget { pane: a });
}

/// The invariant the parser now upholds and `route` depends on: the offset
/// moves past an update the bridge cannot act on, so it is not re-delivered
/// forever.
#[test]
fn an_unsupported_update_still_advances_the_offset() {
    let mut core = BridgeCore::new(true, Some(42), 0);
    let result = core.route(Update {
        update_id: 9,
        kind: UpdateKind::Unsupported,
    });
    assert_eq!(result.action.ready(), InboundAction::Unsupported);
    assert_eq!(result.answer_callback_id, None, "nothing to acknowledge");
    assert_eq!(core.current_offset(), 10);
}

/// Even while disabled: the poll loop does not run then, but a disable
/// that lands mid-batch must not strand the offset either.
#[test]
fn an_unsupported_update_advances_the_offset_while_disabled() {
    let mut core = BridgeCore::new(false, Some(42), 0);
    core.route(Update {
        update_id: 3,
        kind: UpdateKind::Unsupported,
    });
    assert_eq!(core.current_offset(), 4);
}

#[test]
fn a_seeded_offset_skips_already_processed_updates() {
    let mut core = BridgeCore::new(true, Some(42), 100);
    assert_eq!(core.current_offset(), 100);
    core.route(message(99, 42, "/list", None));
    assert_eq!(
        core.current_offset(),
        100,
        "an old update must not rewind the offset"
    );
}

#[test]
fn a_fresh_core_starts_at_zero() {
    let core = BridgeCore::new(true, None, 0);
    assert_eq!(core.current_offset(), 0);
}

/// A permission prompt is always one horizontal row of options, so the
/// assertions below read that row directly — and fail loudly if the
/// prompt ever grows a second one.
fn only_row(keyboard: &InlineKeyboard) -> &[(String, String)] {
    assert_eq!(keyboard.rows.len(), 1, "a permission prompt is one row");
    &keyboard.rows[0]
}

/// A fresh (not-yet-expired, zero-attempts) pending pair code, for
/// tests that seed `pending_pair_code` directly rather than going
/// through `new_pair_code()`.
fn fresh_pending_code(code: &str) -> PendingPairCode {
    PendingPairCode {
        code: code.to_string(),
        generated_at: std::time::Instant::now(),
        attempts: 0,
    }
}

#[test]
fn unauthorized_message_with_no_pending_pair_is_ignored() {
    let mut bridge = BridgeCore::new(true, None, 0);
    let result = bridge.route(message(1, 999, "hello", None));
    assert_eq!(result.action.ready(), InboundAction::Ignore);
    assert_eq!(result.answer_callback_id, None);
}

#[test]
fn pair_code_exact_match_pairs_and_authorizes_future_messages() {
    let mut bridge = BridgeCore::new(true, None, 0);
    let code = bridge.new_pair_code();

    let result = bridge.route(message(1, 555, &format!("/pair {code}"), None));
    assert_eq!(
        result.action.ready(),
        InboundAction::Paired { chat_id: 555 }
    );
    assert_eq!(bridge.authorized_chat_id, Some(555));

    // A follow-up message from the now-authorized chat routes as
    // authorized (falls through to Ignore here only because there's
    // no reply-to / last-pinged pane yet — proves the auth gate
    // passed).
    bridge.last_pinged = Some(pane(1, 1));
    let result = bridge.route(message(2, 555, "ping back", None));
    assert_eq!(
        result.action.ready(),
        InboundAction::InjectPrompt {
            pane: pane(1, 1),
            text: "ping back".to_string(),
        }
    );
}

#[test]
fn pair_code_match_is_case_insensitive() {
    let mut bridge = BridgeCore::new(true, None, 0);
    bridge.pending_pair_code = Some(fresh_pending_code("AB12CD"));

    let result = bridge.route(message(1, 42, "/pair ab12cd", None));
    assert_eq!(result.action.ready(), InboundAction::Paired { chat_id: 42 });
}

#[test]
fn pair_code_mismatch_is_ignored_and_stays_unauthorized() {
    let mut bridge = BridgeCore::new(true, None, 0);
    bridge.pending_pair_code = Some(fresh_pending_code("AB12CD"));

    let result = bridge.route(message(1, 42, "/pair WRONG1", None));
    assert_eq!(result.action.ready(), InboundAction::Ignore);
    assert_eq!(bridge.authorized_chat_id, None);
}

#[test]
fn reply_to_found_in_sent_pings_wins_over_last_pinged() {
    let mut bridge = BridgeCore::new(true, Some(1), 0);
    let reply_target = pane(1, 10);
    let different_last = pane(1, 99);

    bridge.record_sent(41, reply_target);
    bridge.record_sent(50, different_last);
    assert_eq!(bridge.last_pinged, Some(different_last));

    let result = bridge.route(message(1, 1, "answer", Some(41)));
    assert_eq!(
        result.action.ready(),
        InboundAction::InjectPrompt {
            pane: reply_target,
            text: "answer".to_string(),
        }
    );
}

#[test]
fn reply_to_missing_from_sent_pings_falls_back_to_last_pinged() {
    let mut bridge = BridgeCore::new(true, Some(1), 0);
    let fallback = pane(1, 7);
    bridge.record_sent(1, fallback);

    let result = bridge.route(message(2, 1, "answer", Some(999)));
    assert_eq!(
        result.action.ready(),
        InboundAction::InjectPrompt {
            pane: fallback,
            text: "answer".to_string(),
        }
    );
}

#[test]
fn no_reply_to_and_no_last_pinged_reports_no_target() {
    let mut bridge = BridgeCore::new(true, Some(1), 0);
    let result = bridge.route(message(1, 1, "hello", None));
    // Not `Ignore`: an authorized message that reaches nothing is answered,
    // because silence reads as the bot being broken.
    assert_eq!(
        result.action,
        Routed::NeedsTarget(Unaimed::Text {
            text: "hello".into()
        })
    );
}

/// The card stays on screen while the tool call waits, so tapping the
/// same button twice is ordinary use — the token must survive it. (The
/// *decision* is deduplicated by the approval store, not here.)
#[test]
fn an_approval_token_survives_repeated_taps() {
    use crate::control::approval::{ApprovalChoice, ApprovalId};
    let mut bridge = BridgeCore::new(true, Some(42), 0);
    let (approve, refuse) = bridge.record_pending_approval(ApprovalId(1));
    assert_ne!(approve, refuse, "one token per button");

    for _ in 0..2 {
        assert_eq!(
            bridge
                .route(callback(1, 42, "cbq-1", &approve))
                .action
                .ready(),
            InboundAction::ResolveApproval {
                id: ApprovalId(1),
                choice: ApprovalChoice::Approved,
            }
        );
    }
    assert_eq!(
        bridge
            .route(callback(2, 42, "cbq-2", &refuse))
            .action
            .ready(),
        InboundAction::ResolveApproval {
            id: ApprovalId(1),
            choice: ApprovalChoice::Refused,
        }
    );
}

#[test]
fn an_approval_tap_from_an_unauthorized_chat_is_ignored() {
    use crate::control::approval::ApprovalId;
    let mut bridge = BridgeCore::new(true, Some(42), 0);
    let (approve, _refuse) = bridge.record_pending_approval(ApprovalId(1));
    assert_eq!(
        bridge
            .route(callback(1, 99, "cbq-1", &approve))
            .action
            .ready(),
        InboundAction::Ignore
    );
}

/// A card that outlived the process leaves a button in the chat history;
/// its token must not resolve against a request this run happens to
/// number the same.
#[test]
fn an_approval_token_from_another_process_does_not_resolve() {
    use crate::control::approval::ApprovalId;
    let mut before = BridgeCore::new(true, Some(42), 0);
    let (stale, _) = before.record_pending_approval(ApprovalId(1));

    let mut after = BridgeCore::new(true, Some(42), 0);
    let _ = after.record_pending_approval(ApprovalId(1));
    assert_eq!(
        after.route(callback(1, 42, "cbq-1", &stale)).action.ready(),
        InboundAction::Ignore
    );
}

/// Three token namespaces share one callback channel, so a token from one
/// must never be read as another's.
#[test]
fn the_three_token_namespaces_do_not_collide() {
    use crate::control::approval::ApprovalId;
    let mut bridge = BridgeCore::new(true, Some(42), 0);
    let target = pane(1, 3);
    bridge.pending_permissions.insert(
        "abcdef0123456789".to_string(),
        (target, 55, PermissionDecision::Allow("opt_yes".to_string())),
    );
    let (approve, _) = bridge.record_pending_approval(ApprovalId(1));

    assert!(approve.starts_with(APPROVAL_TOKEN_PREFIX));
    assert!(!approve.starts_with(crate::telegram::command::LISTING_TOKEN_PREFIX));
    // A permission token is a bare `Uuid::simple`, so it carries neither
    // prefix and still reaches the permission table.
    assert!(matches!(
        bridge
            .route(callback(1, 42, "cbq-1", "abcdef0123456789"))
            .action
            .ready(),
        InboundAction::RespondPermission { .. }
    ));
}

#[test]
fn callback_with_known_token_responds_and_consumes_it() {
    let mut bridge = BridgeCore::new(true, Some(1), 0);
    let target = pane(1, 3);
    bridge.pending_permissions.insert(
        "tok-a".to_string(),
        (target, 55, PermissionDecision::Allow("opt_yes".to_string())),
    );

    let result = bridge.route(callback(1, 1, "cbq-1", "tok-a"));
    assert_eq!(
        result.action.ready(),
        InboundAction::RespondPermission {
            pane: target,
            perm_id: 55,
            decision: PermissionDecision::Allow("opt_yes".to_string()),
        }
    );
    assert_eq!(result.answer_callback_id, Some("cbq-1".to_string()));

    // Second tap on the same (now-consumed) token is a no-op.
    let result = bridge.route(callback(2, 1, "cbq-2", "tok-a"));
    assert_eq!(result.action.ready(), InboundAction::Ignore);
    assert_eq!(result.answer_callback_id, Some("cbq-2".to_string()));
}

#[test]
fn callback_with_unknown_token_is_ignored_but_still_acked() {
    let mut bridge = BridgeCore::new(true, Some(1), 0);
    let result = bridge.route(callback(1, 1, "cbq-x", "never-registered"));
    assert_eq!(result.action.ready(), InboundAction::Ignore);
    assert_eq!(result.answer_callback_id, Some("cbq-x".to_string()));
}

#[test]
fn callback_from_unauthorized_chat_is_ignored_but_still_acked() {
    let mut bridge = BridgeCore::new(true, Some(1), 0);
    let result = bridge.route(callback(1, 2, "cbq-y", "irrelevant"));
    assert_eq!(result.action.ready(), InboundAction::Ignore);
    assert_eq!(result.answer_callback_id, Some("cbq-y".to_string()));
}

#[test]
fn disabled_bridge_ignores_everything_but_still_acks_callbacks() {
    let mut bridge = BridgeCore::new(false, Some(1), 0);

    let msg_result = bridge.route(message(1, 1, "hello", None));
    assert_eq!(msg_result.action.ready(), InboundAction::Ignore);
    assert_eq!(msg_result.answer_callback_id, None);

    let cb_result = bridge.route(callback(2, 1, "cbq-z", "tok"));
    assert_eq!(cb_result.action.ready(), InboundAction::Ignore);
    assert_eq!(cb_result.answer_callback_id, Some("cbq-z".to_string()));
}

#[test]
fn offset_advances_to_max_update_id_plus_one_across_ignored_updates() {
    let mut bridge = BridgeCore::new(true, None, 0);
    assert_eq!(bridge.current_offset(), 0);

    bridge.route(message(5, 999, "unauthorized", None));
    assert_eq!(bridge.current_offset(), 6);

    // Lower update_id than what's already been seen must not move
    // the offset backwards.
    bridge.route(message(3, 999, "unauthorized again", None));
    assert_eq!(bridge.current_offset(), 6);

    bridge.route(callback(10, 999, "cbq", "unknown-token"));
    assert_eq!(bridge.current_offset(), 11);
}

#[test]
fn sent_pings_bound_evicts_oldest_entries() {
    let mut bridge = BridgeCore::new(true, Some(1), 0);
    let extra = 5;

    for i in 0..(SENT_PINGS_CAP + extra) as i64 {
        bridge.record_sent(i, pane(1, i as u64));
    }

    assert_eq!(bridge.sent_pings.len(), SENT_PINGS_CAP);
    assert_eq!(bridge.sent_pings_order.len(), SENT_PINGS_CAP);

    // The oldest message_ids (0..extra) were evicted; a reply-to
    // against one of them now falls back to last_pinged instead of
    // resolving directly.
    let evicted_id = 0i64;
    assert!(!bridge.sent_pings.contains_key(&evicted_id));
    let result = bridge.route(message(1000, 1, "late reply", Some(evicted_id)));
    assert_eq!(
        result.action.ready(),
        InboundAction::InjectPrompt {
            pane: pane(1, (SENT_PINGS_CAP + extra - 1) as u64),
            text: "late reply".to_string(),
        }
    );

    // A still-present (recent) message_id still resolves directly.
    let surviving_id = (SENT_PINGS_CAP + extra - 1) as i64;
    assert!(bridge.sent_pings.contains_key(&surviving_id));
}

#[test]
fn build_ping_plain_completion_has_no_keyboard() {
    let mut bridge = BridgeCore::new(true, Some(1), 0);
    let msg = bridge.build_ping(BridgePing {
        pane: pane(1, 1),
        header: "Turn finished".to_string(),
        tail: TelegramTail::Plain(String::new()),
        permission: None,
    });

    assert_eq!(msg.chat_id, 1);
    assert_eq!(msg.header, "Turn finished");
    assert!(msg.keyboard.is_none());
    assert!(bridge.pending_permissions.is_empty());
}

#[test]
fn build_ping_permission_registers_two_distinct_tokens() {
    let mut bridge = BridgeCore::new(true, Some(1), 0);
    let msg = bridge.build_ping(BridgePing {
        pane: pane(1, 1),
        header: "Approve this?".to_string(),
        tail: TelegramTail::Plain(String::new()),
        permission: Some(PermissionPromptRef {
            perm_id: 7,
            buttons: vec![
                (
                    "Allow".to_string(),
                    PermissionDecision::Allow("opt_allow".to_string()),
                ),
                (
                    "Reject".to_string(),
                    PermissionDecision::Reject("opt_reject".to_string()),
                ),
            ],
        }),
    });

    let keyboard = msg.keyboard.expect("keyboard present");
    assert_eq!(only_row(&keyboard).len(), 2);
    assert_eq!(only_row(&keyboard)[0].0, "Allow");
    assert_eq!(only_row(&keyboard)[1].0, "Reject");
    assert_ne!(only_row(&keyboard)[0].1, only_row(&keyboard)[1].1);

    assert_eq!(bridge.pending_permissions.len(), 2);
    let allow_token = &only_row(&keyboard)[0].1;
    let reject_token = &only_row(&keyboard)[1].1;
    assert_eq!(
        bridge.pending_permissions.get(allow_token),
        Some(&(
            pane(1, 1),
            7,
            PermissionDecision::Allow("opt_allow".to_string())
        ))
    );
    assert_eq!(
        bridge.pending_permissions.get(reject_token),
        Some(&(
            pane(1, 1),
            7,
            PermissionDecision::Reject("opt_reject".to_string())
        ))
    );
}

#[test]
fn build_ping_permission_registers_one_token_per_button_beyond_two() {
    // The real motivating case: codex-acp can offer more than one
    // Allow-shaped option (Allow Once / Allow for Session / an
    // execpolicy-amendment allow) alongside Reject. Every option the
    // agent offers must become its own button + token, not collapse
    // to a single Allow/Reject pair.
    let mut bridge = BridgeCore::new(true, Some(1), 0);
    let msg = bridge.build_ping(BridgePing {
        pane: pane(1, 1),
        header: "Approve this?".to_string(),
        tail: TelegramTail::Plain(String::new()),
        permission: Some(PermissionPromptRef {
            perm_id: 9,
            buttons: vec![
                (
                    "Allow Once".to_string(),
                    PermissionDecision::Allow("allow_once".to_string()),
                ),
                (
                    "Allow for Session".to_string(),
                    PermissionDecision::Allow("allow_always".to_string()),
                ),
                (
                    "Allow Commands Starting With …".to_string(),
                    PermissionDecision::Allow("accept_execpolicy_amendment".to_string()),
                ),
                (
                    "Reject".to_string(),
                    PermissionDecision::Reject("reject_once".to_string()),
                ),
            ],
        }),
    });

    let keyboard = msg.keyboard.expect("keyboard present");
    assert_eq!(only_row(&keyboard).len(), 4);
    let labels: Vec<&str> = only_row(&keyboard)
        .iter()
        .map(|(l, _)| l.as_str())
        .collect();
    assert_eq!(
        labels,
        vec![
            "Allow Once",
            "Allow for Session",
            "Allow Commands Starting With …",
            "Reject",
        ]
    );

    let tokens: std::collections::HashSet<&String> =
        only_row(&keyboard).iter().map(|(_, t)| t).collect();
    assert_eq!(tokens.len(), 4, "every button gets its own distinct token");
    assert_eq!(bridge.pending_permissions.len(), 4);

    let execpolicy_token = &only_row(&keyboard)[2].1;
    assert_eq!(
        bridge.pending_permissions.get(execpolicy_token),
        Some(&(
            pane(1, 1),
            9,
            PermissionDecision::Allow("accept_execpolicy_amendment".to_string())
        ))
    );
}

#[test]
fn pending_permissions_bound_evicts_oldest_entries() {
    let mut bridge = BridgeCore::new(true, Some(1), 0);
    let prompt = || PermissionPromptRef {
        perm_id: 1,
        buttons: vec![
            (
                "Allow".to_string(),
                PermissionDecision::Allow("opt_allow".to_string()),
            ),
            (
                "Reject".to_string(),
                PermissionDecision::Reject("opt_reject".to_string()),
            ),
        ],
    };

    let first_msg = bridge.build_ping(BridgePing {
        pane: pane(1, 1),
        header: "first".to_string(),
        tail: TelegramTail::Plain(String::new()),
        permission: Some(prompt()),
    });
    let first_keyboard = first_msg.keyboard.expect("keyboard present");
    let first_allow_token = only_row(&first_keyboard)[0].1.clone();
    let first_reject_token = only_row(&first_keyboard)[1].1.clone();

    // Each call below registers 2 more tokens; enough calls to push
    // well past the cap (mirrors `sent_pings_bound_evicts_oldest_entries`).
    for _ in 0..PENDING_PERMISSIONS_CAP {
        bridge.build_ping(BridgePing {
            pane: pane(1, 1),
            header: "more".to_string(),
            tail: TelegramTail::Plain(String::new()),
            permission: Some(prompt()),
        });
    }

    assert_eq!(bridge.pending_permissions.len(), PENDING_PERMISSIONS_CAP);
    assert_eq!(
        bridge.pending_permissions_order.len(),
        PENDING_PERMISSIONS_CAP
    );

    // The oldest tokens (from the very first ping) were evicted —
    // a callback tap against one of them is unresolvable, same as
    // any other unknown/consumed token.
    assert!(!bridge.pending_permissions.contains_key(&first_allow_token));
    assert!(!bridge.pending_permissions.contains_key(&first_reject_token));

    let result = bridge.route(callback(1, 1, "cbq-evicted", &first_allow_token));
    assert_eq!(result.action.ready(), InboundAction::Ignore);
    assert_eq!(result.answer_callback_id, Some("cbq-evicted".to_string()));
}

#[test]
fn pair_code_expired_not_yet_expired() {
    let now = std::time::Instant::now();
    let generated_at = now - std::time::Duration::from_secs(1);
    assert!(!pair_code_expired(generated_at, now, PAIR_CODE_TTL));
}

#[test]
fn pair_code_expired_at_exact_boundary() {
    let now = std::time::Instant::now();
    let generated_at = now - PAIR_CODE_TTL;
    assert!(pair_code_expired(generated_at, now, PAIR_CODE_TTL));
}

#[test]
fn pair_code_expired_well_past_ttl() {
    let now = std::time::Instant::now();
    let generated_at = now - (PAIR_CODE_TTL * 2);
    assert!(pair_code_expired(generated_at, now, PAIR_CODE_TTL));
}

#[test]
fn pair_code_still_works_within_attempt_and_ttl_budget() {
    // Regression check: a couple of wrong guesses (well under
    // `PAIR_CODE_MAX_ATTEMPTS`) must not invalidate the code — the
    // correct code still pairs afterward.
    let mut bridge = BridgeCore::new(true, None, 0);
    let code = bridge.new_pair_code();

    let result = bridge.route(message(1, 42, "/pair WRONG1", None));
    assert_eq!(result.action.ready(), InboundAction::Ignore);
    let result = bridge.route(message(2, 42, "/pair WRONG2", None));
    assert_eq!(result.action.ready(), InboundAction::Ignore);

    let result = bridge.route(message(3, 42, &format!("/pair {code}"), None));
    assert_eq!(result.action.ready(), InboundAction::Paired { chat_id: 42 });
    assert_eq!(bridge.authorized_chat_id, Some(42));
}

#[test]
fn pair_code_exhausted_by_max_attempts_rejects_even_correct_guess() {
    let mut bridge = BridgeCore::new(true, None, 0);
    let code = bridge.new_pair_code();

    for i in 0..PAIR_CODE_MAX_ATTEMPTS {
        let result = bridge.route(message(i as i64, 42, "/pair WRONGCODE", None));
        assert_eq!(result.action.ready(), InboundAction::Ignore);
        assert_eq!(bridge.authorized_chat_id, None);
    }

    // The code is now exhausted (attempts == PAIR_CODE_MAX_ATTEMPTS) —
    // even the CORRECT code is rejected, proving the throttle locks
    // out the code rather than just the wrong guesses.
    let result = bridge.route(message(
        PAIR_CODE_MAX_ATTEMPTS as i64,
        42,
        &format!("/pair {code}"),
        None,
    ));
    assert_eq!(result.action.ready(), InboundAction::Ignore);
    assert_eq!(bridge.authorized_chat_id, None);
    assert!(bridge.pending_pair_code.is_none());
}
