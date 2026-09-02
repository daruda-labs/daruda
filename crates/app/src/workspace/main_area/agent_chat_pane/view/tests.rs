use super::super::fold::{FoldContext, FoldKey};
use super::super::rows::RowKind;
use crate::transcript::fold_mode::FoldPreset;

fn assistant_text_item(text: &str) -> daruda_acp::ChatItem {
    daruda_acp::ChatItem::AssistantText {
        text: text.to_string(),
        streaming: false,
        message_id: None,
        phase: Default::default(),
    }
}

fn tool_call(
    id: &str,
    status: daruda_acp::ToolStatusView,
    parent_tool_id: Option<&str>,
) -> daruda_acp::ChatItem {
    daruda_acp::ChatItem::ToolCall(daruda_acp::ToolCallItem {
        id: id.to_string(),
        title: format!("Tool {id}"),
        kind: daruda_acp::ToolKindView::Read,
        tool_name: None,
        status,
        diffs: Vec::new(),
        output: Vec::new(),
        raw_input: None,
        parent_tool_id: parent_tool_id.map(str::to_string),
        exit: None,
    })
}

fn permission_card(id: u64) -> daruda_acp::ChatItem {
    daruda_acp::ChatItem::Permission(daruda_acp::PermissionItem {
        id,
        tool_title: Some(format!("Write /tmp/{id}")),
        raw_input_summary: None,
        options: vec![
            daruda_acp::PermissionChoice {
                option_id: "allow_once".to_string(),
                name: "Allow".to_string(),
                kind: daruda_acp::PermissionKindView::AllowOnce,
            },
            daruda_acp::PermissionChoice {
                option_id: "reject_once".to_string(),
                name: "Reject".to_string(),
                kind: daruda_acp::PermissionKindView::RejectOnce,
            },
        ],
        resolved: None,
    })
}

/// Build a minimal, offline `AgentChatView` (no cwd → `Idle` status, no
/// adapter spawned) as its own window root, so the post-turn marker
/// methods (`&mut self` + `Instant`/`Duration`, no `Workspace`) can be
/// driven directly. Lighter than `workspace/tests/agent_chat.rs`'s
/// `make_activity_view` (which goes through a full `Workspace` +
/// `create_agent_chat_pane`): `AgentChatView::new` only needs a
/// `Context<Self>`, not a `Workspace` at all.
pub(in crate::workspace::main_area::agent_chat_pane) fn make_test_view(
    cx: &mut gpui::TestAppContext,
) -> gpui::WindowHandle<super::AgentChatView> {
    crate::test_support::init_gpui_component(cx);
    cx.add_window(|window, cx| {
        super::AgentChatView::new(
            0,
            window.window_handle(),
            None,
            super::AgentSessionStatus::Idle,
            None,
            None,
            "claude".to_string(),
            "Claude".to_string(),
            None,
            super::super::transcript_defaults::TranscriptDefaults {
                tail: super::super::rows::tail::TailWindow::All,
                fold_mode: crate::transcript::fold_mode::FoldMode::default(),
                filter: crate::transcript::display_filter::DisplayFilter::default(),
            },
            daruda_config::file_viewer::DEFAULT_SYNTAX_THEME.to_owned(),
            cx,
        )
    })
}

#[gpui::test]
fn host_is_dark_tracks_agent_chat_background_not_ui_theme(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        let light_ui = crate::ui::theme::DarudaTheme {
            title_bar_bg: gpui::Hsla {
                h: 0.0,
                s: 0.0,
                l: 0.95,
                a: 1.0,
            },
            ..Default::default()
        };
        cx.set_global(light_ui);

        crate::ui::theme::set_agent_chat_bg(cx, 0, 0, 0);
        assert!(
            super::AgentChatView::host_is_dark(cx),
            "dark terminal-backed chat background should drive dark Mermaid keys"
        );

        crate::ui::theme::set_agent_chat_bg(cx, 255, 255, 255);
        assert!(
            !super::AgentChatView::host_is_dark(cx),
            "light terminal-backed chat background should drive light Mermaid keys"
        );
    });
}

/// The activity-bar title is derived from `session_title` + the first user
/// prompt, neither of which changes per frame — yet resolving it in `render`
/// made it the paint path's top cost on a pane whose first message was long.
/// It belongs with `rows` / `live_units`: recomputed once per model change, at
/// the `rebuild_rows` funnel every `items` / title mutation already ends in.
#[gpui::test]
fn activity_title_is_derived_once_per_model_change(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, _cx| {
            assert_eq!(view.activity_title(), None, "empty session has no title");

            view.items.push(daruda_acp::ChatItem::UserText(
                "  fix the   parser  ".into(),
            ));
            view.rebuild_rows();
            assert_eq!(
                view.activity_title(),
                Some("fix the parser"),
                "the first prompt stands in until the agent names the session"
            );

            // An agent-supplied title supersedes the fallback at the same funnel.
            view.session_title = Some("Refactor fold state".into());
            view.rebuild_rows();
            assert_eq!(view.activity_title(), Some("Refactor fold state"));

            // Clearing the conversation clears the derived title with it.
            view.items.clear();
            view.session_title = None;
            view.rebuild_rows();
            assert_eq!(view.activity_title(), None);
        })
        .expect("the test window is live");
}

/// Appending a follow-up prompt must not make the previously visible agent
/// response collapse out from under the user. The model still keeps every
/// item either way; this pins the row projection so the prior response's
/// process prose remains visible after the new `UserText` anchor is added.
#[gpui::test]
fn echo_prompt_preserves_visible_tail_response(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.items.push(daruda_acp::ChatItem::UserText("q1".into()));
            view.items.push(assistant_text_item("process"));
            view.items.push(assistant_text_item("final"));
            view.rebuild_rows();

            assert!(
                view.rows
                    .iter()
                    .any(|r| matches!(r.kind, RowKind::AgentItem(1)) && !r.hidden),
                "tail response starts expanded"
            );

            view.echo_prompt("q2".to_string(), cx);

            assert!(
                view.rows
                    .iter()
                    .any(|r| matches!(r.kind, RowKind::AgentItem(1)) && !r.hidden),
                "submitting the next prompt must not hide prior agent prose"
            );
            assert!(
                view.rows
                    .iter()
                    .any(|r| matches!(r.kind, RowKind::ConclusionItem(2)) && !r.hidden),
                "the prior conclusion remains visible too"
            );
        })
        .unwrap();
}

/// The preservation hook only freezes a response that is currently visible;
/// a user collapse remains authoritative across the next prompt.
#[gpui::test]
fn echo_prompt_respects_collapsed_tail_response(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, live_window, cx| {
            view.items.push(daruda_acp::ChatItem::UserText("q1".into()));
            view.items.push(assistant_text_item("process"));
            view.items.push(assistant_text_item("final"));
            view.toggle_fold(FoldKey::Response(1), live_window, cx);

            view.echo_prompt("q2".to_string(), cx);

            assert!(
                view.rows
                    .iter()
                    .any(|r| matches!(r.kind, RowKind::AgentItem(1)) && r.hidden),
                "an explicitly collapsed response stays collapsed after the next prompt"
            );
            assert!(
                view.rows
                    .iter()
                    .any(|r| matches!(r.kind, RowKind::ConclusionItem(2)) && !r.hidden),
                "the conclusion still surfaces from the collapsed response"
            );
        })
        .unwrap();
}

/// The preservation hook must not accumulate: after several prompts, picking a
/// Mode still governs every response. A machine-written *override* would outrank
/// the mode forever and leave the whole transcript pinned open.
#[gpui::test]
fn switching_to_summary_collapses_the_responses_earlier_sends_held_open(
    cx: &mut gpui::TestAppContext,
) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, window, cx| {
            view.items.push(daruda_acp::ChatItem::UserText("q1".into()));
            for turn in 0..3 {
                view.items
                    .push(assistant_text_item(&format!("process {turn}")));
                view.items
                    .push(assistant_text_item(&format!("final {turn}")));
                view.echo_prompt(format!("q{}", turn + 2), cx);
            }
            view.items.push(assistant_text_item("process last"));
            view.items.push(assistant_text_item("final last"));
            view.rebuild_rows();

            view.set_fold_mode(FoldPreset::Summary.mode(), window, cx);

            let open: Vec<usize> = view
                .rows
                .iter()
                .filter(|r| !r.hidden)
                .filter_map(|r| match r.kind {
                    RowKind::AgentItem(ix) => Some(ix),
                    _ => None,
                })
                .collect();
            assert!(
                open.is_empty(),
                "Summary collapses every response's process prose; still open: {open:?}"
            );
        })
        .unwrap();
}

/// A response held open by a send is released by the next send — the hold
/// tracks the newest response only, so history keeps folding itself away.
#[gpui::test]
fn the_next_send_releases_the_response_the_previous_one_held(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            // 0: q1, 1: process, 2: final, 3: q2, 4: process, 5: final, 6: q3
            view.items.push(daruda_acp::ChatItem::UserText("q1".into()));
            view.items.push(assistant_text_item("process 1"));
            view.items.push(assistant_text_item("final 1"));
            view.echo_prompt("q2".to_string(), cx);
            assert!(
                view.rows
                    .iter()
                    .any(|r| matches!(r.kind, RowKind::AgentItem(1)) && !r.hidden),
                "the first send holds the response the user was reading"
            );

            view.items.push(assistant_text_item("process 2"));
            view.items.push(assistant_text_item("final 2"));
            view.echo_prompt("q3".to_string(), cx);
            assert!(
                view.rows
                    .iter()
                    .any(|r| matches!(r.kind, RowKind::AgentItem(1)) && r.hidden),
                "the older response folds once it is no longer the one being read"
            );
            assert!(
                view.rows
                    .iter()
                    .any(|r| matches!(r.kind, RowKind::AgentItem(4)) && !r.hidden),
                "the newly held response stays open"
            );
        })
        .unwrap();
}

fn queued(id: u64, text: &str) -> super::QueuedPrompt {
    super::QueuedPrompt {
        id: super::PromptId(id),
        text: text.to_string(),
        origin: super::PromptOrigin::InApp,
    }
}

fn texts(prompts: &[super::QueuedPrompt]) -> Vec<String> {
    prompts.iter().map(|q| q.text.clone()).collect()
}

/// Stop must preserve queued work and completion bookkeeping across the local
/// cancel paths.
#[gpui::test]
fn cancel_turn_parks_queue_preserves_completion_and_buffers_reprompt(
    cx: &mut gpui::TestAppContext,
) {
    use daruda_acp::{ChatItem, ToolCallItem, ToolKindView, ToolStatusView};

    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            let reset = |view: &mut super::AgentChatView| {
                view.queue = Default::default();
                view.items.clear();
                view.activity = Default::default();
            };
            let running_child = || {
                ChatItem::ToolCall(ToolCallItem {
                    id: "child".into(),
                    title: "child".into(),
                    kind: ToolKindView::Read,
                    tool_name: None,
                    status: ToolStatusView::InProgress,
                    diffs: Vec::new(),
                    output: Vec::new(),
                    raw_input: None,
                    parent_tool_id: Some("parent".into()),
                    exit: None,
                })
            };

            view.set_turn_in_flight();
            view.queue.pending_prompts.push(queued(1, "a"));
            view.queue.pending_prompts.push(queued(2, "b"));

            view.cancel_turn(cx);

            assert!(view.turn_is_idle(), "Stop settles the turn locally");
            assert!(
                view.queue.pending_prompts.is_empty(),
                "the live queue is emptied by the Stop"
            );
            assert_eq!(
                texts(&view.queue.paused_prompts),
                vec!["a".to_string(), "b".to_string()],
                "Stop parks the queue (FIFO) instead of dropping it"
            );

            reset(view);
            view.set_turn_idle();
            view.items = vec![running_child()];
            view.activity.pending_completion = Some(super::TurnOutcome::Completed);
            assert!(
                view.is_busy(),
                "a running child subagent keeps the pane busy"
            );

            view.cancel_turn(cx);
            assert_eq!(
                view.activity.pending_completion,
                Some(super::TurnOutcome::Completed),
                "Stop with no foreground turn keeps the already-captured completion"
            );

            reset(view);
            view.set_turn_in_flight();
            view.cancel_turn(cx);
            assert!(view.turn_is_idle(), "Stop settles the live turn locally");
            assert_eq!(
                view.activity.pending_completion,
                Some(super::TurnOutcome::Stopped),
                "Stop of a live turn stashes Stopped"
            );

            reset(view);
            view.set_turn_in_flight();
            view.cancel_turn(cx);
            assert!(
                view.activity.cancel_in_flight,
                "the cancel window stays open until the ack"
            );

            view.send_prompt_text("again".into(), cx);
            assert!(
                view.turn_is_idle(),
                "the re-prompt is buffered, not raced onto the wire, during cancel"
            );
            assert_eq!(
                texts(&view.queue.pending_prompts),
                vec!["again".to_string()],
                "the re-prompt buffers client-side"
            );

            view.cancel_turn(cx);
            assert!(
                view.queue.pending_prompts.is_empty(),
                "the second Stop empties the live queue"
            );
            assert_eq!(
                texts(&view.queue.paused_prompts),
                vec!["again".to_string()],
                "the re-prompt is parked, not dropped"
            );

            view.apply_event(
                daruda_acp::AcpEvent::TurnEnded {
                    completed_normally: false,
                    stop_reason: "Cancelled".into(),
                },
                "",
                false,
                cx,
            );
            assert!(
                !view.activity.cancel_in_flight,
                "the ack closes the cancel window"
            );
            assert!(view.turn_is_idle(), "nothing left to run after both Stops");
        })
        .unwrap();
}

/// The tooltip says "last active", so the timestamp has to advance whenever
/// this pane goes quiet. The adapter sends `updatedAt` only alongside a
/// *changed* title, so relying on it alone would freeze the value.
#[gpui::test]
fn activity_settle_stamps_last_active(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, _cx| {
            assert_eq!(
                view.session_updated_at, None,
                "a fresh pane has no activity"
            );

            view.set_turn_in_flight();
            view.reconcile_activity(std::time::Instant::now());
            assert_eq!(
                view.session_updated_at, None,
                "still working -- the span has not ended yet"
            );

            view.set_turn_idle();
            view.reconcile_activity(std::time::Instant::now());
            let stamped = view
                .session_updated_at
                .clone()
                .expect("the busy->idle settle stamps last-active");
            // The field feeds `format_last_active`, which parses RFC 3339.
            chrono::DateTime::parse_from_rfc3339(&stamped)
                .unwrap_or_else(|e| panic!("stamped {stamped:?} is not RFC 3339: {e}"));
        })
        .unwrap();
}

/// Activity reconciliation turns `is_busy()` into idle->busy / busy->idle
/// edges, tracks one busy span across contiguous turns, and fires captured
/// completion exactly once when the pane settles.
#[gpui::test]
fn reconcile_activity_edges_and_completion_delivery(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, _cx| {
            let reset = |view: &mut super::AgentChatView| {
                view.set_turn_idle();
                view.items.clear();
                view.activity = super::ActivityTracker::default();
            };

            assert!(view.activity_elapsed().is_none(), "idle -> no span");
            view.set_turn_in_flight();
            let edge = view.reconcile_activity(std::time::Instant::now());
            assert_eq!(edge, None, "the busy edge fires no completion");
            assert!(view.activity.was_busy, "reconcile records the busy level");
            assert!(
                view.activity_elapsed().is_some(),
                "the span start is stamped on idle->busy"
            );

            reset(view);
            view.set_turn_in_flight();
            assert_eq!(view.reconcile_activity(std::time::Instant::now()), None);
            view.activity.pending_completion = Some(super::TurnOutcome::Completed);
            view.set_turn_idle();
            let edge = view.reconcile_activity(std::time::Instant::now());
            assert_eq!(
                edge,
                Some(super::TurnOutcome::Completed),
                "the busy->idle edge returns the stashed outcome"
            );
            assert!(
                view.activity_elapsed().is_none(),
                "the span start clears on settle"
            );
            assert_eq!(
                view.reconcile_activity(std::time::Instant::now()),
                None,
                "the outcome fires exactly once"
            );

            reset(view);
            view.set_turn_in_flight();
            assert_eq!(view.reconcile_activity(std::time::Instant::now()), None);
            view.set_turn_idle();
            assert_eq!(
                view.reconcile_activity(std::time::Instant::now()),
                None,
                "a settle with no pending outcome fires nothing"
            );

            reset(view);
            view.set_turn_in_flight();
            assert_eq!(view.reconcile_activity(std::time::Instant::now()), None);
            view.activity.pending_completion = Some(super::TurnOutcome::Stopped);
            view.set_turn_in_flight();
            assert_eq!(
                view.reconcile_activity(std::time::Instant::now()),
                None,
                "still busy across the turn boundary -> no settle edge"
            );
            view.activity.pending_completion = Some(super::TurnOutcome::Completed);
            view.set_turn_idle();
            assert_eq!(
                view.reconcile_activity(std::time::Instant::now()),
                Some(super::TurnOutcome::Completed),
                "the span fires the latest outcome exactly once at the final settle"
            );

            reset(view);
            view.set_turn_in_flight();
            assert_eq!(view.reconcile_activity(std::time::Instant::now()), None);
            assert!(
                view.activity.was_busy,
                "the connect-time reconcile stamps the busy level"
            );
            view.activity.pending_completion = Some(super::TurnOutcome::Completed);
            view.set_turn_idle();
            assert_eq!(
                view.reconcile_activity(std::time::Instant::now()),
                Some(super::TurnOutcome::Completed),
                "connect-time pump completion fires at settle instead of stranding"
            );
        })
        .unwrap();
}

/// Resuming a parked queue moves the parked prompts back to the FRONT of the
/// live queue (they were submitted before anything queued after the Stop).
/// Offline (no handle) the pump can't dispatch, so they simply land in the
/// live queue in order — the reorder is what this pins.
#[gpui::test]
fn resume_queue_moves_parked_to_front_of_live(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.queue.paused_prompts.push(queued(1, "p1"));
            view.queue.paused_prompts.push(queued(2, "p2"));
            // A prompt queued (behind a since-cancelled turn) after the Stop.
            view.queue.pending_prompts.push(queued(3, "live"));

            view.resume_queue(cx);

            assert!(
                view.queue.paused_prompts.is_empty(),
                "resume drains the parked queue"
            );
            assert_eq!(
                texts(&view.queue.pending_prompts),
                vec!["p1".to_string(), "p2".to_string(), "live".to_string()],
                "parked prompts resume ahead of the live queue (FIFO)"
            );
        })
        .unwrap();
}

/// The per-item remove (×) and clear-all reach parked prompts too — the
/// strip renders those affordances on parked rows, so they must act on
/// `paused_prompts`, not just the live queue.
#[gpui::test]
fn remove_and_clear_reach_parked_prompts(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.queue.paused_prompts.push(queued(1, "a"));
            view.queue.paused_prompts.push(queued(2, "b"));
            view.queue.pending_prompts.push(queued(3, "c"));

            view.remove_queued(super::PromptId(1), cx);
            assert_eq!(
                texts(&view.queue.paused_prompts),
                vec!["b".to_string()],
                "removing a parked prompt drops it from the parked queue"
            );
            assert_eq!(
                texts(&view.queue.pending_prompts),
                vec!["c".to_string()],
                "the live queue is untouched by removing a parked prompt"
            );

            view.clear_queue(cx);
            assert!(
                view.queue.paused_prompts.is_empty() && view.queue.pending_prompts.is_empty(),
                "clear-all empties both the parked and live queues"
            );
        })
        .unwrap();
}

/// Editing a queued prompt replaces its slot in place, whether it is still live
/// or parked. If the edited row has already left the queue, sending falls
/// through to a brand-new queued prompt so the typed text is not lost.
#[gpui::test]
fn editing_prompt_replaces_live_or_parked_and_falls_through_when_stale(
    cx: &mut gpui::TestAppContext,
) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.send_prompt_text("first".into(), cx);
            view.send_prompt_text("second".into(), cx);
            view.send_prompt_text("third".into(), cx);
            let middle = view.queue.pending_prompts[1].id;
            view.begin_edit(middle, cx);
            assert_eq!(
                view.queue.editing_prompt,
                Some(middle),
                "begin_edit records the target"
            );

            view.send_prompt_text("edited".into(), cx);
            assert_eq!(
                texts(&view.queue.pending_prompts),
                vec![
                    "first".to_string(),
                    "edited".to_string(),
                    "third".to_string()
                ],
                "editing replaces the live slot in place, order preserved"
            );
            assert!(
                view.queue.editing_prompt.is_none(),
                "send clears the editing flag"
            );
            assert!(view.items.is_empty(), "an in-place edit does not echo");

            view.queue = Default::default();
            view.items.clear();
            view.queue.paused_prompts.push(queued(1, "old"));
            view.begin_edit(super::PromptId(1), cx);

            view.send_prompt_text("new".to_string(), cx);

            assert_eq!(
                texts(&view.queue.paused_prompts),
                vec!["new".to_string()],
                "the parked prompt's text is replaced in place"
            );
            assert!(
                view.queue.pending_prompts.is_empty(),
                "editing does not enqueue a new prompt"
            );
            assert_eq!(view.queue.editing_prompt, None, "the edit flag is cleared");

            view.queue = Default::default();
            view.items.clear();
            view.send_prompt_text("only".into(), cx);
            let id = view.queue.pending_prompts[0].id;
            view.begin_edit(id, cx);
            // Model the target leaving the queue while the composer still targets it.
            view.remove_queued(id, cx);
            assert_eq!(
                view.queue.editing_prompt,
                Some(id),
                "removing a row does not by itself clear the editing flag"
            );
            assert!(view.queue.pending_prompts.is_empty());

            view.send_prompt_text("typed".into(), cx);
            assert_eq!(
                texts(&view.queue.pending_prompts),
                vec!["typed".to_string()],
                "a drained edit target falls through to a new queued prompt"
            );
            assert!(
                view.queue.editing_prompt.is_none(),
                "the stale editing flag is cleared on send"
            );
            assert!(
                view.items.is_empty(),
                "no handle means the new prompt is queued, not echoed"
            );
        })
        .unwrap();
}

#[gpui::test]
fn permission_resolution_routes_by_id_reprojects_and_cancel_drains(cx: &mut gpui::TestAppContext) {
    use daruda_acp::{ChatItem, PermissionKindView, PermissionResolution, ToolStatusView};

    let window = make_test_view(cx);
    window
        .update(cx, |view, live_window, cx| {
            view.items = vec![permission_card(100), permission_card(200)];
            view.pending_permissions.insert(100);
            view.pending_permissions.insert(200);

            view.respond_permission(
                100,
                "allow_once".to_string(),
                PermissionKindView::AllowOnce,
                cx,
            );

            let ChatItem::Permission(first) = &view.items[0] else {
                panic!("expected first permission card");
            };
            let ChatItem::Permission(second) = &view.items[1] else {
                panic!("expected second permission card");
            };
            assert_eq!(
                first.resolved,
                Some(PermissionResolution::Chosen("allow_once".to_string())),
                "the answered card resolves by id"
            );
            assert_eq!(
                second.resolved, None,
                "answering one permission leaves the other live"
            );
            assert!(!view.is_permission_outstanding(100));
            assert!(view.is_permission_outstanding(200));

            view.respond_permission(
                200,
                "reject_once".to_string(),
                PermissionKindView::RejectOnce,
                cx,
            );
            let ChatItem::Permission(second) = &view.items[1] else {
                panic!("expected second permission card");
            };
            assert_eq!(
                second.resolved,
                Some(PermissionResolution::Chosen("reject_once".to_string()))
            );
            assert!(!view.has_pending_permission());

            view.items = vec![
                ChatItem::UserText("q".into()),
                tool_call("a", ToolStatusView::Completed, None),
                tool_call("b", ToolStatusView::Completed, None),
                permission_card(7),
            ];
            view.pending_permissions.insert(7);
            view.set_all_folds(false, live_window, cx);
            assert!(
                view.rows
                    .iter()
                    .any(|r| matches!(r.kind, RowKind::AgentItem(3)) && !r.hidden),
                "a pending permission stays visible under a collapsed response"
            );

            view.respond_permission(
                7,
                "allow_once".to_string(),
                PermissionKindView::AllowOnce,
                cx,
            );
            assert!(
                view.rows
                    .iter()
                    .any(|r| matches!(r.kind, RowKind::AgentItem(3)) && r.hidden),
                "a resolved permission folds back into the collapsed response immediately"
            );

            view.items = vec![
                permission_card(10),
                permission_card(20),
                permission_card(30),
            ];
            view.pending_permissions.extend([10, 20, 30]);
            view.set_turn_in_flight();
            view.cancel_turn(cx);

            for item in &view.items {
                let ChatItem::Permission(card) = item else {
                    panic!("expected a permission card");
                };
                assert_eq!(
                    card.resolved,
                    Some(PermissionResolution::Cancelled),
                    "Stop cancels every outstanding permission"
                );
            }
            assert!(!view.has_pending_permission());
        })
        .unwrap();
}

#[gpui::test]
fn activity_state_maps_permission_turn_and_background_subagent(cx: &mut gpui::TestAppContext) {
    use daruda_acp::ToolStatusView;

    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, _cx| {
            view.status = super::AgentSessionStatus::Connected;

            view.set_turn_in_flight();
            view.items.clear();
            assert_eq!(view.activity_state(), super::ActivityState::Working);
            assert_eq!(
                view.to_session_status(),
                Some(daruda_agent::SessionStatus::Working)
            );

            view.set_turn_idle();
            view.items = vec![tool_call(
                "child",
                ToolStatusView::InProgress,
                Some("parent"),
            )];
            assert_eq!(
                view.activity_state(),
                super::ActivityState::Working,
                "a running background child tool keeps the pane Working after end_turn"
            );
            assert_eq!(
                view.to_session_status(),
                Some(daruda_agent::SessionStatus::Working)
            );

            view.set_turn_idle();
            view.items.clear();
            view.pending_permissions.insert(7);
            assert_eq!(
                view.activity_state(),
                super::ActivityState::AwaitingPermission
            );
            assert_eq!(
                view.to_session_status(),
                Some(daruda_agent::SessionStatus::NeedsAttention)
            );

            view.set_turn_idle();
            view.pending_permissions.clear();
            view.items = vec![
                tool_call("child", ToolStatusView::Completed, Some("parent")),
                tool_call("top", ToolStatusView::InProgress, None),
            ];
            assert_eq!(
                view.activity_state(),
                super::ActivityState::Idle,
                "a completed child and a top-level running tool are not background activity"
            );
            assert_eq!(
                view.to_session_status(),
                Some(daruda_agent::SessionStatus::Idle)
            );
        })
        .unwrap();
}

#[gpui::test]
fn mode_state_updates_replace_and_survive_config_refresh(cx: &mut gpui::TestAppContext) {
    use daruda_acp::{
        AcpEvent, ConfigChoiceView, ConfigOptionCategoryView, ConfigOptionView, ModeStateView,
        SessionModeView,
    };

    let mode = |id: &str| SessionModeView {
        id: id.to_string(),
        name: id.to_uppercase(),
        description: None,
    };

    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.session_config.modes = Some(ModeStateView {
                available: vec![
                    SessionModeView {
                        id: "auto".to_string(),
                        name: "Auto".to_string(),
                        description: None,
                    },
                    SessionModeView {
                        id: "plan".to_string(),
                        name: "Plan".to_string(),
                        description: Some("Plan mode".to_string()),
                    },
                ],
                current: "auto".to_string(),
            });
            view.set_mode("plan".to_string(), cx);
            assert_eq!(
                view.session_config
                    .modes
                    .as_ref()
                    .expect("modes were injected")
                    .current,
                "plan",
                "set_mode flips current immediately (optimistic)"
            );

            // Connect-time state: the model in use advertises no `auto`.
            view.session_config.modes = Some(ModeStateView {
                available: vec![mode("default"), mode("plan")],
                current: "plan".to_string(),
            });
            // The agent switched to a model that supports `auto` and
            // re-advertised — list and current mode both move.
            view.apply_event(
                AcpEvent::ModeChanged {
                    state: ModeStateView {
                        available: vec![mode("auto"), mode("default"), mode("plan")],
                        current: "auto".to_string(),
                    },
                },
                "",
                false,
                cx,
            );
            let modes = view
                .session_config
                .modes
                .as_ref()
                .expect("modes stay advertised");
            assert_eq!(
                modes
                    .available
                    .iter()
                    .map(|m| m.id.as_str())
                    .collect::<Vec<_>>(),
                ["auto", "default", "plan"],
                "the rebuilt list replaces the connect-time one"
            );
            assert_eq!(
                modes.current, "auto",
                "a mode absent from the connect-time list still applies"
            );

            view.apply_event(
                AcpEvent::ConfigOptionsChanged(vec![ConfigOptionView {
                    id: "model".to_string(),
                    name: "Model".to_string(),
                    description: None,
                    category: ConfigOptionCategoryView::Model,
                    kind: daruda_acp::ConfigOptionKindView::Select {
                        current_value: "sonnet".to_string(),
                        options: vec![ConfigChoiceView {
                            value: "sonnet".to_string(),
                            name: "Sonnet".to_string(),
                            description: None,
                        }],
                    },
                }]),
                "",
                false,
                cx,
            );
            assert_eq!(
                view.session_config
                    .modes
                    .as_ref()
                    .expect("modes untouched")
                    .current,
                "auto"
            );
            assert_eq!(view.session_config.config_options.len(), 1);
        })
        .unwrap();
}

#[gpui::test]
fn start_telegram_watch_if_arms_only_for_telegram_origin(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, _cx| {
            view.start_telegram_watch_if(super::PromptOrigin::InApp);
            assert!(
                !view.is_waiting_for_telegram_first_response(),
                "an in-app dispatch never arms the watch"
            );

            view.start_telegram_watch_if(super::PromptOrigin::Telegram);
            assert!(
                view.is_waiting_for_telegram_first_response(),
                "a telegram-origin dispatch arms the watch"
            );
        })
        .unwrap();
}

/// A telegram-origin prompt that has to wait behind an in-flight turn must
/// not arm the watch while merely queued — only once it actually drains
/// onto the wire.
#[gpui::test]
fn queued_telegram_prompt_arms_the_watch_only_once_drained(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.set_turn_in_flight();
            view.enqueue_prompt("from telegram".to_string(), super::PromptOrigin::Telegram);
            assert!(
                !view.is_waiting_for_telegram_first_response(),
                "queuing alone must not arm the watch"
            );

            view.set_turn_idle();
            view.drain_next_queued_prompt_for_test(cx);
            assert!(
                view.is_waiting_for_telegram_first_response(),
                "draining the telegram-origin prompt arms the watch"
            );
        })
        .unwrap();
}

#[gpui::test]
fn telegram_prompt_does_not_replace_in_app_queue_edit(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.set_turn_in_flight();
            let id = view.enqueue_prompt("local draft".to_string(), super::PromptOrigin::InApp);
            view.begin_edit(id, cx);

            let dispatch = view.send_prompt_text_for_telegram("from phone".to_string(), cx);

            assert_eq!(dispatch, super::PromptDispatch::Queued);
            assert_eq!(
                view.queue
                    .pending_prompts
                    .iter()
                    .map(|q| q.text.as_str())
                    .collect::<Vec<_>>(),
                vec!["local draft", "from phone"],
                "a phone reply is a new prompt, not an edit commit"
            );
            assert_eq!(
                view.queue.editing_prompt,
                Some(id),
                "the in-app edit target remains active"
            );
        })
        .unwrap();
}

#[gpui::test]
fn queued_in_app_prompt_never_arms_the_watch(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.set_turn_in_flight();
            view.enqueue_prompt("typed in app".to_string(), super::PromptOrigin::InApp);
            view.set_turn_idle();

            view.drain_next_queued_prompt_for_test(cx);
            assert!(
                !view.is_waiting_for_telegram_first_response(),
                "an in-app-origin drain must not arm the watch"
            );
        })
        .unwrap();
}

#[gpui::test]
fn turn_end_resolves_telegram_watch_after_finalizing_streaming_text(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.set_turn_in_flight();
            view.start_telegram_first_response_watch_for_test(std::time::Instant::now());
            view.items.push(daruda_acp::ChatItem::AssistantText {
                text: "done".to_string(),
                streaming: true,
                message_id: None,
                phase: Default::default(),
            });

            let effect = view.apply_event(
                daruda_acp::AcpEvent::TurnEnded {
                    completed_normally: true,
                    stop_reason: "EndTurn".into(),
                },
                "",
                false,
                cx,
            );

            assert_eq!(
                effect,
                super::TelegramFirstResponseEffect::Relay(super::FirstResponseOutcome::Text(
                    "done".to_string()
                ))
            );
            assert!(
                !view.is_waiting_for_telegram_first_response(),
                "the resolved watch is cleared at the terminal boundary"
            );
        })
        .unwrap();
}

/// A `session/load` replay coalesces row rebuilding until it reaches a
/// terminal resume signal. The same gate must also release on failure paths so
/// replayed content is not left invisible.
#[gpui::test]
fn restoring_gate_releases_on_connected_error_or_abort(cx: &mut gpui::TestAppContext) {
    use daruda_acp::{AcpEvent, ChatItem};

    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            // Simulate a resume mid-replay: gate set, items populated by the
            // replayed updates, rows not yet projected.
            view.restoring = true;
            view.items = vec![ChatItem::UserText("q".into()), assistant_text_item("a")];
            view.rows.clear();

            // A non-terminal event during the replay must NOT rebuild rows.
            view.apply_event(AcpEvent::Notice("still loading".into()), "", false, cx);
            assert!(view.restoring, "gate stays set until Connected/Error");
            assert!(
                view.rows.is_empty(),
                "row rebuild is coalesced while restoring"
            );
            assert_eq!(view.items.len(), 2, "items still accumulate during replay");

            // Connected clears the gate and runs the catch-up rebuild.
            view.apply_event(
                AcpEvent::Connected {
                    program: None,
                    session_id: "sess-1".into(),
                    modes: None,
                    config_options: Vec::new(),
                    capabilities: Default::default(),
                    login_methods: Vec::new(),
                },
                "",
                false,
                cx,
            );
            assert!(!view.restoring, "Connected releases the gate");
            assert_eq!(view.session_id.as_deref(), Some("sess-1"));
            assert!(
                !view.rows.is_empty(),
                "the catch-up rebuild projects the replayed items"
            );
            assert_eq!(
                view.items.len(),
                2,
                "resume keeps the replayed conversation"
            );

            // A terminal Error mid-restore also clears the gate.
            view.restoring = true;
            view.apply_event(
                AcpEvent::Error(daruda_acp::AcpFailure::unclassified("load rejected")),
                "",
                false,
                cx,
            );
            assert!(!view.restoring, "Error releases the restore gate");

            // The end-of-stream guard releases a gate stuck with no terminal
            // event, projecting whatever accumulated.
            view.restoring = true;
            view.items = vec![ChatItem::UserText("q".into())];
            view.rows.clear();
            view.abort_restore(cx);
            assert!(!view.restoring, "abort_restore releases the gate");
            assert!(
                !view.rows.is_empty(),
                "abort_restore projects the accumulated items"
            );
        })
        .unwrap();
}

/// A `session/prompt` JSON-RPC failure keeps the ACP session alive: it shows an
/// inline error, settles the foreground turn, and records an errored completion
/// without switching the pane to terminal `Error`.
#[gpui::test]
fn turn_failed_keeps_session_connected_and_shows_error(cx: &mut gpui::TestAppContext) {
    use daruda_acp::{AcpEvent, ChatItem};

    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.status = super::AgentSessionStatus::Connected;
            view.set_turn_in_flight();

            view.apply_event(
                AcpEvent::TurnFailed(daruda_acp::AcpFailure::unclassified(
                    "session limit reached",
                )),
                "",
                false,
                cx,
            );

            assert!(
                matches!(view.status, super::AgentSessionStatus::Connected),
                "a per-turn failure must not kill the session, got {:?}",
                view.status
            );
            assert!(view.turn_is_idle(), "the failed turn must settle to idle");
            assert_eq!(
                view.items.last(),
                Some(&ChatItem::Failure(daruda_acp::AcpFailure::unclassified(
                    "session limit reached"
                ))),
                "the failure is surfaced as an inline Error item"
            );
            assert_eq!(
                view.activity.pending_completion,
                Some(super::TurnOutcome::Errored)
            );
        })
        .unwrap();
}

/// `/clear` resets the local session model before the workspace starts the
/// fresh connection.
#[gpui::test]
fn reset_for_new_session_clears_conversation_state(cx: &mut gpui::TestAppContext) {
    use daruda_acp::{ChatItem, PlanEntryView, PlanPriority, PlanStatus};

    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.session_id = Some("abc".to_string());
            view.items.push(ChatItem::UserText("hi".into()));
            view.items.push(ChatItem::UserText("again".into()));
            view.set_turn_in_flight();
            view.plan.push(PlanEntryView {
                content: "step 1".into(),
                priority: PlanPriority::Medium,
                status: PlanStatus::Pending,
            });
            view.fold
                .toggle(FoldKey::Tool("call-1".into()), FoldContext::past(true));
            assert!(
                !view
                    .fold
                    .is_expanded(&FoldKey::Tool("call-1".into()), FoldContext::past(true)),
                "sanity: override collapsed the block while active"
            );
            view.fold.set_mode(FoldPreset::Expanded.mode());

            view.reset_for_new_session(cx);

            assert!(view.items.is_empty(), "reset clears the conversation items");
            assert!(
                view.rows.is_empty(),
                "reset splices the projected rows to 0"
            );
            assert_eq!(
                view.session_id, None,
                "reset clears the persisted session id"
            );
            assert_eq!(
                view.status,
                super::AgentSessionStatus::Connecting,
                "reset parks the view in Connecting for the fresh session/new"
            );
            assert!(view.turn_is_idle(), "reset clears the in-flight turn flag");
            assert!(view.plan.is_empty(), "reset clears the execution plan");
            assert!(
                view.fold
                    .is_expanded(&FoldKey::Tool("call-1".into()), FoldContext::past(true)),
                "reset drops fold overrides back to the natural default"
            );
            assert_eq!(
                view.fold.chosen_mode(),
                Some(FoldPreset::Expanded.mode()),
                "reset keeps the pane's fold mode: it is a user preference, not                  conversation state, and wiping it here also erases it from disk"
            );
        })
        .unwrap();
}

/// The "Esc twice clears the queue" gesture must not be blocked by trailing
/// background-subagent liveness: after a Stop parks the queue, a running
/// subagent keeps `is_busy()` true (its quiescence window), but `handle_escape`
/// still clears the parked queue rather than routing back into a cancel.
#[gpui::test]
fn escape_clears_parked_queue_even_while_a_subagent_runs(cx: &mut gpui::TestAppContext) {
    use daruda_acp::{ChatItem, ToolCallItem, ToolKindView, ToolStatusView};
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            // A queue parked by a prior Stop.
            view.queue.paused_prompts.push(queued(1, "a"));
            // A live child tool under a subagent parent → `is_busy()` true
            // even though no foreground turn is in flight.
            view.items.push(ChatItem::ToolCall(ToolCallItem {
                id: "child".into(),
                title: "child".into(),
                kind: ToolKindView::Read,
                tool_name: None,
                status: ToolStatusView::InProgress,
                diffs: Vec::new(),
                output: Vec::new(),
                raw_input: None,
                parent_tool_id: Some("parent".into()),
                exit: None,
            }));
            assert!(view.is_busy(), "the trailing subagent keeps the pane busy");
            assert!(view.turn_is_idle(), "but no foreground turn is in flight");

            let outcome = view.handle_escape(cx);

            assert!(
                matches!(outcome, super::EscapeOutcome::ClearedQueue),
                "Escape clears the parked queue instead of re-cancelling"
            );
            assert!(
                view.queue.paused_prompts.is_empty(),
                "the parked queue is discarded"
            );
        })
        .unwrap();
}

/// `reconcile_post_turn` withholds the delta until `quiescence` has
/// elapsed since the dirty stamp, then relays it and advances the marker
/// so a second call (still no new text) is a no-op.
#[gpui::test]
fn reconcile_post_turn_waits_for_quiescence_then_dedups(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    let quiescence = std::time::Duration::from_millis(100);
    let dirty_at = std::time::Instant::now();

    window
        .update(cx, |view, _window, _cx| {
            view.items.push(assistant_text_item("done"));
            view.activity.post_turn_dirty_at = Some(dirty_at);
        })
        .unwrap();

    window
        .update(cx, |view, _window, _cx| {
            assert_eq!(
                view.reconcile_post_turn(dirty_at, quiescence),
                None,
                "not yet quiesced"
            );
        })
        .unwrap();

    let settled = dirty_at + quiescence + std::time::Duration::from_millis(1);
    window
        .update(cx, |view, _window, _cx| {
            assert_eq!(
                view.reconcile_post_turn(settled, quiescence),
                Some("done".to_string()),
                "quiesced: relays the delta and advances the marker"
            );
            assert_eq!(
                view.reconcile_post_turn(settled, quiescence),
                None,
                "second call has nothing new to relay"
            );
        })
        .unwrap();
}

/// `take_pending_post_turn` force-flushes a not-yet-quiesced follow-up
/// exactly once, then reports nothing pending.
#[gpui::test]
fn take_pending_post_turn_flushes_once(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, _cx| {
            view.items.push(assistant_text_item("flushed"));
            view.activity.post_turn_dirty_at = Some(std::time::Instant::now());
            assert_eq!(view.take_pending_post_turn(), Some("flushed".to_string()));
            assert_eq!(view.take_pending_post_turn(), None);
        })
        .unwrap();
}

/// After `snap_post_turn_baseline()` syncs the marker to a pre-populated
/// history (as every `restoring = false` site does), a stray post-turn
/// update that arrives with no new assistant text must not resurrect the
/// replayed conversation as a "background follow-up".
#[gpui::test]
fn snap_post_turn_baseline_prevents_replay_from_being_relayed(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, _cx| {
            view.items.push(assistant_text_item("history-1"));
            view.items.push(assistant_text_item("history-2"));
            view.snap_post_turn_baseline();
            assert_eq!(view.activity.post_turn_relayed_assistant_texts, 2);

            view.activity.post_turn_dirty_at =
                Some(std::time::Instant::now() - std::time::Duration::from_secs(10));
            assert_eq!(
                view.reconcile_post_turn(std::time::Instant::now(), super::POST_TURN_QUIESCENCE),
                None,
                "resumed baseline must not re-relay replayed history"
            );
        })
        .unwrap();
}

#[test]
fn post_turn_delta_none_when_nothing_new() {
    let items = vec![assistant_text_item("promise")];
    assert_eq!(super::post_turn_delta(&items, 1), None);
}

#[test]
fn post_turn_delta_returns_new_item_and_advances_count() {
    let items = vec![
        assistant_text_item("promise"),
        assistant_text_item("완료되었습니다"),
    ];
    assert_eq!(
        super::post_turn_delta(&items, 1),
        Some(("완료되었습니다".to_string(), 2))
    );
}

#[test]
fn post_turn_delta_joins_multiple_new_items() {
    let items = vec![
        assistant_text_item("a"),
        assistant_text_item("b"),
        assistant_text_item("c"),
    ];
    assert_eq!(
        super::post_turn_delta(&items, 1),
        Some(("b\n\nc".to_string(), 3))
    );
}

#[test]
fn post_turn_delta_ignores_whitespace_only_delta() {
    let items = vec![assistant_text_item("x"), assistant_text_item("   ")];
    assert_eq!(super::post_turn_delta(&items, 1), None);
}

#[gpui::test]
fn the_compact_split_is_measured_on_the_pane_not_the_list(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            let split = crate::ui::theme::agent_chat_compact_options_w(cx);
            assert!(
                !view.activity_bar_is_compact(cx),
                "before the first paint measures anything, the three chips are assumed to fit"
            );

            view.set_pane_width(gpui::px(split - 1.0), cx);
            assert!(view.activity_bar_is_compact(cx));
            assert!(view.items.is_empty());
            assert!(view.list_bounds.is_none());

            view.items.push(assistant_text_item("now there is a list"));
            assert!(
                view.activity_bar_is_compact(cx),
                "gaining a transcript must not change which bar the pane shows"
            );

            view.set_pane_width(gpui::px(split + 1.0), cx);
            assert!(!view.activity_bar_is_compact(cx));
        })
        .expect("view update");
}

#[gpui::test]
fn only_a_measurement_that_flips_the_split_repaints(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    // The split is font-dependent; read it the way the pane does.
    let split = window
        .update(cx, |_view, _window, cx| {
            crate::ui::theme::agent_chat_compact_options_w(cx)
        })
        .expect("view update");
    let wide = split + 1.0;
    let set = |cx: &mut gpui::TestAppContext, w: f32| {
        window
            .update(cx, |view, _window, cx| view.set_pane_width(gpui::px(w), cx))
            .expect("view update");
        cx.run_until_parked();
    };
    let count = |cx: &mut gpui::TestAppContext| {
        window
            .update(cx, |view, _window, _cx| view.render_count.get())
            .expect("view update")
    };

    set(cx, wide);
    let settled = count(cx);
    assert!(
        settled >= 1,
        "the pane painted at least once, got {settled}"
    );

    set(cx, wide + 40.0);
    set(cx, wide + 80.0);
    assert_eq!(
        count(cx),
        settled,
        "a resize that stays on the wide side is not a layout change"
    );

    set(cx, split - 1.0);
    assert!(
        count(cx) > settled,
        "crossing into the compact bar has to repaint, or the chips stay stale"
    );
}

// ---------------------------------------------------------------------------
// Returning an axis to the configured default
// ---------------------------------------------------------------------------

use super::super::pane_choice::PaneChoice;
use super::super::rows::tail::TailWindow;
use super::super::transcript_defaults::TranscriptDefaults;
use crate::transcript::display_filter::{DisplayFilter, FilterFacet};

/// A config state distinct from the built-in one on every axis, so a reset
/// that lands on the *built-in* default instead of the one in force is caught.
fn other_defaults() -> TranscriptDefaults {
    TranscriptDefaults {
        tail: TailWindow::Last(5),
        fold_mode: FoldPreset::Summary.mode(),
        filter: DisplayFilter::default().toggled(FilterFacet::Thinking),
    }
}

/// The one check that tells `Seeded(x)` from `Chosen(x)`: both panes hold the
/// same value, and only the one that is *following* moves when config does.
#[gpui::test]
fn a_reset_axis_follows_the_next_default_but_an_equal_choice_does_not(
    cx: &mut gpui::TestAppContext,
) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            // Pin the value the pane already shows — a choice, not agreement.
            view.set_tail_window(TailWindow::All, cx);
            assert_eq!(view.tail, PaneChoice::Chosen(TailWindow::All));
            view.reseed_transcript_defaults(&other_defaults(), cx);
            assert_eq!(
                view.tail,
                PaneChoice::Chosen(TailWindow::All),
                "a choice that happens to equal the old default is still a choice"
            );

            // The same pane, handed back: it lands on the default now in force,
            // not on the one it was built with.
            view.reset_tail_window(cx);
            assert_eq!(view.tail, PaneChoice::Seeded(TailWindow::Last(5)));

            view.reseed_transcript_defaults(
                &TranscriptDefaults {
                    tail: TailWindow::Last(3),
                    ..other_defaults()
                },
                cx,
            );
            assert_eq!(
                view.tail,
                PaneChoice::Seeded(TailWindow::Last(3)),
                "a reset pane tracks every later config edit"
            );
        })
        .expect("view update");
}

/// What the panel footers pass to `.disabled(..)`. Value equality is the wrong
/// question: a pane pinned to the default's own value still has an override to
/// undo, and a following pane has none.
#[gpui::test]
fn the_reset_is_offered_on_a_chosen_default_and_withheld_while_following(
    cx: &mut gpui::TestAppContext,
) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, window, cx| {
            assert!(view.fold.mode_choice().is_following(), "a fresh pane");

            view.set_fold_mode(view.defaults.fold_mode, window, cx);
            assert!(
                !view.fold.mode_choice().is_following(),
                "pinning the default's own value still offers the reset"
            );

            view.reset_fold_mode(window, cx);
            assert!(view.fold.mode_choice().is_following());
        })
        .expect("view update");
}

#[gpui::test]
fn resetting_the_tail_window_hands_the_axis_back(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.reseed_transcript_defaults(&other_defaults(), cx);
            view.set_tail_window(TailWindow::Last(1), cx);
            view.reset_tail_window(cx);
            assert_eq!(view.tail, PaneChoice::Seeded(other_defaults().tail));
        })
        .expect("view update");
}

#[gpui::test]
fn resetting_the_fold_mode_hands_the_axis_back(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, window, cx| {
            view.reseed_transcript_defaults(&other_defaults(), cx);
            view.set_fold_mode(FoldPreset::Expanded.mode(), window, cx);
            view.reset_fold_mode(window, cx);
            assert_eq!(
                view.fold.mode_choice(),
                PaneChoice::Seeded(other_defaults().fold_mode)
            );
        })
        .expect("view update");
}

#[gpui::test]
fn resetting_the_display_filter_hands_the_axis_back(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.reseed_transcript_defaults(&other_defaults(), cx);
            view.toggle_display_facet(FilterFacet::ToolRun, cx);
            view.reset_display_filter(cx);
            assert_eq!(
                view.display_filter,
                PaneChoice::Seeded(other_defaults().filter)
            );
        })
        .expect("view update");
}

/// The whole point of the `Custom` segment: pressing a preset must not throw
/// away a hand-edited matrix.
#[gpui::test]
fn the_custom_segment_brings_back_the_hand_edited_matrix(cx: &mut gpui::TestAppContext) {
    use crate::transcript::fold_mode::{BlockRule, FoldBlock, TurnPosition};

    let window = make_test_view(cx);
    window
        .update(cx, |view, window, cx| {
            let edited = FoldPreset::Auto.mode().with_rule(
                TurnPosition::Past,
                FoldBlock::Thinking,
                BlockRule::Collapsed,
            );
            view.set_fold_mode(edited, window, cx);
            assert_eq!(view.fold.mode(), edited);

            view.select_fold_preset(Some(FoldPreset::Summary), window, cx);
            assert_eq!(view.fold.mode(), FoldPreset::Summary.mode());

            view.select_fold_preset(None, window, cx);
            assert_eq!(view.fold.mode(), edited, "the edited matrix, cell for cell");
        })
        .expect("view update");
}

/// `Reset to default` sits beside the preset strip and is the other way out of
/// a hand-edited matrix, so it has to keep the edit reachable too — otherwise
/// two adjacent controls lose the same work differently.
#[gpui::test]
fn resetting_to_the_default_still_leaves_the_matrix_reachable(cx: &mut gpui::TestAppContext) {
    use crate::transcript::fold_mode::{BlockRule, FoldBlock, TurnPosition};

    let window = make_test_view(cx);
    window
        .update(cx, |view, window, cx| {
            let edited = FoldPreset::Auto.mode().with_rule(
                TurnPosition::Past,
                FoldBlock::Thinking,
                BlockRule::Collapsed,
            );
            view.set_fold_mode(edited, window, cx);

            view.reset_fold_mode(window, cx);
            assert_eq!(
                view.fold.mode(),
                view.defaults.fold_mode,
                "the pane follows the default again"
            );

            view.select_fold_preset(None, window, cx);
            assert_eq!(view.fold.mode(), edited, "the edited matrix, cell for cell");
        })
        .expect("view update");
}

/// A configured default is a token list, so it can encode a matrix of its own.
/// The capture has to key on leaving the edit, not on the default happening to
/// be a preset — otherwise resetting onto a custom default loses it silently.
#[gpui::test]
fn a_custom_default_does_not_swallow_the_edit_on_reset(cx: &mut gpui::TestAppContext) {
    use crate::transcript::fold_mode::{BlockRule, FoldBlock, TurnPosition};

    let window = make_test_view(cx);
    window
        .update(cx, |view, window, cx| {
            let custom_default = FoldPreset::Summary.mode().with_rule(
                TurnPosition::Last,
                FoldBlock::Tool,
                BlockRule::Expanded,
            );
            assert!(
                custom_default.preset().is_none(),
                "the fixture only means something if the default is itself custom"
            );
            view.reseed_transcript_defaults(
                &TranscriptDefaults {
                    fold_mode: custom_default,
                    ..other_defaults()
                },
                cx,
            );

            let edited = FoldPreset::Auto.mode().with_rule(
                TurnPosition::Past,
                FoldBlock::Thinking,
                BlockRule::Collapsed,
            );
            view.set_fold_mode(edited, window, cx);

            view.reset_fold_mode(window, cx);
            assert_eq!(view.fold.mode(), custom_default, "it follows the default");

            view.select_fold_preset(None, window, cx);
            assert_eq!(view.fold.mode(), edited, "the edit is still reachable");
        })
        .expect("view update");
}

/// Editing goes custom→custom and keeps the remembered target current, so
/// pressing the already-selected Custom segment cannot roll the pane back to an
/// older matrix.
#[gpui::test]
fn editing_a_custom_matrix_updates_what_is_remembered(cx: &mut gpui::TestAppContext) {
    use crate::transcript::fold_mode::{BlockRule, FoldBlock, TurnPosition};

    let window = make_test_view(cx);
    window
        .update(cx, |view, window, cx| {
            let first = FoldPreset::Auto.mode().with_rule(
                TurnPosition::Past,
                FoldBlock::Thinking,
                BlockRule::Collapsed,
            );
            let second = first.with_rule(TurnPosition::Last, FoldBlock::Diff, BlockRule::Expanded);
            view.set_fold_mode(first, window, cx);
            assert_eq!(view.fold_editor.custom(), Some(first));
            view.set_fold_mode(second, window, cx);
            assert_eq!(
                view.fold_editor.custom(),
                Some(second),
                "custom→custom must keep the segment target on the latest edit"
            );

            view.select_fold_preset(None, window, cx);
            assert_eq!(
                view.fold.mode(),
                second,
                "clicking the selected Custom segment is a no-op"
            );

            view.select_fold_preset(Some(FoldPreset::Expanded), window, cx);
            assert_eq!(view.fold_editor.custom(), Some(second));

            view.select_fold_preset(None, window, cx);
            assert_eq!(view.fold.mode(), second, "the final edit, not the first");
        })
        .expect("view update");
}

/// Nothing edited yet means nothing to return to, so the segment is a no-op
/// (and the strip renders it disabled).
#[gpui::test]
fn the_custom_segment_is_inert_with_nothing_remembered(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, window, cx| {
            view.select_fold_preset(Some(FoldPreset::Summary), window, cx);
            view.select_fold_preset(None, window, cx);
            assert_eq!(view.fold_editor.custom(), None);
            assert_eq!(view.fold.mode(), FoldPreset::Summary.mode());
        })
        .expect("view update");
}
