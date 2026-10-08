//! The tab strip's status dot: each tab shows the most urgent agent session
//! among its panes, or the outcome of a turn that ended out of view until the
//! user sees it.

use super::*;
use crate::workspace::main_area::agent_chat_pane::view::TurnOutcome;
use crate::workspace::main_area::pane_tree::PaneId;
use crate::workspace::tab_indicator::TabIndicator;
use daruda_agent::hooks::status_file::StatusFile;
use daruda_agent::{AgentOutcome, SessionStatus};

fn active_tab_status(
    ws: &gpui::Entity<Workspace>,
    cx: &mut TestAppContext,
) -> Option<TabIndicator> {
    ws.read_with(cx, |ws, cx| {
        ws.tab_cells(cx)
            .into_iter()
            .find(|cell| cell.is_active)
            .and_then(|cell| cell.indicator)
    })
}

#[gpui::test]
async fn an_agent_tab_carries_its_session_status(cx: &mut TestAppContext) {
    let (wh, ws) = build_workspace(cx);
    cx.run_until_parked();
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.open_agent_chat_pane(window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    // No lane cwd, so the pane parks in `Error`.
    assert_eq!(active_tab_status(&ws, cx), Some(TabIndicator::Failed));

    ws.update(cx, |ws, cx| {
        let pane_id = ws.active_runtime().focused_pane_id;
        super::agent_chat::agent_view(ws, pane_id).update(cx, |v, _| {
            v.set_status_for_test(
                crate::workspace::main_area::agent_chat_pane::view::AgentSessionStatus::Connected,
            );
            v.set_turn_in_flight();
        });
    });
    assert_eq!(active_tab_status(&ws, cx), Some(TabIndicator::Working));
}

#[gpui::test]
async fn a_terminal_tab_has_no_status(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    cx.run_until_parked();
    assert_eq!(active_tab_status(&ws, cx), None);
}

/// An agent-chat pane in a tab behind a fresh terminal tab, in a window the
/// user is looking at.
fn agent_behind_a_terminal(
    cx: &mut TestAppContext,
) -> (
    gpui::WindowHandle<gpui_component::Root>,
    gpui::Entity<Workspace>,
    PaneId,
) {
    let (wh, ws) = build_workspace(cx);
    cx.run_until_parked();
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.open_agent_chat_pane(window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    let agent = ws.read_with(cx, |ws, _| ws.active_runtime().focused_pane_id);
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.add_tab(window, cx);
            ws.set_window_active(true, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
    (wh, ws, agent)
}

fn finish(
    ws: &gpui::Entity<Workspace>,
    pane: PaneId,
    outcome: TurnOutcome,
    cx: &mut TestAppContext,
) {
    ws.update(cx, |ws, cx| ws.fire_activity_completion(pane, outcome, cx));
    cx.run_until_parked();
}

fn unseen(
    ws: &gpui::Entity<Workspace>,
    pane: PaneId,
    cx: &mut TestAppContext,
) -> Option<AgentOutcome> {
    ws.read_with(cx, |ws, _| ws.unseen_outcomes.for_panes(&[pane]).next())
}

fn tab_of(ws: &gpui::Entity<Workspace>, pane: PaneId, cx: &mut TestAppContext) -> usize {
    ws.read_with(cx, |ws, _| {
        ws.active_runtime()
            .tabs
            .iter()
            .position(|tab| tab.layout.contains(pane))
            .unwrap()
    })
}

fn activate_tab(
    wh: gpui::WindowHandle<gpui_component::Root>,
    ws: &gpui::Entity<Workspace>,
    index: usize,
    cx: &mut TestAppContext,
) {
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.activate_tab(index, window, cx));
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui::test]
async fn a_turn_ending_in_a_hidden_tab_stays_until_the_tab_is_shown(cx: &mut TestAppContext) {
    let (wh, ws, agent) = agent_behind_a_terminal(cx);
    finish(&ws, agent, TurnOutcome::Completed, cx);
    assert_eq!(unseen(&ws, agent, cx), Some(AgentOutcome::Completed));

    let index = tab_of(&ws, agent, cx);
    activate_tab(wh, &ws, index, cx);
    assert_eq!(
        unseen(&ws, agent, cx),
        None,
        "the user is looking at it now"
    );
}

#[gpui::test]
async fn a_failed_turn_is_kept_as_an_error(cx: &mut TestAppContext) {
    let (_wh, ws, agent) = agent_behind_a_terminal(cx);
    finish(&ws, agent, TurnOutcome::Errored, cx);
    assert_eq!(unseen(&ws, agent, cx), Some(AgentOutcome::Errored));
}

#[gpui::test]
async fn a_stopped_turn_leaves_nothing(cx: &mut TestAppContext) {
    let (_wh, ws, agent) = agent_behind_a_terminal(cx);
    finish(&ws, agent, TurnOutcome::Stopped, cx);
    assert_eq!(
        unseen(&ws, agent, cx),
        None,
        "a Stop is the user's own doing"
    );
}

#[gpui::test]
async fn a_turn_ending_in_front_of_the_user_leaves_nothing(cx: &mut TestAppContext) {
    let (wh, ws, agent) = agent_behind_a_terminal(cx);
    let index = tab_of(&ws, agent, cx);
    activate_tab(wh, &ws, index, cx);
    finish(&ws, agent, TurnOutcome::Completed, cx);
    assert_eq!(unseen(&ws, agent, cx), None);
}

/// The active tab of a background window is not in front of anyone.
#[gpui::test]
async fn a_background_window_keeps_the_outcome_until_it_comes_forward(cx: &mut TestAppContext) {
    let (wh, ws, agent) = agent_behind_a_terminal(cx);
    let index = tab_of(&ws, agent, cx);
    activate_tab(wh, &ws, index, cx);
    ws.update(cx, |ws, cx| ws.set_window_active(false, cx));
    finish(&ws, agent, TurnOutcome::Completed, cx);
    assert_eq!(unseen(&ws, agent, cx), Some(AgentOutcome::Completed));

    ws.update(cx, |ws, cx| ws.set_window_active(true, cx));
    cx.run_until_parked();
    assert_eq!(unseen(&ws, agent, cx), None);
}

/// Settings covers the tab without changing which tab is active, and
/// closing it goes through no tab or persistence path.
#[gpui::test]
async fn closing_settings_over_the_tab_shows_the_outcome(cx: &mut TestAppContext) {
    let (wh, ws, agent) = agent_behind_a_terminal(cx);
    let index = tab_of(&ws, agent, cx);
    activate_tab(wh, &ws, index, cx);
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.open_settings(daruda_config::BuiltinSection::Font, window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();
    finish(&ws, agent, TurnOutcome::Completed, cx);
    assert_eq!(unseen(&ws, agent, cx), Some(AgentOutcome::Completed));

    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.close_settings(window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(unseen(&ws, agent, cx), None);
}

#[gpui::test]
async fn closing_the_pane_forgets_its_outcome(cx: &mut TestAppContext) {
    let (wh, ws, agent) = agent_behind_a_terminal(cx);
    finish(&ws, agent, TurnOutcome::Completed, cx);
    let index = tab_of(&ws, agent, cx);
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.request_close_tab(index, window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.unseen_outcomes.panes().count(), 0)
    });
}

/// A terminal tab bound to Claude session `sess-pty`, behind a second tab,
/// whose turn the hook channel has seen open.
fn claude_terminal_behind_a_tab(
    cx: &mut TestAppContext,
) -> (gpui::Entity<Workspace>, PaneId, tempfile::TempDir) {
    let (wh, ws) = build_workspace(cx);
    cx.run_until_parked();
    let dir = tempfile::tempdir().unwrap();
    let pane = cx
        .update_window(wh.into(), |_, window, cx| {
            ws.update(cx, |ws, cx| {
                let pane = ws.active_runtime().focused_pane_id;
                ws.claude.pty_claude_bindings.insert(
                    pane,
                    daruda_agent::pty_tracker::PtyBinding {
                        claude_pid: 4242,
                        session_id: SESSION.into(),
                    },
                );
                ws.add_tab(window, cx);
                ws.set_window_active(true, cx);
                pane
            })
        })
        .unwrap();
    cx.run_until_parked();
    hook(
        &ws,
        &dir,
        SESSION,
        SessionStatus::Working,
        "UserPromptSubmit",
        cx,
    );
    (ws, pane, dir)
}

const SESSION: &str = "sess-pty";

fn hook(
    ws: &gpui::Entity<Workspace>,
    dir: &tempfile::TempDir,
    session: &str,
    status: SessionStatus,
    event: &str,
    cx: &mut TestAppContext,
) {
    let path = dir.path().join(format!("{session}.json"));
    let file = StatusFile::new_hook(session, dir.path(), status, event);
    daruda_agent::hooks::status_file::write_atomic(&path, &file).unwrap();
    ws.update(cx, |ws, cx| {
        ws.apply_claude_status_event(crate::hooks::watcher::StatusEvent::Changed(path), cx)
    });
    cx.run_until_parked();
}

#[gpui::test]
async fn a_terminal_claude_stopping_out_of_view_leaves_a_completion(cx: &mut TestAppContext) {
    let (ws, pane, dir) = claude_terminal_behind_a_tab(cx);
    hook(&ws, &dir, SESSION, SessionStatus::Idle, "Stop", cx);
    assert_eq!(unseen(&ws, pane, cx), Some(AgentOutcome::Completed));
}

#[gpui::test]
async fn a_terminal_claude_stop_failure_leaves_an_error(cx: &mut TestAppContext) {
    let (ws, pane, dir) = claude_terminal_behind_a_tab(cx);
    hook(&ws, &dir, SESSION, SessionStatus::Idle, "StopFailure", cx);
    assert_eq!(unseen(&ws, pane, cx), Some(AgentOutcome::Errored));
}

/// The watcher hands the same file over again; that turn already ended.
#[gpui::test]
async fn a_redelivered_stop_ends_no_turn(cx: &mut TestAppContext) {
    let (ws, pane, dir) = claude_terminal_behind_a_tab(cx);
    hook(&ws, &dir, SESSION, SessionStatus::Idle, "Stop", cx);
    ws.update(cx, |ws, _| ws.unseen_outcomes.forget(&[pane]));
    hook(&ws, &dir, SESSION, SessionStatus::Idle, "Stop", cx);
    assert_eq!(unseen(&ws, pane, cx), None);
}

/// The JSONL fallback writes the same status store and can settle the
/// session to `Idle` — with a newer timestamp than the last hook — before
/// the `Stop` hook lands. The turn still ends with its outcome.
#[gpui::test]
async fn a_jsonl_idle_ahead_of_the_stop_hook_keeps_the_outcome(cx: &mut TestAppContext) {
    let (ws, pane, dir) = claude_terminal_behind_a_tab(cx);
    ws.update(cx, |ws, cx| {
        ws.apply_claude_jsonl_event(
            crate::hooks::jsonl_watcher::JsonlEvent {
                session_id: SESSION.into(),
                cwd: dir.path().to_path_buf(),
                jsonl_path: dir.path().join("sess-pty.jsonl"),
                status: SessionStatus::Idle,
                timestamp: chrono::Utc::now() + chrono::Duration::seconds(5),
            },
            cx,
        )
    });
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| {
        assert_eq!(
            ws.claude.claude_status.get(SESSION).map(|f| f.status),
            Some(SessionStatus::Idle),
            "the fallback did overtake the hook's Working"
        );
    });
    hook(&ws, &dir, SESSION, SessionStatus::Idle, "Stop", cx);
    assert_eq!(unseen(&ws, pane, cx), Some(AgentOutcome::Completed));
}

/// Hook events fan out to every workspace; one that holds no terminal for
/// the session records nothing.
#[gpui::test]
async fn a_stop_for_a_session_with_no_pane_here_records_nothing(cx: &mut TestAppContext) {
    let (ws, _pane, dir) = claude_terminal_behind_a_tab(cx);
    hook(
        &ws,
        &dir,
        "elsewhere",
        SessionStatus::Working,
        "UserPromptSubmit",
        cx,
    );
    hook(&ws, &dir, "elsewhere", SessionStatus::Idle, "Stop", cx);
    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.unseen_outcomes.panes().count(), 0)
    });
}

/// The real activation edge, not the mirror's setter: a test window starts
/// in the background, so a turn ending in its active tab is unseen until
/// the window comes forward.
#[gpui::test]
async fn the_window_coming_forward_shows_the_active_tab(cx: &mut TestAppContext) {
    let (wh, ws) = build_workspace(cx);
    cx.run_until_parked();
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.open_agent_chat_pane(window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    let agent = ws.read_with(cx, |ws, _| {
        assert!(
            !ws.window_runtime.active,
            "a test window starts in the background"
        );
        ws.active_runtime().focused_pane_id
    });
    finish(&ws, agent, TurnOutcome::Completed, cx);
    assert_eq!(unseen(&ws, agent, cx), Some(AgentOutcome::Completed));

    cx.update_window(wh.into(), |_, window, _| window.activate_window())
        .unwrap();
    cx.run_until_parked();
    assert!(ws.read_with(cx, |ws, _| ws.window_runtime.active));
    assert_eq!(unseen(&ws, agent, cx), None);
}
/// The tab strip reads the mark: the hidden tab shows it, the visible one
/// does not, and a live failure outranks it.
#[gpui::test]
async fn the_hidden_tab_shows_its_unseen_outcome(cx: &mut TestAppContext) {
    let (_wh, ws, agent) = agent_behind_a_terminal(cx);
    finish(&ws, agent, TurnOutcome::Completed, cx);
    let index = tab_of(&ws, agent, cx);
    let indicators = |ws: &gpui::Entity<Workspace>, cx: &mut TestAppContext| {
        ws.read_with(cx, |ws, cx| {
            ws.tab_cells(cx)
                .into_iter()
                .map(|cell| cell.indicator)
                .collect::<Vec<_>>()
        })
    };
    // The pane parks in `Error` without a cwd, so its live status is a
    // failure, which outranks the unseen completion.
    assert_eq!(indicators(&ws, cx)[index], Some(TabIndicator::Failed));

    ws.update(cx, |ws, cx| {
        super::agent_chat::agent_view(ws, agent).update(cx, |v, _| {
            v.set_status_for_test(
                crate::workspace::main_area::agent_chat_pane::view::AgentSessionStatus::Connected,
            );
        });
    });
    let shown = indicators(&ws, cx);
    assert_eq!(shown[index], Some(TabIndicator::Done));
    assert!(
        shown
            .iter()
            .enumerate()
            .all(|(i, d)| i == index || d.is_none()),
        "only the agent's tab carries a dot: {shown:?}"
    );
}
