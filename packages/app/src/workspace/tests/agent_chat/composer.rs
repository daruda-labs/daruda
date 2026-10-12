//! The bottom dock's composer reaching a pane: sending, the queue it edits,
//! the empty-Enter resume gesture, and Stop.

use super::*;

#[gpui::test]
async fn cancel_turn_ends_the_turn_locally_without_an_agent_reply(cx: &mut TestAppContext) {
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
                // A turn mid-flight that no stop reason will ever end, as a
                // hung agent leaves it.
                view.update(cx, |v, _| v.set_turn_in_flight());
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
        // What Stop does to the items is the chat's own test
        // (`daruda_chat` `view/tests/stop.rs`); the host's part is that the turn ends.
        assert!(
            view.turn_is_idle(),
            "Stop ends the turn without an agent reply"
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
            let queue = snap
                .queued_prompts
                .as_ref()
                .expect("active agent queue is projected into the bottom snapshot");
            assert_eq!(queue.pane_id, second_id);
            assert_eq!(
                queue
                    .prompts
                    .iter()
                    .map(|q| q.text.clone())
                    .collect::<Vec<_>>(),
                vec!["active first".to_string(), "active second".to_string()],
                "only the focused agent pane's queue is projected"
            );

            ws.active_runtime_mut().focused_pane_id = first_id;
            let snap = ws.prepare_bottom_dock_snapshot(cx);
            let queue = snap
                .queued_prompts
                .as_ref()
                .expect("newly active agent queue is projected");
            assert_eq!(queue.pane_id, first_id);
            assert_eq!(
                queue
                    .prompts
                    .iter()
                    .map(|q| q.text.clone())
                    .collect::<Vec<_>>(),
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
                assert!(view.items().is_empty(), "a queued prompt is not echoed");
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
                assert!(view.items().is_empty());
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
                    .queue()
                    .last_prompt_id()
                    .expect("queue non-empty");
                ws.input_dock
                    .input
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
            ws.input_dock.input.read(cx).value(),
            "typing",
            "a non-empty composer is left for history recall, not a queue edit"
        );
        assert!(
            agent_view(ws, pane_id).read(cx).queue().editing().is_none(),
            "no queue edit begins when the composer is non-empty"
        );
    });

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.input_dock
                .input
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
            ws.input_dock.input.read(cx).value(),
            "q2",
            "the composer receives the last queued prompt's text"
        );
        assert_eq!(
            agent_view(ws, pane_id).read(cx).queue().editing(),
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
                let prompt_id = agent_view(ws, id)
                    .read(cx)
                    .queue()
                    .in_strip_order()
                    .find(|(_, parked)| !parked)
                    .expect("a queued prompt")
                    .0
                    .id();
                ws.begin_edit_queued_prompt(id, prompt_id, window, cx);
                (id, prompt_id)
            })
        })
        .unwrap();
    cx.run_until_parked();

    // begin pulled the text into the composer and set the editing flag.
    workspace.read_with(cx, |ws, cx| {
        assert_eq!(ws.input_dock.input.read(cx).value(), "editable");
        assert_eq!(agent_view(ws, pane_id).read(cx).queue().editing(), Some(id));
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
            ws.input_dock.input.read(cx).value(),
            "",
            "cancel empties the composer"
        );
        assert!(
            agent_view(ws, pane_id).read(cx).queue().editing().is_none(),
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
                    .queue()
                    .last_prompt_id()
                    .unwrap();
                ws.begin_edit_queued_prompt(pane_id, last, window, cx);
                last
            })
        })
        .unwrap();
    cx.run_until_parked();

    workspace.read_with(cx, |ws, cx| {
        assert_eq!(ws.input_dock.input.read(cx).value(), "q2");
        assert_eq!(
            agent_view(ws, pane_id).read(cx).queue().editing(),
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
        assert!(!view.read(cx).queue().has_pending(), "queue cleared");
        assert!(
            view.read(cx).queue().editing().is_none(),
            "editing flag cleared"
        );
        assert_eq!(
            ws.input_dock.input.read(cx).value(),
            "",
            "the orphaned edit text is cleared from the composer"
        );
    });

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.send_agent_prompt_text(pane_id, "editable".into(), cx);
            let pid = agent_view(ws, pane_id)
                .read(cx)
                .queue()
                .in_strip_order()
                .find(|(_, parked)| !parked)
                .expect("a queued prompt")
                .0
                .id();
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
            view.read(cx).queue().editing().is_none(),
            "whitespace submit cancels the edit"
        );
        assert_eq!(
            queue_texts(view.read(cx)),
            vec!["editable".to_string()],
            "the queued prompt is left intact (edit cancelled, not sent)"
        );
        assert_eq!(
            ws.input_dock.input.read(cx).value(),
            "",
            "the composer is emptied"
        );
    });
}

/// The empty-composer Enter gesture end to end through the Workspace funnel:
/// two presses on a parked queue resume it, and neither press reaches the
/// per-lane input history (an empty entry there would poison ↑ recall).
#[gpui::test]
async fn empty_composer_enter_twice_resumes_the_parked_queue(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    let tmp = std::env::temp_dir();
    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let pane = ws.create_agent_chat_pane(
                Some(PaneCwd::Local(tmp.clone())),
                None,
                daruda_config::AgentDefinition::claude_default().id,
                None,
                window,
                cx,
            );
            let pane_id = pane.id;
            ws.active_runtime_mut().panes.push(pane);
            ws.active_runtime_mut().focused_pane_id = pane_id;

            // Park a queue the way a Stop does: prompts buffered behind a turn,
            // then cancelled.
            let view = agent_view(ws, pane_id);
            view.update(cx, |v, _| v.set_turn_in_flight());
            ws.send_agent_prompt_text(pane_id, "one".to_string(), cx);
            ws.send_agent_prompt_text(pane_id, "two".to_string(), cx);
            view.update(cx, |v, cx| v.cancel_turn(cx));
            assert!(
                queue_texts(view.read(cx)).is_empty(),
                "the Stop moved both prompts out of the live queue"
            );

            ws.send_terminal_input(window, cx);
            assert!(
                view.read(cx).resume_armed(),
                "the first empty Enter arms the resume gesture"
            );
            assert!(
                queue_texts(view.read(cx)).is_empty(),
                "arming alone must not put anything back on the live queue"
            );

            ws.send_terminal_input(window, cx);
            assert_eq!(
                queue_texts(view.read(cx)),
                vec!["one".to_string(), "two".to_string()],
                "the second empty Enter resumed the parked queue in order"
            );

            let lane_ref = ws.active_ref();
            assert!(
                !ws.lane_scoped[&lane_ref].input_history.has_entries(),
                "an empty submit must not push an entry onto the input history"
            );
        });
    })
    .unwrap();
}

/// Only a composer that holds text ends the gesture. The composer is written
/// programmatically too — a focus swap restores the incoming pane's draft — and
/// `set_value` emits `Change` even for empty text, so an unconditional disarm
/// would cancel the arm of the pane being focused.
#[gpui::test]
async fn only_a_non_empty_composer_disarms_the_resume_gesture(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    let tmp = std::env::temp_dir();
    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let pane = ws.create_agent_chat_pane(
                Some(PaneCwd::Local(tmp.clone())),
                None,
                daruda_config::AgentDefinition::claude_default().id,
                None,
                window,
                cx,
            );
            let pane_id = pane.id;
            ws.active_runtime_mut().panes.push(pane);
            ws.active_runtime_mut().focused_pane_id = pane_id;

            let view = agent_view(ws, pane_id);
            view.update(cx, |v, _| v.set_turn_in_flight());
            ws.send_agent_prompt_text(pane_id, "parked".to_string(), cx);
            view.update(cx, |v, cx| v.cancel_turn(cx));
            ws.send_terminal_input(window, cx);
            assert!(view.read(cx).resume_armed());

            ws.disarm_queue_resume(cx);
            assert!(
                view.read(cx).resume_armed(),
                "an empty composer leaves the gesture armed"
            );

            ws.input_dock
                .input
                .update(cx, |s, cx_state| s.set_value("next", window, cx_state));
            ws.disarm_queue_resume(cx);
            assert!(
                !view.read(cx).resume_armed(),
                "typed text ends the gesture — the next Enter sends it"
            );
        });
    })
    .unwrap();
}

/// A queued-prompt edit outranks the resume gesture: clearing the composer and
/// pressing Enter reverts the "Editing…" row rather than arming, so the strip
/// never shows both states at once.
#[gpui::test]
async fn empty_composer_enter_cancels_a_queued_edit_before_arming(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    let tmp = std::env::temp_dir();
    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let pane = ws.create_agent_chat_pane(
                Some(PaneCwd::Local(tmp.clone())),
                None,
                daruda_config::AgentDefinition::claude_default().id,
                None,
                window,
                cx,
            );
            let pane_id = pane.id;
            ws.active_runtime_mut().panes.push(pane);
            ws.active_runtime_mut().focused_pane_id = pane_id;

            let view = agent_view(ws, pane_id);
            view.update(cx, |v, _| v.set_turn_in_flight());
            ws.send_agent_prompt_text(pane_id, "parked".to_string(), cx);
            view.update(cx, |v, cx| v.cancel_turn(cx));
            let parked_id = view
                .read(cx)
                .queue()
                .in_strip_order()
                .find(|(_, parked)| *parked)
                .expect("a parked prompt")
                .0
                .id();
            ws.begin_edit_queued_prompt(pane_id, parked_id, window, cx);
            // `begin_edit_queued_prompt` filled the composer; clearing it is what
            // makes the next Enter an empty submit.
            ws.input_dock
                .input
                .update(cx, |s, cx_state| s.set_value("", window, cx_state));

            ws.send_terminal_input(window, cx);
            assert!(
                view.read(cx).queue().editing().is_none(),
                "the empty Enter cancelled the edit"
            );
            assert!(
                !view.read(cx).resume_armed(),
                "cancelling an edit must not also arm the resume gesture"
            );
        });
    })
    .unwrap();
}

/// An empty composer is not a terminal command. The resume gesture reads an
/// empty submit, but only an agent pane does — routing it through the
/// pane-agnostic funnel would hand a focused terminal a bare `\r`, submitting
/// whatever the user had typed at the shell prompt itself.
#[gpui::test]
async fn empty_composer_enter_never_reaches_a_focused_terminal(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let focused = ws.active_runtime().focused_pane_id;
            assert!(
                ws.active_runtime()
                    .panes
                    .iter()
                    .any(|p| p.id == focused && p.terminal_view().is_some()),
                "a fresh workspace focuses a terminal pane"
            );
            // The funnel bumps this on every accepted delivery, so an unchanged
            // tick is what proves the terminal was never handed anything.
            let before = ws.main_area.activity_tick;

            ws.send_terminal_input(window, cx);

            assert_eq!(
                ws.main_area.activity_tick, before,
                "an empty composer delivers nothing to a focused terminal"
            );
        });
    })
    .unwrap();
}
