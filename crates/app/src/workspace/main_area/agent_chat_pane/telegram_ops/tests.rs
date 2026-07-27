use futures::{FutureExt as _, StreamExt as _};
use gpui::AppContext as _;

use super::{permission_buttons, permission_wait_tail, preview_for};
use crate::surface::strings as s;
use crate::telegram::bridge::PermissionDecision;
use daruda_acp::{PermissionChoice, PermissionKindView};
use daruda_store::project::PaneCwd;

#[test]
fn should_defer_only_when_enabled_and_active() {
    assert!(super::should_defer_relay(true, true, 60));
    assert!(!super::should_defer_relay(true, false, 60));
    assert!(!super::should_defer_relay(false, true, 60));
    assert!(!super::should_defer_relay(true, true, 0));
}

fn tool_call(title: &str) -> daruda_acp::ChatItem {
    use daruda_acp::{ToolCallItem, ToolKindView, ToolStatusView};
    daruda_acp::ChatItem::ToolCall(ToolCallItem {
        id: "tool-1".to_string(),
        title: title.to_string(),
        kind: ToolKindView::Edit,
        tool_name: None,
        status: ToolStatusView::InProgress,
        diffs: Vec::new(),
        output: Vec::new(),
        raw_input: None,
        parent_tool_id: None,
    })
}

fn assistant_text(text: &str, streaming: bool) -> daruda_acp::ChatItem {
    daruda_acp::ChatItem::AssistantText {
        text: text.to_string(),
        streaming,
        message_id: None,
    }
}

fn thinking(text: &str) -> daruda_acp::ChatItem {
    daruda_acp::ChatItem::Thinking {
        text: text.to_string(),
        streaming: false,
        message_id: None,
    }
}

#[test]
fn resolve_ignores_thinking_and_still_streaming_text() {
    let watch = super::FirstResponseWatch::start(std::time::Instant::now(), 0);
    let items = vec![thinking("pondering"), assistant_text("partial", true)];
    assert_eq!(watch.resolve(&items), None);
}

#[test]
fn resolve_returns_the_first_completed_assistant_text() {
    let watch = super::FirstResponseWatch::start(std::time::Instant::now(), 0);
    let items = vec![thinking("hmm"), assistant_text("done", false)];
    assert_eq!(
        watch.resolve(&items),
        Some(super::FirstResponseOutcome::Text("done".to_string()))
    );
}

#[test]
fn resolve_returns_a_tool_call_with_no_preceding_text() {
    let watch = super::FirstResponseWatch::start(std::time::Instant::now(), 0);
    let items = vec![thinking("hmm"), tool_call("Write /tmp/x.rs")];
    assert_eq!(
        watch.resolve(&items),
        Some(super::FirstResponseOutcome::Tool {
            tool_title: Some("Write /tmp/x.rs".to_string())
        })
    );
}

#[test]
fn resolve_only_scans_items_appended_after_the_watch_started() {
    // A prior turn's completed AssistantText, present *before* the watch's
    // anchor point, must not be mistaken for this turn's first response.
    let items = vec![assistant_text("previous turn's answer", false)];
    let watch = super::FirstResponseWatch::start(std::time::Instant::now(), items.len());
    assert_eq!(watch.resolve(&items), None);
}

#[test]
fn is_overdue_boundary_at_exactly_the_timeout() {
    let started = std::time::Instant::now();
    let watch = super::FirstResponseWatch::start(started, 0);
    assert!(!watch.is_overdue(started + std::time::Duration::from_secs(59), 60));
    assert!(watch.is_overdue(started + std::time::Duration::from_secs(60), 60));
}

/// `queued_at` no longer participates in the push-time decision; a fresh
/// timestamp is enough for every test entry here.
fn mk_relay(kind: super::DeferKind) -> super::DeferredRelay {
    super::DeferredRelay {
        kind,
        header: String::new(),
        tail: super::TelegramTail::Plain(String::new()),
        permission: None,
        queued_at: std::time::Instant::now(),
    }
}

#[test]
fn push_deferred_keeps_one_completion_but_accumulates_others() {
    use super::DeferKind;
    let mut q = Vec::new();
    super::push_deferred(&mut q, mk_relay(DeferKind::Completion));
    super::push_deferred(&mut q, mk_relay(DeferKind::PostTurn));
    super::push_deferred(&mut q, mk_relay(DeferKind::Completion));
    assert_eq!(q.len(), 2);
    assert_eq!(q[0].kind, DeferKind::PostTurn);
    assert_eq!(q[1].kind, DeferKind::Completion);
}

#[test]
fn push_deferred_evicts_oldest_beyond_cap() {
    use super::DeferKind;
    let mut q = Vec::new();
    let extra = 5;
    for i in 0..(super::MAX_DEFERRED_PER_PANE + extra) as u64 {
        super::push_deferred(&mut q, mk_relay(DeferKind::Permission { perm_id: i }));
    }
    assert_eq!(q.len(), super::MAX_DEFERRED_PER_PANE);
    // The oldest entries (ids 0..extra) were evicted; the newest survive.
    assert_eq!(
        q[0].kind,
        DeferKind::Permission {
            perm_id: extra as u64
        }
    );
    assert_eq!(
        q.last().unwrap().kind,
        DeferKind::Permission {
            perm_id: (super::MAX_DEFERRED_PER_PANE + extra - 1) as u64
        }
    );
}

#[test]
fn partition_deferred_drops_stale_permission() {
    use super::DeferKind;
    let now = std::time::Instant::now();
    let q = vec![
        mk_relay(DeferKind::Completion),
        mk_relay(DeferKind::Permission { perm_id: 7 }),
    ];
    // Only id 9 is outstanding, so the ping for the resolved id 7 is dropped
    // outright — not delivered, not held for later.
    let live: std::collections::HashSet<u64> = [9].into_iter().collect();
    // `app_active: false` makes every non-stale entry immediately ready,
    // isolating the staleness filter from the readiness check below.
    let (ready, still_holding) = super::partition_deferred(q, &live, now, false, 0.0, 60);
    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].kind, DeferKind::Completion);
    assert!(still_holding.is_empty());
}

#[test]
fn partition_deferred_keeps_each_outstanding_permission() {
    use super::DeferKind;
    let now = std::time::Instant::now();
    let q = vec![
        mk_relay(DeferKind::Permission { perm_id: 7 }),
        mk_relay(DeferKind::Permission { perm_id: 8 }),
        mk_relay(DeferKind::Permission { perm_id: 9 }),
    ];
    // 7 and 9 are still outstanding; 8 was answered → its ping is dropped,
    // the other two both survive (a single live id could not express this).
    let live: std::collections::HashSet<u64> = [7, 9].into_iter().collect();
    let (ready, still_holding) = super::partition_deferred(q, &live, now, false, 0.0, 60);
    assert_eq!(ready.len(), 2);
    assert_eq!(ready[0].kind, DeferKind::Permission { perm_id: 7 });
    assert_eq!(ready[1].kind, DeferKind::Permission { perm_id: 9 });
    assert!(still_holding.is_empty());
}

#[test]
fn partition_deferred_holds_back_entries_not_yet_ready() {
    // Still foregrounded, just queued (age 0), currently idle — not ready:
    // the quiet window anchors to this entry's own `queued_at`, not to the
    // (already-past-threshold) idle reading alone.
    use super::DeferKind;
    let now = std::time::Instant::now();
    let q = vec![mk_relay(DeferKind::Completion)];
    let live = std::collections::HashSet::new();
    let (ready, still_holding) = super::partition_deferred(q, &live, now, true, 90.0, 60);
    assert!(ready.is_empty());
    assert_eq!(still_holding.len(), 1);
}

#[test]
fn ready_to_deliver_fires_immediately_once_app_is_not_active() {
    let now = std::time::Instant::now();
    // Just queued, and idle_secs is 0 (input this instant) — would fail
    // every other condition, but leaving the app delivers regardless.
    assert!(super::ready_to_deliver(now, now, false, 0.0, 60));
}

#[test]
fn ready_to_deliver_requires_both_elapsed_and_idle_while_foregrounded() {
    let queued_at = std::time::Instant::now();
    let past_window = queued_at + std::time::Duration::from_secs(61);

    // Enough real time has passed AND the user has been idle that whole
    // time → ready.
    assert!(super::ready_to_deliver(
        queued_at,
        past_window,
        true,
        90.0,
        60
    ));
    let already_quiet = past_window - std::time::Duration::from_secs(61);
    assert!(super::ready_to_deliver(
        already_quiet,
        past_window,
        true,
        60.0,
        60
    ));
    // Enough real time has passed, but `idle_secs` shows recent input
    // (the user came back and used the keyboard) → still held.
    assert!(!super::ready_to_deliver(
        queued_at,
        past_window,
        true,
        5.0,
        60
    ));
    // Not enough real time has passed yet, even though `idle_secs` alone
    // would clear the threshold — the ping's own quiet window hasn't
    // elapsed, so a long turn's leftover idle streak can't fire it early.
    assert!(!super::ready_to_deliver(
        queued_at,
        queued_at + std::time::Duration::from_secs(5),
        true,
        90.0,
        60
    ));
}

#[test]
fn permission_wait_tail_appends_the_tool_title_when_present() {
    assert_eq!(
        permission_wait_tail(Some("Write /tmp/x.rs"), None),
        format!("{}\n{}", s::agent_notification_waiting(), "Write /tmp/x.rs")
    );
}

#[test]
fn permission_wait_tail_appends_the_raw_input_summary_after_the_title() {
    assert_eq!(
        permission_wait_tail(Some("Run npm install"), Some("command: npm install")),
        format!(
            "{}\n{}\n{}",
            s::agent_notification_waiting(),
            "Run npm install",
            "command: npm install"
        )
    );
}

#[test]
fn permission_wait_tail_appends_only_the_raw_input_summary_without_a_title() {
    assert_eq!(
        permission_wait_tail(None, Some("file: /tmp/x.rs")),
        format!("{}\n{}", s::agent_notification_waiting(), "file: /tmp/x.rs")
    );
}

#[test]
fn permission_wait_tail_falls_back_to_plain_waiting_text_without_a_title_or_summary() {
    assert_eq!(
        permission_wait_tail(None, None),
        s::agent_notification_waiting()
    );
}

#[test]
fn permission_wait_tail_treats_empty_strings_as_absent() {
    assert_eq!(
        permission_wait_tail(Some(""), Some("")),
        s::agent_notification_waiting()
    );
}

#[test]
fn first_tool_ack_tail_appends_the_title_when_present() {
    assert_eq!(
        super::first_tool_ack_tail(Some("Write /tmp/x.rs")),
        format!(
            "{}\n{}",
            s::agent_notification_telegram_first_tool_ack(),
            "Write /tmp/x.rs"
        )
    );
}

#[test]
fn first_tool_ack_tail_treats_an_empty_title_as_absent() {
    assert_eq!(
        super::first_tool_ack_tail(Some("")),
        s::agent_notification_telegram_first_tool_ack()
    );
}

#[test]
fn first_tool_ack_tail_without_a_title() {
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
fn preview_for_under_threshold_is_verbatim() {
    let text = "a".repeat(2000);
    assert_eq!(preview_for(&text, "…(marker)…"), text);
}

#[test]
fn preview_for_over_threshold_keeps_head_and_tail_around_marker() {
    // 1000 'a's + 1000 'b's = 2001 chars, one over the threshold.
    let text = format!("{}{}", "a".repeat(1000), "b".repeat(1001));
    let result = preview_for(&text, "…(marker)…");
    assert_eq!(
        result,
        format!("{}\n…(marker)…\n{}", "a".repeat(1000), "b".repeat(1000))
    );
}

#[test]
fn preview_for_is_char_safe_with_multibyte_text() {
    // Korean text well past the threshold — must not panic on a
    // byte-boundary split, and must actually keep 1000 *characters*
    // (not bytes) on each side.
    let text = "가".repeat(2500);
    let result = preview_for(&text, "…(중략)…");
    let expected = format!("{}\n…(중략)…\n{}", "가".repeat(1000), "가".repeat(1000));
    assert_eq!(result, expected);
}

#[test]
fn permission_buttons_exposes_every_option_not_just_first_of_kind() {
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
}

#[test]
fn permission_buttons_maps_reject_always_to_reject_decision() {
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
}

#[test]
fn permission_buttons_empty_options_is_empty() {
    assert!(permission_buttons(&[]).is_empty());
}

/// A phone reply injected into a pane that has never connected (no
/// `handle`, so `send_prompt_text_for_telegram` always queues) sends the
/// "queued" notice — there's nothing yet to watch a first response for.
#[gpui::test]
async fn inject_bot_reply_sends_the_queued_notice_when_it_has_to_wait(
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
            !view.read(cx).is_waiting_for_telegram_first_response(),
            "queuing alone must not arm the watch"
        );
    });

    let sent = outbound
        .next()
        .await
        .expect("the queued notice should be sent");
    assert_eq!(
        sent.tail,
        super::TelegramTail::Plain(s::agent_notification_telegram_reply_queued())
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

/// `deliver_deferred_telegram` must tolerate a queued entry whose pane has
/// since closed, filter stale permission pings through the live pane's
/// `pending_permissions` outstanding-ids set, and still deliver the
/// remaining live-pane entry.
#[gpui::test]
async fn deliver_deferred_telegram_skips_closed_pane_filters_stale_permission_and_sends_live_entry(
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
    let live_pane = cx
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

    // A pane id well past anything the test ever allocated — stands in
    // for "the pane closed after the ping was queued".
    const CLOSED_PANE: super::PaneId = u64::MAX;

    let mk = |kind, tail: &str| super::DeferredRelay {
        kind,
        header: "header".to_string(),
        tail: super::TelegramTail::Plain(tail.to_string()),
        permission: None,
        queued_at: std::time::Instant::now(),
    };

    workspace.update(cx, |ws, _| {
        ws.deferred_telegram
            .entry(CLOSED_PANE)
            .or_default()
            .push(mk(super::DeferKind::Completion, "closed"));
        ws.deferred_telegram
            .entry(live_pane)
            .or_default()
            .push(mk(super::DeferKind::Completion, "completion"));
        // Live pane, but a permission id that is not in the pane's
        // `pending_permissions` set (empty — no permission was ever
        // requested on this pane) — exercises the `partition_deferred`
        // staleness filter path on a real view.
        ws.deferred_telegram
            .entry(live_pane)
            .or_default()
            .push(mk(super::DeferKind::Permission { perm_id: 42 }, "stale"));
    });

    // `app_active: false` mirrors "presence already dropped" — every
    // non-stale entry is immediately ready regardless of age/idle.
    workspace.update(cx, |ws, cx| {
        ws.deliver_deferred_telegram(false, 999.0, 60, cx);
    });
    cx.run_until_parked();

    workspace.read_with(cx, |ws, _| {
        assert!(
            ws.deferred_telegram.is_empty(),
            "the queue drains after delivery"
        );
    });

    let sent = outbound
        .next()
        .await
        .expect("the live completion entry should be sent");
    assert_eq!(sent.pane.pane, live_pane);
    assert_eq!(sent.header, "header");
    assert_eq!(
        sent.tail,
        super::TelegramTail::Plain("completion".to_string())
    );
    assert!(sent.permission.is_none());
    assert!(
        outbound.next().now_or_never().is_none(),
        "closed panes and stale permissions must not emit extra pings"
    );
}

/// A ping that hasn't cleared its own quiet window yet stays queued, then
/// drains once the same foregrounded/idle pane has cleared that window.
#[gpui::test]
async fn deliver_deferred_telegram_holds_until_entry_quiet_window_clears(
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
                id
            })
        })
        .unwrap();
    cx.run_until_parked();

    workspace.update(cx, |ws, _| {
        ws.deferred_telegram
            .entry(pane)
            .or_default()
            .push(super::DeferredRelay {
                kind: super::DeferKind::Completion,
                header: "header".to_string(),
                tail: super::TelegramTail::Plain("completion".to_string()),
                permission: None,
                queued_at: std::time::Instant::now(),
            });
    });

    // Foregrounded, and `idle_secs` alone would already clear the
    // threshold — but the entry was just queued, so its own window
    // hasn't elapsed yet.
    workspace.update(cx, |ws, cx| {
        ws.deliver_deferred_telegram(true, 90.0, 60, cx);
    });
    cx.run_until_parked();

    workspace.read_with(cx, |ws, _| {
        assert_eq!(
            ws.deferred_telegram.get(&pane).map(Vec::len),
            Some(1),
            "the entry stays queued instead of firing early"
        );
    });
    assert!(
        outbound.next().now_or_never().is_none(),
        "nothing should have been sent yet"
    );

    workspace.update(cx, |ws, _| {
        ws.deferred_telegram
            .get_mut(&pane)
            .and_then(|q| q.get_mut(0))
            .expect("held entry should still be queued")
            .queued_at = std::time::Instant::now() - std::time::Duration::from_secs(61);
    });

    workspace.update(cx, |ws, cx| {
        ws.deliver_deferred_telegram(true, 60.0, 60, cx);
    });
    cx.run_until_parked();

    workspace.read_with(cx, |ws, _| {
        assert!(
            ws.deferred_telegram.is_empty(),
            "the entry drains once its foreground quiet window clears"
        );
    });
    let sent = outbound
        .next()
        .await
        .expect("the quiet-window-cleared entry should be sent");
    assert_eq!(sent.pane.pane, pane);
    assert_eq!(sent.header, "header");
    assert_eq!(
        sent.tail,
        super::TelegramTail::Plain("completion".to_string())
    );
}

/// Turning off active-window deferral should release already-held pings on
/// the next flush instead of leaving them governed by the old foreground
/// quiet-window rules.
#[gpui::test]
async fn flush_deferred_telegram_drains_existing_queue_when_defer_disabled(
    cx: &mut gpui::TestAppContext,
) {
    let mut outbound =
        cx.update(|cx| crate::telegram::global::install_for_test(true, Some(42), cx));
    let mut config = daruda_config::Config::default();
    config.telegram.enabled = true;
    config.telegram.authorized_chat_id = Some(42);
    config.telegram.defer_while_active = false;
    let (handle, workspace) = make_window(cx, &config);
    cx.run_until_parked();

    let tmp = std::env::temp_dir();
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
                id
            })
        })
        .unwrap();
    cx.run_until_parked();

    workspace.update(cx, |ws, _| {
        ws.deferred_telegram
            .entry(pane)
            .or_default()
            .push(super::DeferredRelay {
                kind: super::DeferKind::Completion,
                header: "header".to_string(),
                tail: super::TelegramTail::Plain("completion".to_string()),
                permission: None,
                queued_at: std::time::Instant::now(),
            });
    });

    workspace.update(cx, |ws, cx| {
        ws.flush_deferred_telegram(cx);
    });
    cx.run_until_parked();

    workspace.read_with(cx, |ws, _| {
        assert!(
            ws.deferred_telegram.is_empty(),
            "defer-disabled flush should drain the held queue"
        );
    });

    let sent = outbound
        .next()
        .await
        .expect("the held completion entry should be sent");
    assert_eq!(sent.pane.pane, pane);
    assert_eq!(sent.header, "header");
    assert_eq!(
        sent.tail,
        super::TelegramTail::Plain("completion".to_string())
    );
    assert!(outbound.next().now_or_never().is_none());
}

#[gpui::test]
async fn flush_telegram_first_response_fallbacks_sends_once_for_overdue_watch(
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
                    let started = std::time::Instant::now()
                        - std::time::Duration::from_secs(super::FIRST_RESPONSE_FALLBACK_SECS + 1);
                    v.start_telegram_first_response_watch_for_test(started);
                });
                id
            })
        })
        .unwrap();
    cx.run_until_parked();

    workspace.update(cx, |ws, cx| {
        ws.flush_telegram_first_response_fallbacks(cx);
    });
    cx.run_until_parked();

    workspace.read_with(cx, |ws, cx| {
        let view = ws.agent_chat_view(pane).cloned().unwrap();
        assert!(
            !view.read(cx).is_waiting_for_telegram_first_response(),
            "the overdue fallback consumes the watch"
        );
    });

    let sent = outbound
        .next()
        .await
        .expect("the fallback ack should be sent");
    assert_eq!(sent.pane.pane, pane);
    assert_eq!(
        sent.tail,
        super::TelegramTail::Plain(s::agent_notification_telegram_reply_ack())
    );

    workspace.update(cx, |ws, cx| {
        ws.flush_telegram_first_response_fallbacks(cx);
    });
    cx.run_until_parked();
    assert!(outbound.next().now_or_never().is_none());
}

#[gpui::test]
async fn first_response_permission_without_buttons_sends_fallback_ack(
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
                    v.start_telegram_first_response_watch_for_test(std::time::Instant::now());
                });
                id
            })
        })
        .unwrap();
    cx.run_until_parked();

    workspace.update(cx, |ws, cx| {
        ws.relay_permission_wait_to_telegram(pane, 7, &[], Some("Tool without choices"), None, cx);
    });
    cx.run_until_parked();

    let sent = outbound
        .next()
        .await
        .expect("empty-button first response should still ack the phone");
    assert_eq!(sent.pane.pane, pane);
    assert_eq!(
        sent.tail,
        super::TelegramTail::Plain(s::agent_notification_telegram_reply_ack())
    );
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
