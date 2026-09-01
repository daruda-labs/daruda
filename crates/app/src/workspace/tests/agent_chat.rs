//! AgentChat pane — open action and the pure parts of the prompt / permission
//! / mode ops (no live ACP session required). Tests build the view via
//! `create_agent_chat_pane`, which opens no connection, so `handle` stays
//! `None` while host-side state transitions still run.

use daruda_store::project::PaneCwd;
use gpui::{AppContext as _, Entity, TestAppContext};

use super::build_workspace;
use crate::workspace::Workspace;
use crate::workspace::main_area::agent_chat_pane::display_filter::{DisplayFilter, FilterFacet};
use crate::workspace::main_area::agent_chat_pane::fold_mode::{FoldMode, FoldPreset};
use crate::workspace::main_area::agent_chat_pane::pane_choice::PaneChoice;
use crate::workspace::main_area::agent_chat_pane::rows::tail::TailWindow;
use crate::workspace::main_area::agent_chat_pane::view::{
    AgentChatView, AgentSessionStatus, ChatContentWidth,
};
use crate::workspace::main_area::pane::PaneContent;
use crate::workspace::main_area::pane_tree::PaneId;

/// Fetch the `AgentChatView` entity for `pane_id` (panics if the pane is gone
/// or is not an AgentChat pane).
pub(super) fn agent_view(ws: &Workspace, pane_id: PaneId) -> Entity<AgentChatView> {
    ws.active_runtime()
        .panes
        .iter()
        .find(|p| p.id == pane_id)
        .and_then(|p| p.agent_chat_view())
        .cloned()
        .expect("agent chat pane present")
}

/// The queued-prompt texts in FIFO order — the queue holds `QueuedPrompt`
/// (id + text), so tests compare on the text projection.
fn queue_texts(v: &AgentChatView) -> Vec<String> {
    v.queue
        .pending_prompts
        .iter()
        .map(|q| q.text.clone())
        .collect()
}

/// A pane's diff embeds are fingerprinted against the Workspace-resolved syntax
/// palette, and a fold expand or a seeded transcript can ask for one before any
/// ACP event has arrived. Filling the field from the first event instead left an
/// event-less pane rendering every diff through the inline fallback for the rest
/// of its life, so the seed belongs at construction.
#[gpui::test]
async fn a_new_pane_is_born_with_the_resolved_syntax_theme(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.open_agent_chat_pane(window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();

    workspace.read_with(cx, |ws, cx| {
        let pane_id = ws.active_runtime().panes.last().expect("pane opened").id;
        let view = agent_view(ws, pane_id);
        assert_eq!(
            view.read(cx).syntax_theme(),
            ws.syntax_theme,
            "the pane starts on the Workspace's resolved palette, not on nothing"
        );
    });
}

#[gpui::test]
async fn open_agent_chat_pane_creates_agent_chat_leaf(cx: &mut TestAppContext) {
    use crate::surface::strings as s;

    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    let tabs_before = workspace.read_with(cx, |ws, _| ws.active_runtime().tabs.len());

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.open_agent_chat_pane(window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();

    workspace.read_with(cx, |ws, cx| {
        assert_eq!(
            ws.active_runtime().tabs.len(),
            tabs_before + 1,
            "opening an agent chat pane appends a tab"
        );
        let pane = ws
            .active_runtime()
            .panes
            .last()
            .expect("open_agent_chat_pane pushed a pane");
        match &pane.content {
            PaneContent::AgentChat(ac) => {
                let view = ac.view.read(cx);
                // No resolvable lane cwd → the pane parks in `Error` rather
                // than attempting a connection, keeping the suite offline.
                let AgentSessionStatus::Error { message, .. } = &view.status else {
                    panic!(
                        "no lane cwd → error status, not a live connect, got {:?}",
                        view.status
                    );
                };
                assert_eq!(message.as_str(), s::agent_chat_no_lane_cwd());
                assert_ne!(
                    message.as_str(),
                    s::agent_chat_error_prefix(),
                    "payload must be the reason, not the prefix the banner re-adds"
                );
                assert!(view.items.is_empty(), "items start empty");
                assert!(view.handle.is_none(), "no session without a cwd");
            }
            _ => panic!("expected an AgentChat pane"),
        }
        assert_eq!(ws.active_runtime().focused_pane_id, pane.id);
    });

    let with_cwd = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let remote = ws.create_agent_chat_pane(
                    Some(PaneCwd::Remote("host:/repo/lane".to_string())),
                    None,
                    daruda_config::AgentDefinition::claude_default().id,
                    None,
                    window,
                    cx,
                );
                assert_eq!(
                    remote.cwd(),
                    None,
                    "PaneCwd::Remote must not surface through Pane::cwd()"
                );

                let tmp = std::env::temp_dir();
                let pane = ws.create_agent_chat_pane(
                    Some(PaneCwd::Local(tmp.clone())),
                    None,
                    daruda_config::AgentDefinition::claude_default().id,
                    None,
                    window,
                    cx,
                );
                assert_eq!(pane.cwd(), Some(tmp.as_path()));
                match &pane.content {
                    PaneContent::AgentChat(ac) => ac.view.read(cx).status.clone(),
                    _ => panic!("expected an AgentChat pane"),
                }
            })
        })
        .unwrap();

    assert_eq!(
        with_cwd,
        AgentSessionStatus::Idle,
        "a pane with a cwd must stay dormant until first focus, got {with_cwd:?}"
    );
}

/// Core perf contract: a `cx.notify()` on the `AgentChatView` must
/// re-render the (cached) view — the mechanism that lets an async event (a
/// streamed chunk, a landed mermaid image) repaint the conversation without the
/// whole window re-rendering. Guards against a future change that breaks the
/// cached-view notify path (the lost-wakeup class).
#[gpui::test]
async fn notify_rerenders_cached_agent_view(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();
    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| ws.open_agent_chat_pane(window, cx));
    })
    .unwrap();
    cx.run_until_parked();

    let view = workspace.read_with(cx, |ws, _| {
        ws.active_runtime()
            .panes
            .last()
            .and_then(|p| p.agent_chat_view())
            .cloned()
            .expect("agent chat pane present")
    });
    let before = view.read_with(cx, |v, _| v.render_count.get());
    assert!(
        before >= 1,
        "the view should have rendered at least once after open, got {before}"
    );

    // Simulate the async mermaid-completion / re-parse notify.
    cx.update(|cx| view.update(cx, |_v, cx| cx.notify()));
    cx.run_until_parked();

    let after = view.read_with(cx, |v, _| v.render_count.get());
    assert!(
        after > before,
        "cx.notify() must re-render the cached view: before={before} after={after}"
    );
}

#[gpui::test]
async fn cancel_turn_ends_the_turn_locally_without_an_agent_reply(cx: &mut TestAppContext) {
    use daruda_acp::{ChatItem, ToolCallItem, ToolKindView, ToolStatusView};

    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    let tmp = std::env::temp_dir();
    let pane_id = cx
        .update_window(window_handle.into(), |_, window, cx| {
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
                let view = agent_view(ws, id);
                // A turn mid-flight: streaming text + a still-running tool call,
                // exactly the state a hung agent leaves (no stop reason ever
                // arrives, so `TurnEnded` would never clear this).
                view.update(cx, |v, _| {
                    v.set_turn_in_flight();
                    v.items = vec![
                        ChatItem::AssistantText {
                            text: "working".into(),
                            streaming: true,
                            message_id: None,
                            phase: Default::default(),
                        },
                        ChatItem::ToolCall(ToolCallItem {
                            id: "t1".into(),
                            title: "Read".into(),
                            kind: ToolKindView::Read,
                            tool_name: None,
                            status: ToolStatusView::InProgress,
                            diffs: Vec::new(),
                            output: Vec::new(),
                            raw_input: None,
                            parent_tool_id: None,
                            exit: None,
                        }),
                    ];
                });
                // Offline (no handle): `session/cancel` is a no-op, so only the
                // authoritative local teardown can end the turn.
                ws.cancel_agent_turn(id, cx);
                id
            })
        })
        .unwrap();
    cx.run_until_parked();

    workspace.read_with(cx, |ws, cx| {
        let view = agent_view(ws, pane_id);
        let view = view.read(cx);
        assert!(
            view.turn_is_idle(),
            "Stop ends the turn without an agent reply"
        );
        assert!(view.turn_is_idle(), "the turn is settled to idle");
        let ChatItem::AssistantText { streaming, .. } = &view.items[0] else {
            panic!("expected the streamed assistant text");
        };
        assert!(!streaming, "streaming text settles on Stop");
        let ChatItem::ToolCall(tc) = &view.items[1] else {
            panic!("expected the tool call");
        };
        assert_eq!(
            tc.status,
            ToolStatusView::Cancelled,
            "the in-progress tool call is marked cancelled (stops the rollup pulse)"
        );
    });

    cx.update_window(window_handle.into(), |_, _window, cx| {
        workspace.update(cx, |ws, cx| {
            // Not busy yet → the Escape shim is a no-op and reports it did not
            // handle the key, so Escape can propagate normally.
            assert!(
                !ws.cancel_agent_turn_if_active(pane_id, cx),
                "no-op when the pane is idle"
            );

            // Simulate a turn in flight (as `send_prompt` would); this makes
            // `is_busy()` true.
            agent_view(ws, pane_id).update(cx, |v, _| v.set_turn_in_flight());
            assert!(
                ws.cancel_agent_turn_if_active(pane_id, cx),
                "cancels and reports handled when the pane is busy"
            );

            // An id that is not an agent chat pane reports not-handled, so
            // Escape keeps propagating to ancestors.
            let bogus: PaneId = pane_id + 999;
            assert!(
                !ws.cancel_agent_turn_if_active(bogus, cx),
                "no-op for an id that is not an agent chat pane"
            );
        });
    })
    .unwrap();
}

/// Regression: the agent status indicator disappears after a lane switch. A
/// parked lane's AgentChat status must still reach the rendered left-dock
/// snapshot. Drives a real `activate_lane` switch and asserts on the real
/// `prepare_left_dock_snapshot`'s `agent_status_per_lane` map (the badge
/// source), exercising the production render path.
#[gpui::test]
async fn parked_lane_agent_status_reaches_left_dock_aggregate(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    // Both roots must exist so activation classifies them `Present`.
    let root_a = std::path::PathBuf::from("/tmp/test_acp_indicator_a");
    let root_b = std::path::PathBuf::from("/tmp/test_acp_indicator_b");
    let _ = std::fs::create_dir_all(&root_a);
    let _ = std::fs::create_dir_all(&root_b);
    let project = daruda_store::project::Project::from_path(&root_a);
    let window_handle = cx.add_window(|window, cx| {
        Workspace::new_with_project_for_test_full(
            &config,
            Some(project),
            super::fresh_test_data_dir(),
            window,
            cx,
        )
    });
    let workspace = window_handle.root(cx).unwrap();
    cx.run_until_parked();

    let lane0 = workspace.read_with(cx, |ws, _| ws.active);
    // Seed a second lane to switch into.
    workspace.update(cx, |ws, _| {
        if let Some(p) = ws.active_project_mut() {
            p.lanes
                .push(crate::lane::Lane::default_for_project(1, root_b.clone()));
        }
    });
    let lane1 = daruda_store::project::LaneRef {
        project: lane0.project,
        lane: 1,
    };

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            // An agent chat pane in lane 0 (active) with a turn in flight →
            // `to_session_status()` is `Working`. Wire it into a tab layout
            // (pane + TabEntry) so it appears in `pane_lane_index`; do it
            // directly rather than via the open action to skip `focus_pane`'s
            // lazy `maybe_connect`, which would spawn a real adapter.
            let pane = ws.create_agent_chat_pane(
                Some(PaneCwd::Local(root_a.clone())),
                None,
                daruda_config::AgentDefinition::claude_default().id,
                None,
                window,
                cx,
            );
            let id = pane.id;
            let tab_id = ws.alloc_id();
            ws.active_runtime_mut().panes.push(pane);
            ws.active_runtime_mut()
                .tabs
                .push(crate::workspace::main_area::pane::TabEntry {
                    id: tab_id,
                    layout: crate::workspace::main_area::pane_tree::PaneLayout::Pane(id),
                    last_focused_pane: id,
                    user_label: None,
                });
            agent_view(ws, id).update(cx, |v, _| {
                v.status = AgentSessionStatus::Connected;
                v.set_turn_in_flight();
            });

            // Switch away → lane 0 is now parked, lane 1 active.
            ws.activate_lane(lane1, window, cx);
            assert_eq!(ws.active, lane1);
            assert!(
                ws.agent_chat_view(id).is_some(),
                "agent_chat_view must find a pane parked in an inactive lane"
            );

            // Build the actual left-dock snapshot the renderer consumes and
            // assert the parked lane's badge source is `Working`.
            let snap = ws.prepare_left_dock_snapshot(cx);
            assert_eq!(
                snap.agent_status_per_lane.get(&lane0),
                Some(&daruda_agent::SessionStatus::Working),
                "a parked lane's agent Working status must appear in the rendered \
                 left-dock snapshot keyed by its LaneRef"
            );
        });
    })
    .unwrap();

    let _ = std::fs::remove_dir_all(&root_a);
    let _ = std::fs::remove_dir_all(&root_b);
}

#[gpui::test]
async fn bottom_dock_snapshot_reflects_active_agent_queue(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    let tmp = std::env::temp_dir();
    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let first = ws.create_agent_chat_pane(
                Some(PaneCwd::Local(tmp.clone())),
                None,
                daruda_config::AgentDefinition::claude_default().id,
                None,
                window,
                cx,
            );
            let first_id = first.id;
            ws.active_runtime_mut().panes.push(first);
            ws.send_agent_prompt_text(first_id, "inactive".to_string(), cx);

            let second = ws.create_agent_chat_pane(
                Some(PaneCwd::Local(tmp.clone())),
                None,
                daruda_config::AgentDefinition::claude_default().id,
                None,
                window,
                cx,
            );
            let second_id = second.id;
            ws.active_runtime_mut().panes.push(second);
            ws.active_runtime_mut().focused_pane_id = second_id;
            ws.send_agent_prompt_text(second_id, "active first".to_string(), cx);
            ws.send_agent_prompt_text(second_id, "active second".to_string(), cx);

            let snap = ws.prepare_bottom_dock_snapshot(cx);
            let (pane_id, queued) = snap
                .queued_prompts
                .as_ref()
                .expect("active agent queue is projected into the bottom snapshot");
            assert_eq!(*pane_id, second_id);
            assert_eq!(
                queued.iter().map(|q| q.text.clone()).collect::<Vec<_>>(),
                vec!["active first".to_string(), "active second".to_string()],
                "only the focused agent pane's queue is projected"
            );

            ws.active_runtime_mut().focused_pane_id = first_id;
            let snap = ws.prepare_bottom_dock_snapshot(cx);
            let (pane_id, queued) = snap
                .queued_prompts
                .as_ref()
                .expect("newly active agent queue is projected");
            assert_eq!(*pane_id, first_id);
            assert_eq!(
                queued.iter().map(|q| q.text.clone()).collect::<Vec<_>>(),
                vec!["inactive".to_string()]
            );
        });
    })
    .unwrap();
}

/// `deliver_text_to_pane` dispatch table: the funnel routes by pane kind at one
/// place. An AgentChat submit with a non-empty body echoes/queues a prompt; a
/// whitespace-only submit is an accepted no-op (no blank ACP turn — the
/// "Enter-only" macro case); a non-text pane (TaskEdit) and a missing id both
/// return `false`.
#[gpui::test]
async fn deliver_text_to_pane_routes_by_kind(cx: &mut TestAppContext) {
    use crate::workspace::main_area::pane_input_ops::{PaneTextInput, PaneTextIntent};

    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    let tmp = std::env::temp_dir();
    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            // AgentChat pane: non-empty submit → accepted + queued (no live
            // handle here, so the prompt is queued rather than echoed).
            let chat = ws.create_agent_chat_pane(
                Some(PaneCwd::Local(tmp.clone())),
                None,
                daruda_config::AgentDefinition::claude_default().id,
                None,
                window,
                cx,
            );
            let chat_id = chat.id;
            ws.active_runtime_mut().panes.push(chat);
            assert!(
                ws.deliver_text_to_pane(
                    chat_id,
                    PaneTextInput {
                        body: "  do the thing  ".to_string(),
                        intent: PaneTextIntent::Command { submit: true },
                    },
                    window,
                    cx,
                ),
                "AgentChat submit is accepted"
            );
            {
                let view = agent_view(ws, chat_id);
                let view = view.read(cx);
                assert!(view.items.is_empty(), "a queued prompt is not echoed");
                assert_eq!(
                    queue_texts(view),
                    vec!["do the thing".to_string()],
                    "the body is trimmed at the single dispatch point, then queued"
                );
            }

            // Whitespace-only submit → accepted no-op: no new queue entry, no turn.
            assert!(
                ws.deliver_text_to_pane(
                    chat_id,
                    PaneTextInput {
                        body: "   \n  ".to_string(),
                        intent: PaneTextIntent::Command { submit: true },
                    },
                    window,
                    cx,
                ),
                "whitespace-only submit is an accepted no-op (returns true)"
            );
            {
                let view = agent_view(ws, chat_id);
                let view = view.read(cx);
                assert_eq!(
                    queue_texts(view).len(),
                    1,
                    "a blank submit adds no queue entry and fires no ACP turn"
                );
                assert!(view.items.is_empty());
                assert!(view.turn_is_idle());
            }

            // Missing pane id → false.
            let bogus: PaneId = chat_id + 9999;
            assert!(
                !ws.deliver_text_to_pane(
                    bogus,
                    PaneTextInput {
                        body: "x".to_string(),
                        intent: PaneTextIntent::Command { submit: true },
                    },
                    window,
                    cx,
                ),
                "a missing pane id cannot receive text"
            );

            // TaskEdit pane → false (kind cannot receive delivered text).
            ws.open_task_edit_pane(None, window, cx);
            let te_id = ws
                .active_runtime()
                .panes
                .last()
                .expect("open_task_edit_pane pushed a pane")
                .id;
            assert!(
                !ws.deliver_text_to_pane(
                    te_id,
                    PaneTextInput {
                        body: "x".to_string(),
                        intent: PaneTextIntent::Command { submit: false },
                    },
                    window,
                    cx,
                ),
                "a TaskEdit pane is not a text-delivery target"
            );
        });
    })
    .unwrap();
}

/// A non-default `codex` id. Used by the restore-wiring tests below to prove the
/// persisted `agent_id` is threaded through `restore_from_disk` (not just the
/// pure `resolve_restored_agent` seam covered by unit tests).
fn codex_agent() -> daruda_config::AgentDefinition {
    daruda_config::AgentDefinition {
        id: "codex".to_string(),
        name: "Codex".to_string(),
        launch: daruda_config::AgentLaunch::Raw("codex-acp".to_string()),
        default_mode: None,
        default_model: None,
        fold_mode: None,
        tail_window: None,
        display_filter: None,
    }
}

/// The restore-visible slice of one agent-chat pane. A struct rather than a
/// tuple so each persisted preference is asserted by name — the mis-wire this
/// covers (one preference's tokens read into another's field) is invisible
/// when the assertions read by position.
struct RestoredChat {
    agent_id: String,
    session_id: Option<String>,
    title: Option<String>,
    restoring: bool,
    dormant: bool,
    content_width: ChatContentWidth,
    tail: PaneChoice<TailWindow>,
    display_filter: PaneChoice<DisplayFilter>,
    fold_mode: Option<FoldMode>,
}

fn restored_chats(ws: &Workspace, cx: &gpui::App) -> Vec<RestoredChat> {
    ws.active_runtime()
        .panes
        .iter()
        .filter_map(|p| p.agent_chat_view())
        .map(|view| {
            let view = view.read(cx);
            RestoredChat {
                agent_id: view.agent_id.clone(),
                session_id: view.session_id.clone(),
                title: view.session_title.clone(),
                restoring: view.restoring,
                dormant: view.handle.is_none(),
                content_width: view.content_width,
                tail: view.tail,
                display_filter: view.display_filter,
                fold_mode: view.fold.chosen_mode(),
            }
        })
        .collect()
}

/// Persist AgentChat panes with session identity and a non-default owner, then
/// restore both with and without that owner still in the catalog. A surviving
/// owner keeps its session id; a removed owner falls back to the default agent
/// and drops the id. Panes sit in non-active tabs so restore focus never
/// triggers a connect.
#[gpui::test]
async fn agent_chat_agent_id_restore_handles_present_and_removed_owner(cx: &mut TestAppContext) {
    use crate::workspace::main_area::pane::TabEntry;
    use crate::workspace::main_area::pane_tree::PaneLayout;

    let config = daruda_config::Config::default();
    // Per-process root: a fixed path makes the restore assertions depend on
    // whatever else in the run happened to write here first.
    let project_root = std::env::temp_dir().join(format!(
        "daruda_agent_chat_restore_agent_present_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&project_root);
    let _ = std::fs::create_dir_all(&project_root);
    let project = daruda_store::project::Project::from_path(&project_root);

    let (window_handle, workspace) = super::build_workspace_with(cx, &config, Some(project));
    cx.run_until_parked();

    let (workspace_state, project_states) = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let cwd = ws.active_lane().map(|w| PaneCwd::Local(w.path.clone()));

                // A default-agent pane with persisted session id + title. This
                // proves the snapshot/restore round-trip preserves resume
                // identity and the label shown before a session load completes.
                let titled = ws.create_agent_chat_pane(
                    cwd.clone(),
                    Some("sess-restore-123".to_string()),
                    daruda_config::AgentDefinition::claude_default().id,
                    None,
                    window,
                    cx,
                );
                let titled_id = titled.id;
                ws.active_runtime_mut().panes.push(titled);
                let titled_tab_id = ws.alloc_id();
                ws.active_runtime_mut().tabs.push(TabEntry {
                    id: titled_tab_id,
                    layout: PaneLayout::Pane(titled_id),
                    last_focused_pane: titled_id,
                    user_label: None,
                });
                let view = ws
                    .agent_chat_view(titled_id)
                    .cloned()
                    .expect("agent chat view present");
                view.update(cx, |v, cx| {
                    v.session_title = Some("Investigate flaky test".to_string());
                    v.content_width = ChatContentWidth::Reading;
                    v.tail = PaneChoice::Chosen(TailWindow::Last(3));
                    // Distinct token vocabularies: each preference's tokens are
                    // unreadable to the others, so a crossed wire restores a
                    // default rather than the wrong-but-plausible value.
                    v.display_filter =
                        PaneChoice::Chosen(DisplayFilter::default().toggled(FilterFacet::Prose));
                    v.fold.set_mode(FoldPreset::Summary.mode());
                    cx.notify();
                });

                // Pane owned by `codex` (a non-default agent), with a session id
                // stamped as a connected session would.
                let pane = ws.create_agent_chat_pane(
                    cwd,
                    Some("sess-codex-1".to_string()),
                    codex_agent().id,
                    None,
                    window,
                    cx,
                );
                let pane_id = pane.id;
                ws.active_runtime_mut().panes.push(pane);
                let tab_id = ws.alloc_id();
                ws.active_runtime_mut().tabs.push(TabEntry {
                    id: tab_id,
                    layout: PaneLayout::Pane(pane_id),
                    last_focused_pane: pane_id,
                    user_label: None,
                });
                ws.snapshot_for_disk(cx).expect("snapshot")
            })
        })
        .unwrap();

    // Restore into a fresh workspace whose catalog still includes `codex`.
    let restored_handle = cx.add_window(|window, cx| {
        let mut ws = Workspace::new_with_project_for_test(
            &config,
            None,
            super::fresh_test_data_dir(),
            window,
            cx,
        );
        ws.agents = vec![
            daruda_config::AgentDefinition::claude_default(),
            codex_agent(),
        ];
        ws.restore_from_disk(&workspace_state, &project_states, window, cx);
        ws
    });
    let restored = restored_handle.root(cx).unwrap();

    restored.read_with(cx, |ws, cx| {
        let views = restored_chats(ws, cx);
        let titled = views
            .iter()
            .find(|v| v.session_id.as_deref() == Some("sess-restore-123"))
            .expect("restored titled agent chat pane present");
        assert_eq!(
            titled.title.as_deref(),
            Some("Investigate flaky test"),
            "persisted title must round-trip (seeds the tab label before load)"
        );
        assert!(
            !titled.restoring,
            "a restored dormant pane is not yet loading"
        );
        assert!(titled.dormant, "no live session until first focus connects");
        assert_eq!(
            titled.content_width,
            ChatContentWidth::Reading,
            "per-pane reading-width mode must round-trip"
        );
        assert_eq!(
            titled.tail,
            PaneChoice::Chosen(TailWindow::Last(3)),
            "per-pane tail window must round-trip as the user's own choice"
        );
        assert_eq!(
            titled.display_filter,
            PaneChoice::Chosen(DisplayFilter::default().toggled(FilterFacet::Prose)),
            "per-pane display filter must round-trip as the user's own choice"
        );
        assert_eq!(
            titled.fold_mode,
            Some(FoldPreset::Summary.mode()),
            "per-pane fold mode must round-trip as the user's own choice"
        );

        let codex = views
            .iter()
            .find(|v| v.agent_id == "codex")
            .expect("restored codex-owned pane present");
        assert_eq!(
            codex.session_id.as_deref(),
            Some("sess-codex-1"),
            "session id is kept when the owning agent is present (resume valid)"
        );
    });

    // Restore into a fresh workspace whose catalog is the default (claude only) —
    // `codex` was removed. `Workspace::new_with_project_for_test` seeds `agents`
    // from `config`, so no override is needed.
    let fallback_handle = cx.add_window(|window, cx| {
        let mut ws = Workspace::new_with_project_for_test(
            &config,
            None,
            super::fresh_test_data_dir(),
            window,
            cx,
        );
        ws.restore_from_disk(&workspace_state, &project_states, window, cx);
        ws
    });
    let fallback = fallback_handle.root(cx).unwrap();

    fallback.read_with(cx, |ws, cx| {
        let default_id = daruda_config::AgentDefinition::claude_default().id;
        let views = restored_chats(ws, cx);
        assert!(
            views.iter().all(|v| v.agent_id == default_id),
            "every restored agent-chat pane falls back to or keeps the default agent"
        );
        let kept = views
            .iter()
            .find(|v| v.session_id.as_deref() == Some("sess-restore-123"))
            .expect("the default-agent session keeps its persisted id");
        assert_eq!(kept.title.as_deref(), Some("Investigate flaky test"));
        assert_eq!(kept.content_width, ChatContentWidth::Reading);
        assert_eq!(kept.tail, PaneChoice::Chosen(TailWindow::Last(3)));
        assert_eq!(
            kept.display_filter,
            PaneChoice::Chosen(DisplayFilter::default().toggled(FilterFacet::Prose))
        );
        assert_eq!(kept.fold_mode, Some(FoldPreset::Summary.mode()));
        assert!(
            views
                .iter()
                .any(|v| v.session_id.is_none() && v.title.is_none()),
            "session id is dropped when its owning agent is absent (resume invalid)"
        );
    });

    let _ = std::fs::remove_dir_all(&project_root);
}

/// A second agent entry so open/switch/split tests have a non-default agent to
/// pick. `claude_default()` is catalog[0] (the session default); `codex` is the
/// distinct target.
fn codex() -> daruda_config::AgentDefinition {
    daruda_config::AgentDefinition {
        id: "codex".to_string(),
        name: "Codex".to_string(),
        launch: daruda_config::AgentLaunch::Raw("codex-acp".to_string()),
        default_mode: None,
        default_model: None,
        fold_mode: None,
        tail_window: None,
        display_filter: None,
    }
}

/// The inaccessible-lane guard in `open_agent_chat_pane_with_agent` fires
/// *before* it records the agent choice: a rejected open adds no pane and leaves
/// `last_agent_id` untouched (so a stale/inaccessible state can't poison the
/// session default).
#[gpui::test]
async fn open_with_agent_guards_before_recording_last_id(cx: &mut TestAppContext) {
    use crate::lane::availability::LaneAvailability;

    let config = daruda_config::Config::default();
    let root = std::env::temp_dir().join("daruda_agent_open_guard");
    let _ = std::fs::create_dir_all(&root);
    let project = daruda_store::project::Project::from_path(&root);
    let (window_handle, workspace) = super::build_workspace_with(cx, &config, Some(project));
    cx.run_until_parked();

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            // Force the active lane inaccessible → the empty-state; the open must
            // be rejected before it mutates `last_agent_id`.
            ws.active_lane_mut().expect("active lane").availability = LaneAvailability::Missing;
            assert!(ws.active_lane_is_inaccessible());
            assert_eq!(ws.last_agent_id, None, "no prior choice recorded");
            let panes_before = ws.active_runtime().panes.len();

            ws.open_agent_chat_pane_with_agent("codex".to_string(), window, cx);

            assert_eq!(
                ws.active_runtime().panes.len(),
                panes_before,
                "an inaccessible lane rejects the open — no pane added"
            );
            assert_eq!(
                ws.last_agent_id, None,
                "the guard fires before recording, so last_agent_id stays untouched"
            );
        });
    })
    .unwrap();
    let _ = std::fs::remove_dir_all(&root);
}

/// Switching agents opens a *new* pane under the target agent and leaves the
/// source pane + its (offline) session untouched — the backend swap is a fresh
/// conversation, never a reuse of the existing session. Splitting that focused
/// pane then inherits the source agent, rather than resetting to the catalog
/// default.
#[gpui::test]
async fn switch_agent_preserves_source_and_split_inherits_agent(cx: &mut TestAppContext) {
    use crate::workspace::main_area::pane_tree::SplitDirection;
    use crate::workspace::main_area::tab_ops::NewPaneKind;

    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    let tmp = std::env::temp_dir();
    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            // Two-entry catalog: claude (default) + codex (switch target).
            ws.agents = vec![daruda_config::AgentDefinition::claude_default(), codex()];
            let claude_id = daruda_config::AgentDefinition::claude_default().id;

            // The original agent-chat pane, chatting under the default agent.
            let src = ws.create_agent_chat_pane(
                Some(PaneCwd::Local(tmp.clone())),
                None,
                claude_id.clone(),
                None,
                window,
                cx,
            );
            let src_id = src.id;
            ws.active_runtime_mut().panes.push(src);
            let panes_before = ws.active_runtime().panes.len();

            // Switch to codex — opens a fresh pane (no project → cwd None → the
            // new pane parks in `Error`, so its lazy connect is skipped: offline).
            ws.open_agent_chat_pane_with_agent("codex".to_string(), window, cx);

            assert_eq!(
                ws.active_runtime().panes.len(),
                panes_before + 1,
                "the switch appends a new pane, it does not replace the source"
            );

            // The source pane and its session survive untouched.
            let src_view = agent_view(ws, src_id);
            let src_view = src_view.read(cx);
            assert_eq!(
                src_view.agent_id, claude_id,
                "the source pane keeps chatting under its own agent"
            );
            assert!(src_view.handle.is_none(), "the source session is untouched");

            // The new pane runs under the target agent.
            let new_id = ws
                .active_runtime()
                .panes
                .last()
                .expect("the switch pushed a pane")
                .id;
            assert_ne!(new_id, src_id, "a distinct pane was opened");
            assert_eq!(
                agent_view(ws, new_id).read(cx).agent_id,
                "codex",
                "the new pane runs under the switched-to agent"
            );

            ws.split_focused_pane_kind(
                NewPaneKind::AgentChat,
                SplitDirection::Horizontal,
                window,
                cx,
            );

            let split_id = ws
                .active_runtime()
                .panes
                .last()
                .expect("the split pushed a pane")
                .id;
            assert_ne!(split_id, new_id, "the split created a distinct pane");
            assert_eq!(
                agent_view(ws, split_id).read(cx).agent_id,
                "codex",
                "the split inherits the source pane's agent, not the catalog default"
            );
        });
    })
    .unwrap();
}

/// The ↑ consume-predicate `history_navigate_possible`: a non-empty queue on the
/// focused agent pane makes ↑ consumable even with no lane input history (so the
/// key reaches `do_history_navigate` and begins the edit); an empty queue with no
/// history does not consume ↑, and Down is never affected by the queue.
#[gpui::test]
async fn up_arrow_consumes_queue_and_edits_last_prompt(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();
    let tmp = std::env::temp_dir();

    let pane_id = cx
        .update_window(window_handle.into(), |_, window, cx| {
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
                ws.active_runtime_mut().focused_pane_id = id;
                id
            })
        })
        .unwrap();
    cx.run_until_parked();

    // Empty queue, no history → ↑ falls through to cursor movement.
    workspace.read_with(cx, |ws, cx| {
        assert!(
            !ws.history_navigate_possible(crate::ui::HistoryDir::Up, cx),
            "no queue and no history → ↑ is not consumed"
        );
        assert!(!ws.history_navigate_possible(crate::ui::HistoryDir::Down, cx));
    });

    // Enqueue offline (no handle → buffered, not sent); no input history is
    // recorded by this path.
    let last_id = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                ws.send_agent_prompt_text(pane_id, "q1".into(), cx);
                ws.send_agent_prompt_text(pane_id, "q2".into(), cx);
                let last = agent_view(ws, pane_id)
                    .read(cx)
                    .queue
                    .pending_prompts
                    .last()
                    .expect("queue non-empty")
                    .id;
                ws.terminal_input
                    .update(cx, |s, cx_state| s.set_value("typing", window, cx_state));
                last
            })
        })
        .unwrap();
    cx.run_until_parked();

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.do_history_navigate(crate::ui::HistoryDir::Up, window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();

    workspace.read_with(cx, |ws, cx| {
        assert_eq!(
            ws.terminal_input.read(cx).value(),
            "typing",
            "a non-empty composer is left for history recall, not a queue edit"
        );
        assert!(
            agent_view(ws, pane_id)
                .read(cx)
                .queue
                .editing_prompt
                .is_none(),
            "no queue edit begins when the composer is non-empty"
        );
    });

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.terminal_input
                .update(cx, |s, cx_state| s.set_value("", window, cx_state));
        });
    })
    .unwrap();

    workspace.read_with(cx, |ws, cx| {
        assert!(
            ws.history_navigate_possible(crate::ui::HistoryDir::Up, cx),
            "a queued prompt makes Up consumable even without input history"
        );
        assert!(
            !ws.history_navigate_possible(crate::ui::HistoryDir::Down, cx),
            "Down is unaffected by the queue"
        );
    });

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.do_history_navigate(crate::ui::HistoryDir::Up, window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();

    workspace.read_with(cx, |ws, cx| {
        assert_eq!(
            ws.terminal_input.read(cx).value(),
            "q2",
            "the composer receives the last queued prompt's text"
        );
        assert_eq!(
            agent_view(ws, pane_id).read(cx).queue.editing_prompt,
            Some(last_id),
            "the editing flag targets the last queued prompt"
        );
    });
}

/// Queue edit exit paths clear the editing flag and composer. Cancel leaves the
/// queued prompt intact; clear-all deletes it; whitespace submit cancels the
/// edit rather than sending an empty prompt.
#[gpui::test]
async fn queued_prompt_edit_exit_paths_clear_state(cx: &mut TestAppContext) {
    use crate::workspace::main_area::pane_input_ops::{PaneTextInput, PaneTextIntent};

    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();
    let tmp = std::env::temp_dir();

    let (pane_id, id) = cx
        .update_window(window_handle.into(), |_, window, cx| {
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
                ws.active_runtime_mut().focused_pane_id = id;
                ws.send_agent_prompt_text(id, "editable".into(), cx);
                let prompt_id = agent_view(ws, id).read(cx).queue.pending_prompts[0].id;
                ws.begin_edit_queued_prompt(id, prompt_id, window, cx);
                (id, prompt_id)
            })
        })
        .unwrap();
    cx.run_until_parked();

    // begin pulled the text into the composer and set the editing flag.
    workspace.read_with(cx, |ws, cx| {
        assert_eq!(ws.terminal_input.read(cx).value(), "editable");
        assert_eq!(
            agent_view(ws, pane_id).read(cx).queue.editing_prompt,
            Some(id)
        );
    });

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.cancel_edit_queued_prompt(pane_id, window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();

    workspace.read_with(cx, |ws, cx| {
        assert_eq!(
            ws.terminal_input.read(cx).value(),
            "",
            "cancel empties the composer"
        );
        assert!(
            agent_view(ws, pane_id)
                .read(cx)
                .queue
                .editing_prompt
                .is_none(),
            "cancel clears the editing flag; the prompt stays queued"
        );
        assert_eq!(
            queue_texts(agent_view(ws, pane_id).read(cx)),
            vec!["editable".to_string()],
            "cancel leaves the queued prompt intact"
        );
    });

    let clear_id = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                ws.send_agent_prompt_text(pane_id, "q2".into(), cx);
                let last = agent_view(ws, pane_id)
                    .read(cx)
                    .queue
                    .pending_prompts
                    .last()
                    .unwrap()
                    .id;
                ws.begin_edit_queued_prompt(pane_id, last, window, cx);
                last
            })
        })
        .unwrap();
    cx.run_until_parked();

    workspace.read_with(cx, |ws, cx| {
        assert_eq!(ws.terminal_input.read(cx).value(), "q2");
        assert_eq!(
            agent_view(ws, pane_id).read(cx).queue.editing_prompt,
            Some(clear_id)
        );
    });

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| ws.clear_queued_prompts(pane_id, window, cx));
    })
    .unwrap();
    cx.run_until_parked();

    workspace.read_with(cx, |ws, cx| {
        let view = agent_view(ws, pane_id);
        assert!(
            view.read(cx).queue.pending_prompts.is_empty(),
            "queue cleared"
        );
        assert!(
            view.read(cx).queue.editing_prompt.is_none(),
            "editing flag cleared"
        );
        assert_eq!(
            ws.terminal_input.read(cx).value(),
            "",
            "the orphaned edit text is cleared from the composer"
        );
    });

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.send_agent_prompt_text(pane_id, "editable".into(), cx);
            let pid = agent_view(ws, pane_id).read(cx).queue.pending_prompts[0].id;
            ws.begin_edit_queued_prompt(pane_id, pid, window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();

    // Submit a whitespace-only body while the edit is in progress.
    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.deliver_text_to_pane(
                pane_id,
                PaneTextInput {
                    body: "   ".into(),
                    intent: PaneTextIntent::Command { submit: true },
                },
                window,
                cx,
            )
        });
    })
    .unwrap();
    cx.run_until_parked();

    workspace.read_with(cx, |ws, cx| {
        let view = agent_view(ws, pane_id);
        assert!(
            view.read(cx).queue.editing_prompt.is_none(),
            "whitespace submit cancels the edit"
        );
        assert_eq!(
            queue_texts(view.read(cx)),
            vec!["editable".to_string()],
            "the queued prompt is left intact (edit cancelled, not sent)"
        );
        assert_eq!(
            ws.terminal_input.read(cx).value(),
            "",
            "the composer is emptied"
        );
    });
}

/// A diff block's "open in file view" / "open externally" actions must
/// refuse a pane whose session is remote (`PaneCwd::Remote`) rather than
/// reading `Diff.path` — a remote-host path — off *this* machine's local
/// disk, which would either fail or silently show an unrelated local file at
/// the same absolute path.
#[gpui::test]
async fn diff_actions_on_a_remote_pane_report_an_error_instead_of_reading_local_disk(
    cx: &mut TestAppContext,
) {
    use crate::surface::strings as s;

    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    let pane_id = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let pane = ws.create_agent_chat_pane(
                    Some(PaneCwd::Remote("host:/repo/lane".to_string())),
                    None,
                    daruda_config::AgentDefinition::claude_default().id,
                    None,
                    window,
                    cx,
                );
                let pane_id = pane.id;
                ws.active_runtime_mut().panes.push(pane);
                pane_id
            })
        })
        .unwrap();

    let errors_before = workspace.read_with(cx, |ws, _| ws.error_history().len());
    let tabs_before = workspace.read_with(cx, |ws, _| ws.active_runtime().tabs.len());

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.open_diff_in_file_view(
                pane_id,
                std::path::PathBuf::from("/repo/lane/src/main.rs"),
                window,
                cx,
            );
            ws.open_diff_externally(
                pane_id,
                std::path::PathBuf::from("/repo/lane/src/main.rs"),
                cx,
            );
        });
    })
    .unwrap();

    workspace.read_with(cx, |ws, _| {
        assert_eq!(
            ws.active_runtime().tabs.len(),
            tabs_before,
            "must not open a file-viewer tab for a remote pane's path"
        );
        assert_eq!(
            ws.error_history().len(),
            errors_before + 2,
            "both actions must report an error, one each"
        );
        for report in &ws.error_history()[..2] {
            assert_eq!(report.title, s::diff_remote_path_unsupported());
        }
    });
}

/// A live `[agent]` edit must reach an already-open pane, not wait for the next
/// restore: a pane the user never touched follows config, a pane they chose for
/// keeps its own settings.
#[gpui::test]
async fn a_config_reload_moves_an_untouched_panes_transcript_settings(cx: &mut TestAppContext) {
    use crate::workspace::main_area::pane::TabEntry;
    use crate::workspace::main_area::pane_tree::PaneLayout;

    let before = daruda_config::Config::default();
    let project_root = std::env::temp_dir().join(format!(
        "daruda_agent_chat_live_reseed_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&project_root);
    let _ = std::fs::create_dir_all(&project_root);
    let project = daruda_store::project::Project::from_path(&project_root);

    let (window_handle, workspace) = super::build_workspace_with(cx, &before, Some(project));
    cx.run_until_parked();

    let (untouched_id, chosen_id) = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let cwd = ws.active_lane().map(|w| PaneCwd::Local(w.path.clone()));
                let open = |ws: &mut Workspace, window: &mut gpui::Window, cx: &mut _| {
                    let pane = ws.create_agent_chat_pane(
                        cwd.clone(),
                        None,
                        daruda_config::AgentDefinition::claude_default().id,
                        None,
                        window,
                        cx,
                    );
                    let pane_id = pane.id;
                    ws.active_runtime_mut().panes.push(pane);
                    let tab_id = ws.alloc_id();
                    ws.active_runtime_mut().tabs.push(TabEntry {
                        id: tab_id,
                        layout: PaneLayout::Pane(pane_id),
                        last_focused_pane: pane_id,
                        user_label: None,
                    });
                    pane_id
                };
                let untouched_id = open(ws, window, cx);
                let chosen_id = open(ws, window, cx);
                (untouched_id, chosen_id)
            })
        })
        .unwrap();

    // The second pane's user makes an explicit choice on all three settings.
    let chosen = workspace.read_with(cx, |ws, _| agent_view(ws, chosen_id));
    cx.update_window(window_handle.into(), |_, window, cx| {
        chosen.update(cx, |v, cx| {
            v.set_tail_window(TailWindow::Last(2), cx);
            v.toggle_display_facet(FilterFacet::Prose, cx);
            v.set_fold_mode(FoldPreset::Expanded.mode(), window, cx);
        });
    })
    .expect("the test window is live");
    cx.run_until_parked();

    let mut after = daruda_config::Config::default();
    after.agent.tail_window = 5;
    after.agent.fold_mode = vec!["summary".to_string()];
    assert_ne!(FoldPreset::Summary.mode(), FoldMode::default());

    cx.update_window(window_handle.into(), |_, _window, cx| {
        workspace.update(cx, |ws, cx| ws.reload_config(&after, cx));
    })
    .unwrap();
    cx.run_until_parked();

    let untouched = workspace.read_with(cx, |ws, _| agent_view(ws, untouched_id));
    untouched.read_with(cx, |v, _| {
        assert_eq!(
            v.tail,
            PaneChoice::Seeded(TailWindow::Last(5)),
            "a reloaded tail window must reach an untouched pane"
        );
        assert_eq!(
            v.display_filter,
            PaneChoice::Seeded(DisplayFilter::default()),
            "the filter is a per-pane act, so no reload can narrow an untouched pane"
        );
        assert_eq!(
            v.fold.mode(),
            FoldPreset::Summary.mode(),
            "a reloaded fold mode must reach an untouched pane"
        );
        assert_eq!(v.fold.chosen_mode(), None, "and it is still only a seed");
    });

    chosen.read_with(cx, |v, _| {
        assert_eq!(
            v.tail,
            PaneChoice::Chosen(TailWindow::Last(2)),
            "config must not overwrite a chosen tail window"
        );
        assert_eq!(
            v.display_filter,
            PaneChoice::Chosen(DisplayFilter::default().toggled(FilterFacet::Prose)),
            "config must not overwrite a chosen display filter"
        );
        assert_eq!(
            v.fold.chosen_mode(),
            Some(FoldPreset::Expanded.mode()),
            "config must not overwrite a chosen fold mode"
        );
    });

    let _ = std::fs::remove_dir_all(&project_root);
}

/// The two agent ids the per-agent transcript tests below run their panes on.
const TRANSCRIPT_AGENT_A: &str = "transcript-agent-a";
const TRANSCRIPT_AGENT_B: &str = "transcript-agent-b";

/// One `[[agents]]` row stating all three transcript axes. Callers give the two
/// agents disagreeing values on every axis, so a pane on one can never be
/// mistaken for a pane on the other. The id doubles as the name: these entries
/// exist only to be told apart.
fn transcript_agent_entry(
    id: &str,
    tail_window: u8,
    fold_mode: &str,
    filter: FilterFacet,
) -> daruda_config::AgentEntry {
    daruda_config::AgentEntry::Custom(daruda_config::AgentDefinition {
        id: id.to_string(),
        name: id.to_string(),
        fold_mode: Some(vec![fold_mode.to_string()]),
        tail_window: Some(tail_window),
        display_filter: Some(vec![filter.token().to_string()]),
        ..daruda_config::AgentDefinition::claude_default()
    })
}

/// Open a chat pane on `agent_id` in the active runtime, in its own tab, and
/// return its id.
fn open_agent_chat_pane_on(
    ws: &mut Workspace,
    agent_id: &str,
    window: &mut gpui::Window,
    cx: &mut gpui::Context<Workspace>,
) -> PaneId {
    use crate::workspace::main_area::pane::TabEntry;
    use crate::workspace::main_area::pane_tree::PaneLayout;

    let cwd = ws.active_lane().map(|w| PaneCwd::Local(w.path.clone()));
    let pane = ws.create_agent_chat_pane(cwd, None, agent_id.to_string(), None, window, cx);
    let pane_id = pane.id;
    ws.active_runtime_mut().panes.push(pane);
    let tab_id = ws.alloc_id();
    ws.active_runtime_mut().tabs.push(TabEntry {
        id: tab_id,
        layout: PaneLayout::Pane(pane_id),
        last_focused_pane: pane_id,
        user_label: None,
    });
    pane_id
}

/// The three transcript axes a pane currently sits on, as one tuple: the tests
/// below compare two panes against each other as well as against config.
fn transcript_settings(
    view: &Entity<AgentChatView>,
    cx: &mut TestAppContext,
) -> (PaneChoice<TailWindow>, FoldMode, PaneChoice<DisplayFilter>) {
    view.read_with(cx, |v, _| (v.tail, v.fold.mode(), v.display_filter))
}

/// A reload must resolve the defaults per pane, against that pane's own agent.
/// Two agents are what makes this observable: with one agent in the window,
/// resolving once for the whole window and resolving per pane land every pane
/// on the same values, so only disagreeing agents can tell the two apart.
#[gpui::test]
async fn a_config_reload_moves_each_pane_to_its_own_agents_transcript_settings(
    cx: &mut TestAppContext,
) {
    let before = daruda_config::Config {
        agents: vec![
            transcript_agent_entry(TRANSCRIPT_AGENT_A, 3, "expanded", FilterFacet::Thinking),
            transcript_agent_entry(TRANSCRIPT_AGENT_B, 7, "summary", FilterFacet::Tools),
        ],
        ..daruda_config::Config::default()
    };
    let project_root = std::env::temp_dir().join(format!(
        "daruda_agent_chat_per_agent_reseed_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&project_root);
    let _ = std::fs::create_dir_all(&project_root);
    let project = daruda_store::project::Project::from_path(&project_root);

    let (window_handle, workspace) = super::build_workspace_with(cx, &before, Some(project));
    cx.run_until_parked();

    let (pane_a, pane_b) = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                (
                    open_agent_chat_pane_on(ws, TRANSCRIPT_AGENT_A, window, cx),
                    open_agent_chat_pane_on(ws, TRANSCRIPT_AGENT_B, window, cx),
                )
            })
        })
        .unwrap();

    // Every axis of both agents changes, and the two stay disagreeing.
    let after = daruda_config::Config {
        agents: vec![
            transcript_agent_entry(TRANSCRIPT_AGENT_A, 2, "summary", FilterFacet::Tools),
            transcript_agent_entry(TRANSCRIPT_AGENT_B, 6, "expanded", FilterFacet::Thinking),
        ],
        ..daruda_config::Config::default()
    };
    cx.update_window(window_handle.into(), |_, _window, cx| {
        workspace.update(cx, |ws, cx| ws.reload_config(&after, cx));
    })
    .unwrap();
    cx.run_until_parked();

    let view_a = workspace.read_with(cx, |ws, _| agent_view(ws, pane_a));
    let view_b = workspace.read_with(cx, |ws, _| agent_view(ws, pane_b));
    let (tail_a, fold_a, filter_a) = transcript_settings(&view_a, cx);
    let (tail_b, fold_b, filter_b) = transcript_settings(&view_b, cx);

    assert_eq!(
        tail_a,
        PaneChoice::Seeded(TailWindow::Last(2)),
        "the first pane must land on its own agent's reloaded tail window"
    );
    assert_eq!(
        fold_a,
        FoldPreset::Summary.mode(),
        "the first pane must land on its own agent's reloaded fold mode"
    );
    assert_eq!(
        filter_a,
        PaneChoice::Seeded(DisplayFilter::from_tokens([FilterFacet::Tools.token()])),
        "the first pane must land on its own agent's reloaded display filter"
    );

    assert_eq!(
        tail_b,
        PaneChoice::Seeded(TailWindow::Last(6)),
        "the second pane must land on its own agent's reloaded tail window"
    );
    assert_eq!(
        fold_b,
        FoldPreset::Expanded.mode(),
        "the second pane must land on its own agent's reloaded fold mode"
    );
    assert_eq!(
        filter_b,
        PaneChoice::Seeded(DisplayFilter::from_tokens([FilterFacet::Thinking.token()])),
        "the second pane must land on its own agent's reloaded display filter"
    );

    assert_ne!(
        tail_a, tail_b,
        "one resolve for the window would tie the two panes' tail windows together"
    );
    assert_ne!(
        fold_a, fold_b,
        "one resolve for the window would tie the two panes' fold modes together"
    );
    assert_ne!(
        filter_a, filter_b,
        "one resolve for the window would tie the two panes' display filters together"
    );

    let _ = std::fs::remove_dir_all(&project_root);
}

/// A pane whose owning agent was removed falls back at reconnect time. That id
/// rewrite must move untouched transcript axes to the fallback agent's own
/// defaults, not leave them on the global defaults used while the id was stale.
#[gpui::test]
async fn stale_agent_reconnect_reseeds_to_the_fallback_agents_transcript_settings(
    cx: &mut TestAppContext,
) {
    let before = daruda_config::Config {
        agents: vec![transcript_agent_entry(
            TRANSCRIPT_AGENT_A,
            3,
            "expanded",
            FilterFacet::Thinking,
        )],
        ..daruda_config::Config::default()
    };
    let project_root = std::env::temp_dir().join(format!(
        "daruda_agent_chat_stale_agent_reseed_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&project_root);
    let _ = std::fs::create_dir_all(&project_root);
    let project = daruda_store::project::Project::from_path(&project_root);

    let (window_handle, workspace) = super::build_workspace_with(cx, &before, Some(project));
    cx.run_until_parked();

    let pane_id = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                open_agent_chat_pane_on(ws, TRANSCRIPT_AGENT_A, window, cx)
            })
        })
        .unwrap();

    let mut after = daruda_config::Config {
        agents: vec![transcript_agent_entry(
            TRANSCRIPT_AGENT_B,
            6,
            "summary",
            FilterFacet::Tools,
        )],
        ..daruda_config::Config::default()
    };
    after.agent.tail_window = 9;
    after.agent.fold_mode = vec!["expanded".to_string()];
    after.agent.display_filter = Some(vec![FilterFacet::Thinking.token().to_string()]);

    cx.update_window(window_handle.into(), |_, _window, cx| {
        workspace.update(cx, |ws, cx| ws.reload_config(&after, cx));
    })
    .unwrap();
    cx.run_until_parked();

    let view = workspace.read_with(cx, |ws, _| agent_view(ws, pane_id));
    assert_eq!(
        transcript_settings(&view, cx),
        (
            PaneChoice::Seeded(TailWindow::Last(9)),
            FoldPreset::Expanded.mode(),
            PaneChoice::Seeded(DisplayFilter::from_tokens([FilterFacet::Thinking.token()])),
        ),
        "while the id is stale, reload can only fall back to the global defaults"
    );

    workspace.update(cx, |ws, cx| {
        ws.resolve_pane_launch_for_test(pane_id, cx)
            .expect("the fallback agent is launchable");
    });

    let view = workspace.read_with(cx, |ws, _| agent_view(ws, pane_id));
    let agent_id = view.read_with(cx, |v, _| v.agent_id.clone());
    assert_eq!(agent_id, TRANSCRIPT_AGENT_B);
    assert_eq!(
        transcript_settings(&view, cx),
        (
            PaneChoice::Seeded(TailWindow::Last(6)),
            FoldPreset::Summary.mode(),
            PaneChoice::Seeded(DisplayFilter::from_tokens([FilterFacet::Tools.token()])),
        ),
        "the reconnect fallback must reseed against the agent it now runs"
    );

    let _ = std::fs::remove_dir_all(&project_root);
}

/// A new pane is born on its own agent's transcript defaults, not on whichever
/// agent the catalog lists first. As with the reload above, a single agent
/// cannot show the difference — both panes would be born identical either way.
#[gpui::test]
async fn a_new_pane_is_born_on_its_own_agents_transcript_settings(cx: &mut TestAppContext) {
    let config = daruda_config::Config {
        agents: vec![
            transcript_agent_entry(TRANSCRIPT_AGENT_A, 3, "expanded", FilterFacet::Thinking),
            transcript_agent_entry(TRANSCRIPT_AGENT_B, 7, "summary", FilterFacet::Tools),
        ],
        ..daruda_config::Config::default()
    };
    let project_root = std::env::temp_dir().join(format!(
        "daruda_agent_chat_per_agent_birth_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&project_root);
    let _ = std::fs::create_dir_all(&project_root);
    let project = daruda_store::project::Project::from_path(&project_root);

    let (window_handle, workspace) = super::build_workspace_with(cx, &config, Some(project));
    cx.run_until_parked();

    let (pane_a, pane_b) = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                (
                    open_agent_chat_pane_on(ws, TRANSCRIPT_AGENT_A, window, cx),
                    open_agent_chat_pane_on(ws, TRANSCRIPT_AGENT_B, window, cx),
                )
            })
        })
        .unwrap();
    cx.run_until_parked();

    let view_a = workspace.read_with(cx, |ws, _| agent_view(ws, pane_a));
    let view_b = workspace.read_with(cx, |ws, _| agent_view(ws, pane_b));
    let (tail_a, fold_a, filter_a) = transcript_settings(&view_a, cx);
    let (tail_b, fold_b, filter_b) = transcript_settings(&view_b, cx);

    assert_eq!(
        tail_a,
        PaneChoice::Seeded(TailWindow::Last(3)),
        "the first pane must be born on its own agent's tail window"
    );
    assert_eq!(
        fold_a,
        FoldPreset::Expanded.mode(),
        "the first pane must be born on its own agent's fold mode"
    );
    assert_eq!(
        filter_a,
        PaneChoice::Seeded(DisplayFilter::from_tokens([FilterFacet::Thinking.token()])),
        "the first pane must be born on its own agent's display filter"
    );

    assert_eq!(
        tail_b,
        PaneChoice::Seeded(TailWindow::Last(7)),
        "the second pane must be born on its own agent's tail window"
    );
    assert_eq!(
        fold_b,
        FoldPreset::Summary.mode(),
        "the second pane must be born on its own agent's fold mode"
    );
    assert_eq!(
        filter_b,
        PaneChoice::Seeded(DisplayFilter::from_tokens([FilterFacet::Tools.token()])),
        "the second pane must be born on its own agent's display filter"
    );

    assert_ne!(
        tail_a, tail_b,
        "resolving against one catalog entry would tie the two panes' tail windows together"
    );
    assert_ne!(
        fold_a, fold_b,
        "resolving against one catalog entry would tie the two panes' fold modes together"
    );
    assert_ne!(
        filter_a, filter_b,
        "resolving against one catalog entry would tie the two panes' display filters together"
    );

    let _ = std::fs::remove_dir_all(&project_root);
}

#[gpui::test]
async fn an_untouched_pane_keeps_following_the_config_defaults(cx: &mut TestAppContext) {
    use crate::workspace::main_area::pane::TabEntry;
    use crate::workspace::main_area::pane_tree::PaneLayout;

    let before = daruda_config::Config::default();
    let project_root = std::env::temp_dir().join(format!(
        "daruda_agent_chat_untouched_defaults_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&project_root);
    let _ = std::fs::create_dir_all(&project_root);
    let project = daruda_store::project::Project::from_path(&project_root);

    let (window_handle, workspace) = super::build_workspace_with(cx, &before, Some(project));
    cx.run_until_parked();

    let (workspace_state, project_states) = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let cwd = ws.active_lane().map(|w| PaneCwd::Local(w.path.clone()));
                let pane = ws.create_agent_chat_pane(
                    cwd,
                    None,
                    daruda_config::AgentDefinition::claude_default().id,
                    None,
                    window,
                    cx,
                );
                let pane_id = pane.id;
                ws.active_runtime_mut().panes.push(pane);
                let tab_id = ws.alloc_id();
                ws.active_runtime_mut().tabs.push(TabEntry {
                    id: tab_id,
                    layout: PaneLayout::Pane(pane_id),
                    last_focused_pane: pane_id,
                    user_label: None,
                });
                ws.snapshot_for_disk(cx).expect("snapshot")
            })
        })
        .unwrap();

    let leaf_json = serde_json::to_string(&project_states).expect("serialize project states");
    for key in ["tail_window", "display_filter", "fold_mode"] {
        assert!(
            !leaf_json.contains(key),
            "{key} must stay out of an untouched pane's state: {leaf_json}"
        );
    }

    let mut after = daruda_config::Config::default();
    after.agent.tail_window = 5;
    after.agent.fold_mode = vec!["summary".to_string()];

    let restored_handle = cx.add_window(|window, cx| {
        let mut ws = Workspace::new_with_project_for_test(
            &after,
            None,
            super::fresh_test_data_dir(),
            window,
            cx,
        );
        ws.restore_from_disk(&workspace_state, &project_states, window, cx);
        ws
    });
    let restored = restored_handle.root(cx).unwrap();

    restored.read_with(cx, |ws, cx| {
        let view = ws
            .active_runtime()
            .panes
            .iter()
            .find_map(|p| p.agent_chat_view())
            .expect("restored agent chat pane present")
            .read(cx);
        assert_eq!(
            view.tail,
            PaneChoice::Seeded(TailWindow::Last(5)),
            "the new config tail window must reach an untouched pane"
        );
        assert_eq!(
            view.display_filter,
            PaneChoice::Seeded(DisplayFilter::default()),
            "a restored pane opens unfiltered — no config key narrows it"
        );
        assert_eq!(
            view.fold.mode(),
            FoldPreset::Summary.mode(),
            "the new config fold mode must reach an untouched pane"
        );
        assert_eq!(
            view.fold.chosen_mode(),
            None,
            "and it is still the seed, not a choice"
        );
        assert_ne!(FoldPreset::Summary.mode(), FoldMode::default());
    });

    let _ = std::fs::remove_dir_all(&project_root);
}

/// A `Model`-category select advertising `choices`, plus a `ThoughtLevel`
/// select that must never be mistaken for the model axis.
fn model_options(choices: &[(&str, &str)]) -> Vec<daruda_acp::ConfigOptionView> {
    use daruda_acp::{
        ConfigChoiceView, ConfigOptionCategoryView, ConfigOptionKindView, ConfigOptionView,
    };
    vec![
        ConfigOptionView {
            id: "model".into(),
            name: "Model".into(),
            description: None,
            category: ConfigOptionCategoryView::Model,
            kind: ConfigOptionKindView::Select {
                current_value: choices[0].0.to_string(),
                options: choices
                    .iter()
                    .map(|(value, name)| ConfigChoiceView {
                        value: (*value).to_string(),
                        name: (*name).to_string(),
                        description: None,
                    })
                    .collect(),
            },
        },
        ConfigOptionView {
            id: "effort".into(),
            name: "Effort".into(),
            description: None,
            category: ConfigOptionCategoryView::ThoughtLevel,
            kind: ConfigOptionKindView::Select {
                current_value: "high".into(),
                options: vec![ConfigChoiceView {
                    value: "high".into(),
                    name: "High".into(),
                    description: None,
                }],
            },
        },
    ]
}

#[gpui::test]
async fn connect_and_later_events_replace_only_their_vocabulary_axis(cx: &mut TestAppContext) {
    use daruda_acp::{AcpEvent, ModeStateView, SessionCapabilitiesView, SessionModeView};
    use daruda_store::agent_vocabulary::{AgentVocabularyCache, VocabEntry};

    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    let agent = daruda_config::AgentDefinition::claude_default();
    let source = crate::lane::session_host::adapter_command(&agent.launch)
        .trim()
        .to_string();
    let agent_id = agent.id;
    let tmp = std::env::temp_dir();
    let pane_id = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let pane = ws.create_agent_chat_pane(
                    Some(PaneCwd::Local(tmp.clone())),
                    None,
                    agent_id.clone(),
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

    // What the adapter advertises at connect: both axes at once.
    let connected = AcpEvent::Connected {
        program: None,
        session_id: "s1".into(),
        modes: Some(ModeStateView {
            available: vec![
                SessionModeView {
                    id: "default".into(),
                    name: "Default".into(),
                    description: None,
                },
                SessionModeView {
                    id: "plan".into(),
                    name: "Plan".into(),
                    description: None,
                },
            ],
            current: "default".into(),
        }),
        config_options: model_options(&[("opus", "Opus"), ("sonnet", "Sonnet")]),
        capabilities: SessionCapabilitiesView::default(),
        login_methods: Vec::new(),
    };

    let data_dir = workspace.update(cx, |ws, cx| {
        ws.record_agent_vocabulary(pane_id, &connected, cx);
        ws.data_dir.clone()
    });

    workspace.read_with(cx, |ws, _| {
        assert_eq!(
            ws.agent_vocabulary.known_modes_for(&agent_id, &source),
            Some(
                vec![
                    VocabEntry::new("default", "Default"),
                    VocabEntry::new("plan", "Plan"),
                ]
                .as_slice()
            )
        );
        assert_eq!(
            ws.agent_vocabulary.known_models_for(&agent_id, &source),
            Some(
                vec![
                    VocabEntry::new("opus", "Opus"),
                    VocabEntry::new("sonnet", "Sonnet"),
                ]
                .as_slice()
            ),
            "only the Model-category select feeds the model axis"
        );
    });
    assert_eq!(
        AgentVocabularyCache::load_in(&data_dir).known_models_for(&agent_id, &source),
        Some(
            vec![
                VocabEntry::new("opus", "Opus"),
                VocabEntry::new("sonnet", "Sonnet"),
            ]
            .as_slice()
        ),
        "a changed vocabulary is persisted, not just cached in memory"
    );

    // A later option replacement must leave the mode axis alone.
    let changed = AcpEvent::ConfigOptionsChanged(model_options(&[("haiku", "Haiku")]));
    workspace.update(cx, |ws, cx| {
        ws.record_agent_vocabulary(pane_id, &changed, cx);
    });
    workspace.read_with(cx, |ws, _| {
        assert_eq!(
            ws.agent_vocabulary.known_models_for(&agent_id, &source),
            Some(vec![VocabEntry::new("haiku", "Haiku")].as_slice())
        );
        assert_eq!(
            ws.agent_vocabulary.known_modes_for(&agent_id, &source),
            Some(
                vec![
                    VocabEntry::new("default", "Default"),
                    VocabEntry::new("plan", "Plan"),
                ]
                .as_slice()
            ),
            "ConfigOptionsChanged replaces options only — modes are untouched"
        );
    });

    // A model switch may rebuild the advertised modes. The reconciled
    // ModeChanged state replaces that axis without disturbing models.
    let mode_changed = AcpEvent::ModeChanged {
        state: ModeStateView {
            available: vec![SessionModeView {
                id: "review".into(),
                name: "Review".into(),
                description: None,
            }],
            current: "review".into(),
        },
    };
    workspace.update(cx, |ws, cx| {
        ws.record_agent_vocabulary(pane_id, &mode_changed, cx);
    });
    workspace.read_with(cx, |ws, _| {
        assert_eq!(
            ws.agent_vocabulary.known_modes_for(&agent_id, &source),
            Some(vec![VocabEntry::new("review", "Review")].as_slice())
        );
        assert_eq!(
            ws.agent_vocabulary.known_models_for(&agent_id, &source),
            Some(vec![VocabEntry::new("haiku", "Haiku")].as_slice()),
            "ModeChanged replaces modes only — models are untouched"
        );
    });
    assert_eq!(
        AgentVocabularyCache::load_in(&data_dir).known_modes_for(&agent_id, &source),
        Some(vec![VocabEntry::new("review", "Review")].as_slice()),
        "the changed mode vocabulary is persisted"
    );
}

#[gpui::test]
async fn a_model_pick_is_remembered_and_survives_a_restore(cx: &mut TestAppContext) {
    use crate::workspace::main_area::pane::TabEntry;
    use crate::workspace::main_area::pane_tree::PaneLayout;

    let config = daruda_config::Config::default();
    let project_root = std::env::temp_dir().join(format!(
        "daruda_agent_chat_model_restore_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&project_root);
    let _ = std::fs::create_dir_all(&project_root);
    let project = daruda_store::project::Project::from_path(&project_root);

    let (window_handle, workspace) = super::build_workspace_with(cx, &config, Some(project));
    cx.run_until_parked();

    let (workspace_state, project_states) = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let cwd = ws.active_lane().map(|w| PaneCwd::Local(w.path.clone()));
                let pane = ws.create_agent_chat_pane(
                    cwd,
                    Some("sess-model-1".to_string()),
                    daruda_config::AgentDefinition::claude_default().id,
                    None,
                    window,
                    cx,
                );
                let pane_id = pane.id;
                ws.active_runtime_mut().panes.push(pane);
                let tab_id = ws.alloc_id();
                ws.active_runtime_mut().tabs.push(TabEntry {
                    id: tab_id,
                    layout: PaneLayout::Pane(pane_id),
                    last_focused_pane: pane_id,
                    user_label: None,
                });
                // Stand in for what `Connected` folds in before a chip can be
                // clicked: the advertised option set the pick lands on.
                let view = agent_view(ws, pane_id);
                view.update(cx, |v, _| {
                    v.session_config.config_options =
                        model_options(&[("opus", "Opus"), ("sonnet", "Sonnet")]);
                });

                // A non-model option must not be mistaken for the model axis.
                ws.set_agent_config_option(
                    pane_id,
                    "effort".to_string(),
                    daruda_acp::ConfigValueView::Id("high".to_string()),
                    cx,
                );
                assert_eq!(view.read(cx).last_known_model_id, None);

                ws.set_agent_config_option(
                    pane_id,
                    "model".to_string(),
                    daruda_acp::ConfigValueView::Id("sonnet".to_string()),
                    cx,
                );
                assert_eq!(
                    view.read(cx).last_known_model_id.as_deref(),
                    Some("sonnet"),
                    "the chip pick is remembered so the next connect reapplies it"
                );

                ws.snapshot_for_disk(cx).expect("snapshot")
            })
        })
        .unwrap();

    let persisted = project_states
        .iter()
        .flat_map(|p| p.lanes.iter())
        .flat_map(|l| l.tabs.iter())
        .find_map(|t| match &t.layout {
            daruda_store::project::SerializedLayout::Leaf {
                content: daruda_store::project::SerializedPaneContent::AgentChat(ac),
                ..
            } => ac.model_id.clone(),
            _ => None,
        });
    assert_eq!(
        persisted.as_deref(),
        Some("sonnet"),
        "the pick reaches the persisted pane record"
    );

    let restored_handle = cx.add_window(|window, cx| {
        let mut ws = Workspace::new_with_project_for_test(
            &config,
            None,
            super::fresh_test_data_dir(),
            window,
            cx,
        );
        ws.restore_from_disk(&workspace_state, &project_states, window, cx);
        ws
    });
    let restored = restored_handle.root(cx).unwrap();
    restored.read_with(cx, |ws, cx| {
        let view = ws
            .active_runtime()
            .panes
            .iter()
            .find_map(|p| p.agent_chat_view())
            .expect("restored agent chat pane present")
            .read(cx);
        assert_eq!(
            view.last_known_model_id.as_deref(),
            Some("sonnet"),
            "a restored pane still knows its model, so its lazy connect reapplies it"
        );
    });

    let _ = std::fs::remove_dir_all(&project_root);
}
