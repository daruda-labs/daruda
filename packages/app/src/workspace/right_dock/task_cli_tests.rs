//! CLI runs are owned by their pane, confirmed gone only by the OS, and
//! mirrored read-only until then.

use super::*;
use crate::control::result::ControlError;
use crate::hooks::pty_tracker::PtyTrackerEvent;
use crate::workspace::main_area::agent_chat_pane::agent_chat_event_ops::PumpStep;
use crate::workspace::main_area::agent_chat_pane::view::{
    AgentSessionStatus, LoadIntent, PromptDispatch,
};
use crate::workspace::main_area::pane::{Pane, TabEntry};
use crate::workspace::main_area::pane_input_ops::{PaneTextInput, PaneTextIntent};
use crate::workspace::main_area::pane_tree::PaneLayout;
use crate::workspace::tests::build_workspace;
use daruda_store::project::PaneCwd;
use daruda_store::tasks::{AgentChatAccess, SessionEndReason, Task, TaskAgentSurface};
use gpui::{AppContext as _, TestAppContext, Window};

fn cli_task(cwd: &Path, cx: &mut gpui::App) -> String {
    let mut task = Task::new(
        daruda_store::project::ProjectUuid::default(),
        "CLI".into(),
        "prompt".into(),
        None,
    );
    task.agent_surface = TaskAgentSurface::Terminal;
    task.state = TaskState::Running {
        worktree_path: cwd.to_path_buf(),
    };
    let id = task.id.clone();
    cx.update_global::<GlobalTasks, _>(|tasks, _| {
        tasks.add(task);
    });
    id
}

fn terminal_panes(ws: &Workspace) -> Vec<PaneId> {
    ws.active_runtime()
        .panes
        .iter()
        .filter(|pane| matches!(pane.content, PaneContent::Terminal(_)))
        .map(|pane| pane.id)
        .collect()
}

fn run(id: &str, cx: &gpui::App) -> TaskExecution {
    cx.global::<GlobalTasks>()
        .get(id)
        .unwrap()
        .execution
        .clone()
        .unwrap()
}

fn process(id: &str, cx: &gpui::App) -> CliProcessState {
    run(id, cx).source.cli_process().cloned().unwrap()
}

fn bind(ws: &mut Workspace, pane: PaneId, pid: u32, session: &str, cx: &mut Context<Workspace>) {
    ws.apply_pty_tracker_event(
        PtyTrackerEvent::BindingChanged {
            pane_id: pane,
            binding: Some(PtyBinding {
                claude_pid: pid,
                session_id: session.into(),
            }),
        },
        cx,
    );
}

/// What the tracker reports when `pid` stops running `session`: the panes it
/// was bound in unbind, then the exit.
fn exit(ws: &mut Workspace, pid: u32, session: &str, cx: &mut Context<Workspace>) {
    let bound: Vec<PaneId> = ws
        .claude
        .pty_claude_bindings
        .iter()
        .filter(|(_, binding)| binding.session_id == session && binding.claude_pid == pid)
        .map(|(pane, _)| *pane)
        .collect();
    for pane_id in bound {
        ws.apply_pty_tracker_event(
            PtyTrackerEvent::BindingChanged {
                pane_id,
                binding: None,
            },
            cx,
        );
    }
    ws.apply_pty_tracker_event(
        PtyTrackerEvent::SessionProcessExited {
            session_id: session.into(),
            claude_pid: pid,
        },
        cx,
    );
}

/// A read-only chat pane mirroring `task_id`'s current run.
fn snapshot_pane(
    ws: &mut Workspace,
    task_id: &str,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> PaneId {
    let execution_id = run(task_id, cx).id;
    let pane = ws.create_agent_chat_pane(
        Some(PaneCwd::Local(std::env::temp_dir())),
        Some("cli-session".into()),
        ws.agents[0].id.clone(),
        None,
        window,
        cx,
    );
    let id = pane.id;
    pane.agent_chat_view().unwrap().update(cx, |v, _| {
        v.set_access(AgentChatAccess::CliSnapshot(ExecutionRef {
            task_id: task_id.into(),
            execution_id,
        }));
        v.status = AgentSessionStatus::Connected;
    });
    let tab_id = ws.alloc_id();
    ws.active_runtime_mut().panes.push(pane);
    ws.active_runtime_mut().tabs.push(TabEntry {
        id: tab_id,
        layout: PaneLayout::Pane(id),
        last_focused_pane: id,
        user_label: None,
    });
    id
}

/// Give `pane` a live-looking session and return the probe watching it.
fn attach_handle(
    ws: &Workspace,
    pane: PaneId,
    cx: &mut Context<Workspace>,
) -> daruda_acp::HandleProbe {
    let (handle, probe) = daruda_acp::AcpSessionHandle::detached_for_test();
    ws.agent_chat_view(pane)
        .unwrap()
        .update(cx, |v, _| v.attach_handle(handle));
    probe
}

/// A task whose run was bound to the first terminal pane, then confirmed.
fn bound_task(ws: &mut Workspace, pid: u32, cx: &mut Context<Workspace>) -> (String, PaneId) {
    let cwd = std::env::temp_dir();
    let id = cli_task(&cwd, cx);
    let pane = terminal_panes(ws)[0];
    ws.bind_task_cli_execution(&id, pane, &cwd, cx);
    bind(ws, pane, pid, "cli-session", cx);
    (id, pane)
}

#[gpui::test]
fn start_creates_a_run_owned_by_the_pane_before_any_session(cx: &mut TestAppContext) {
    let (window, workspace) = build_workspace(cx);
    cx.update_window(window.into(), |_, _, cx| {
        workspace.update(cx, |ws, cx| {
            let cwd = std::env::temp_dir();
            let id = cli_task(&cwd, cx);
            let pane = terminal_panes(ws)[0];
            // A stub pane has no shell to register; a spawned one is
            // registered before any task can be dispatched into it.
            ws.claude.pty_tracker.register(pane, 4242);
            ws.bind_task_cli_execution(&id, pane, &cwd, cx);
            let execution = run(&id, cx);
            assert_eq!(execution.session_id, None);
            assert_eq!(process(&id, cx), CliProcessState::Discovering);
            assert!(!execution.chat_available());
            let owner = ws.active_runtime().panes.iter().find(|p| p.id == pane);
            let PaneContent::Terminal(terminal) = &owner.unwrap().content else {
                panic!("terminal pane");
            };
            assert_eq!(
                terminal.task_run,
                Some(ExecutionRef {
                    task_id: id.clone(),
                    execution_id: execution.id
                })
            );
            assert!(ws.claude.pty_tracker.awaits_binding(pane));
        })
    })
    .unwrap();
}

#[gpui::test]
fn binding_records_only_the_owning_pane_and_its_first_session(cx: &mut TestAppContext) {
    let (window, workspace) = build_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.add_tab(window, cx);
            let panes = terminal_panes(ws);
            let cwd = std::env::temp_dir();
            let owned = cli_task(&cwd, cx);
            let neighbour = cli_task(&cwd, cx);
            ws.bind_task_cli_execution(&owned, panes[0], &cwd, cx);
            ws.bind_task_cli_execution(&neighbour, panes[1], &cwd, cx);

            bind(ws, panes[1], 20, "neighbour-session", cx);
            assert_eq!(run(&owned, cx).session_id, None);

            bind(ws, panes[0], 10, "owned-session", cx);
            let execution = run(&owned, cx);
            assert_eq!(execution.session_id.as_deref(), Some("owned-session"));
            assert_eq!(process(&owned, cx), CliProcessState::Running { pid: 10 });
            assert!(execution.chat_available());
            let task = cx.global::<GlobalTasks>().get(&owned).unwrap();
            assert_eq!(task.session_ids, vec!["owned-session".to_string()]);

            // A second `claude` in the same terminal is not this run.
            bind(ws, panes[0], 11, "later-session", cx);
            assert_eq!(run(&owned, cx).session_id.as_deref(), Some("owned-session"));
            assert_eq!(process(&owned, cx), CliProcessState::Running { pid: 10 });
            assert_eq!(
                run(&neighbour, cx).session_id.as_deref(),
                Some("neighbour-session")
            );

            // A hook for a same-cwd session no run owns attaches nothing.
            ws.apply_task_session_changed("stray-session", cx);
            let task = cx.global::<GlobalTasks>().get(&owned).unwrap();
            assert_eq!(task.session_ids, vec!["owned-session".to_string()]);
        })
    })
    .unwrap();
}

#[gpui::test]
fn a_session_already_bound_to_the_pane_is_not_the_new_run(cx: &mut TestAppContext) {
    let (window, workspace) = build_workspace(cx);
    cx.update_window(window.into(), |_, _, cx| {
        workspace.update(cx, |ws, cx| {
            let cwd = std::env::temp_dir();
            let pane = terminal_panes(ws)[0];
            bind(ws, pane, 5, "earlier-session", cx);
            let id = cli_task(&cwd, cx);
            ws.bind_task_cli_execution(&id, pane, &cwd, cx);
            assert_eq!(run(&id, cx).session_id, None);
            assert_eq!(process(&id, cx), CliProcessState::Discovering);

            // The run's own `claude` is a new binding, so its event arrives.
            bind(ws, pane, 6, "run-session", cx);
            assert_eq!(run(&id, cx).session_id.as_deref(), Some("run-session"));
            assert_eq!(process(&id, cx), CliProcessState::Running { pid: 6 });
        })
    })
    .unwrap();
}

#[gpui::test]
fn late_events_never_reach_a_rerun(cx: &mut TestAppContext) {
    let (window, workspace) = build_workspace(cx);
    cx.update_window(window.into(), |_, _, cx| {
        workspace.update(cx, |ws, cx| {
            let (id, pane) = bound_task(ws, 10, cx);
            let cwd = std::env::temp_dir();
            ws.bind_task_cli_execution(&id, pane, &cwd, cx);
            let rerun = run(&id, cx).id;

            exit(ws, 10, "cli-session", cx);
            assert_eq!(process(&id, cx), CliProcessState::Discovering);
            assert_eq!(run(&id, cx).id, rerun);

            // A binding still held against the replaced run id is ignored.
            let PaneContent::Terminal(terminal) = &mut ws
                .active_runtime_mut()
                .panes
                .iter_mut()
                .find(|p| p.id == pane)
                .unwrap()
                .content
            else {
                panic!("terminal pane");
            };
            terminal.task_run = Some(ExecutionRef {
                task_id: id.clone(),
                execution_id: "replaced-run".into(),
            });
            bind(ws, pane, 12, "stale-session", cx);
            assert_eq!(run(&id, cx).session_id, None);
        })
    })
    .unwrap();
}

#[gpui::test]
fn transcript_fills_whichever_of_hook_and_binding_arrives_first(cx: &mut TestAppContext) {
    let (window, workspace) = build_workspace(cx);
    cx.update_window(window.into(), |_, _, cx| {
        workspace.update(cx, |ws, cx| {
            let transcript = |run: TaskExecution| match run.source {
                TaskExecutionSource::ClaudeCli {
                    transcript_path, ..
                } => transcript_path,
                TaskExecutionSource::AgentChat => panic!("CLI run"),
            };
            let cwd = std::env::temp_dir();

            // Hook first: the status store already knows the transcript.
            let early = cli_task(&cwd, cx);
            let pane = terminal_panes(ws)[0];
            ws.bind_task_cli_execution(&early, pane, &cwd, cx);
            let mut file = daruda_agent::hooks::status_file::StatusFile::new_hook(
                "early-session",
                &cwd,
                daruda_agent::SessionStatus::Working,
                "SessionStart",
            );
            file.transcript_path = Some("/tmp/early.jsonl".into());
            ws.claude.claude_status.update(file);
            ws.record_task_cli_transcript("early-session", Some(Path::new("/tmp/x")), cx);
            assert_eq!(transcript(run(&early, cx)), None);
            bind(ws, pane, 10, "early-session", cx);
            assert_eq!(transcript(run(&early, cx)), Some("/tmp/early.jsonl".into()));

            // Binding first: the hook fills it in afterwards.
            let late = cli_task(&cwd, cx);
            ws.bind_task_cli_execution(&late, pane, &cwd, cx);
            bind(ws, pane, 11, "late-session", cx);
            assert_eq!(transcript(run(&late, cx)), None);
            ws.record_task_cli_transcript("late-session", Some(Path::new("/tmp/late.jsonl")), cx);
            assert_eq!(transcript(run(&late, cx)), Some("/tmp/late.jsonl".into()));
        })
    })
    .unwrap();
}

#[gpui::test]
fn only_the_observed_pid_leaving_the_os_confirms_exit(cx: &mut TestAppContext) {
    let (window, workspace) = build_workspace(cx);
    cx.update_window(window.into(), |_, _, cx| {
        workspace.update(cx, |ws, cx| {
            let (id, pane) = bound_task(ws, 10, cx);

            // Stop / SessionEnd / status-file removal end the task, not the CLI.
            ws.apply_task_session_ended("cli-session", SessionEndReason::Stop, cx);
            let state = cx.global::<GlobalTasks>().get(&id).unwrap().state.clone();
            assert!(matches!(state, TaskState::Done { .. }));
            assert_eq!(process(&id, cx), CliProcessState::Running { pid: 10 });

            // Detaching the pane is not an exit either.
            ws.apply_pty_tracker_event(
                PtyTrackerEvent::BindingChanged {
                    pane_id: pane,
                    binding: None,
                },
                cx,
            );
            assert_eq!(process(&id, cx), CliProcessState::Running { pid: 10 });

            exit(ws, 99, "cli-session", cx);
            assert_eq!(process(&id, cx), CliProcessState::Running { pid: 10 });

            exit(ws, 10, "cli-session", cx);
            assert!(matches!(
                process(&id, cx),
                CliProcessState::ExitConfirmed { .. }
            ));
            // The task's own outcome is untouched by the process record.
            assert_eq!(cx.global::<GlobalTasks>().get(&id).unwrap().state, state);
        })
    })
    .unwrap();
}

/// `claude --resume` keeps the session id, so a binding for an exited run's
/// session — in any terminal — means it is being written again.
#[gpui::test]
fn a_resumed_session_undoes_a_confirmed_exit(cx: &mut TestAppContext) {
    let (window, workspace) = build_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let (id, _) = bound_task(ws, 10, cx);
            let chat = snapshot_pane(ws, &id, window, cx);
            exit(ws, 10, "cli-session", cx);
            assert!(matches!(
                process(&id, cx),
                CliProcessState::ExitConfirmed { .. }
            ));

            ws.add_tab(window, cx);
            let other = *terminal_panes(ws).last().unwrap();
            bind(ws, other, 20, "cli-session", cx);
            assert_eq!(process(&id, cx), CliProcessState::Running { pid: 20 });
            ws.continue_cli_chat(chat, cx);
            assert_eq!(
                ws.agent_chat_view(chat).unwrap().read(cx).load_intent(),
                Some(LoadIntent::Snapshot)
            );

            exit(ws, 20, "cli-session", cx);
            assert!(matches!(
                process(&id, cx),
                CliProcessState::ExitConfirmed { .. }
            ));
        })
    })
    .unwrap();
}

/// A confirmed exit is not enough while something still shows the session
/// live — a pane bound to it, or a process the tracker has not seen go.
#[gpui::test]
fn continue_needs_the_session_to_be_idle_everywhere(cx: &mut TestAppContext) {
    let (window, workspace) = build_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let (id, _) = bound_task(ws, 10, cx);
            let chat = snapshot_pane(ws, &id, window, cx);
            exit(ws, 10, "cli-session", cx);
            let run = ws
                .agent_chat_view(chat)
                .unwrap()
                .read(cx)
                .mirrored_run()
                .cloned()
                .unwrap();
            assert!(ws.cli_run_may_continue(&run, "cli-session", cx));

            let dir = tempfile::tempdir().unwrap();
            ws.claude.pty_tracker.track_session(
                "cli-session".into(),
                std::process::id(),
                dir.path().to_path_buf(),
            );
            assert!(!ws.cli_run_may_continue(&run, "cli-session", cx));
        })
    })
    .unwrap();
}

#[gpui::test]
fn restart_rearms_tracking_for_running_pids_only(cx: &mut TestAppContext) {
    let (window, workspace) = build_workspace(cx);
    cx.update_window(window.into(), |_, _, cx| {
        workspace.update(cx, |ws, cx| {
            let cwd = std::env::temp_dir();
            let running = cli_task(&cwd, cx);
            let ended = cli_task(&cwd, cx);
            let set =
                |id: &str, session: &str, process: CliProcessState, cx: &mut Context<Workspace>| {
                    cx.update_global::<GlobalTasks, _>(|tasks, _| {
                        tasks.get_mut(id).unwrap().execution = Some(TaskExecution {
                            source: TaskExecutionSource::ClaudeCli {
                                transcript_path: None,
                                process,
                            },
                            id: "run".into(),
                            agent_id: "claude".into(),
                            account_id: None,
                            cwd: std::env::temp_dir(),
                            session_id: Some(session.into()),
                        });
                    });
                };
            set(
                &running,
                "running-session",
                CliProcessState::Running { pid: 4242 },
                cx,
            );
            set(
                &ended,
                "ended-session",
                CliProcessState::ExitConfirmed {
                    ended_at: Utc::now(),
                },
                cx,
            );
            let orphan = cli_task(&cwd, cx);
            set(&orphan, "orphan-session", CliProcessState::Discovering, cx);
            ws.restore_task_cli_tracking(cx);
            assert_eq!(
                ws.claude.pty_tracker.known_process("running-session"),
                Some(4242)
            );
            assert_eq!(ws.claude.pty_tracker.known_process("ended-session"), None);
            // Its pane did not survive the restart, so nothing will bind it.
            assert_eq!(process(&orphan, cx), CliProcessState::Unknown);
        })
    })
    .unwrap();
}

#[gpui::test]
fn cli_snapshot_refuses_every_input_path(cx: &mut TestAppContext) {
    let (window, workspace) = build_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let (task, _) = bound_task(ws, 10, cx);
            let id = snapshot_pane(ws, &task, window, cx);
            let mut probe = attach_handle(ws, id, cx);
            let view = ws.agent_chat_view(id).unwrap().clone();
            let before = {
                let v = view.read(cx);
                (v.picked_mode_id.clone(), v.picked_model_id.clone())
            };

            ws.send_agent_prompt_text(id, "must not send".into(), cx);
            ws.send_agent_prompt_text(id, "/clear".into(), cx);
            assert!(matches!(
                ws.send_agent_prompt_text_from_telegram(id, "phone".into(), cx),
                Some(PromptDispatch::ReadOnly)
            ));
            assert!(matches!(
                ws.control_say(id, "orchestrator".into(), cx),
                Err(ControlError::TargetReadOnly)
            ));
            assert!(!ws.deliver_text_to_pane(
                id,
                PaneTextInput {
                    body: "macro".into(),
                    intent: PaneTextIntent::Command { submit: true },
                },
                window,
                cx,
            ));
            ws.reset_agent_chat_session(id, cx);
            ws.resume_queued_prompts(id, cx);
            view.update(cx, |v, cx| {
                v.set_mode("plan".into(), cx);
                v.set_config_option(
                    "model".into(),
                    daruda_acp::ConfigValueView::Id("other".into()),
                    cx,
                );
                v.cancel_turn(cx);
            });
            // Nothing reached the session, and none of it tore the session down.
            assert_eq!(probe.drain(), (0, false));

            let v = view.read(cx);
            assert!(v.queue.pending_prompts.is_empty());
            assert!(v.items.is_empty());
            assert_eq!(v.session_id.as_deref(), Some("cli-session"));
            assert!(v.is_read_only());
            assert_eq!(
                (v.picked_mode_id.clone(), v.picked_model_id.clone()),
                before
            );
            assert!(ws.task_chat_owner(id, cx).is_none());
            assert!(ws.requires_exact_resume(id, cx));
        });
    })
    .unwrap();
}

#[gpui::test]
fn a_snapshot_never_starts_a_fresh_session(cx: &mut TestAppContext) {
    let (window, workspace) = build_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let (task, _) = bound_task(ws, 10, cx);
            let id = snapshot_pane(ws, &task, window, cx);
            let cwd = PaneCwd::Local(std::env::temp_dir());
            ws.connect_agent_chat(id, cwd, None, cx);
            let v = ws.agent_chat_view(id).unwrap().read(cx);
            assert!(matches!(v.status, AgentSessionStatus::Error { .. }));
            assert!(v.any_handle().is_none());
            assert_eq!(v.session_id.as_deref(), Some("cli-session"));
        });
    })
    .unwrap();
}

fn connected() -> daruda_acp::AcpEvent {
    daruda_acp::AcpEvent::Connected {
        program: None,
        session_id: "cli-session".into(),
        modes: None,
        config_options: Vec::new(),
        capabilities: Default::default(),
        login_methods: Vec::new(),
    }
}

/// Every load step goes through the pump's own per-event fold.
#[gpui::test]
fn continue_waits_for_confirmed_exit_and_a_completed_load(cx: &mut TestAppContext) {
    let (window, workspace) = build_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let (task, _) = bound_task(ws, 10, cx);
            let id = snapshot_pane(ws, &task, window, cx);
            let view = ws.agent_chat_view(id).unwrap().clone();
            let state = cx.global::<GlobalTasks>().get(&task).unwrap().state.clone();

            // Still running: continuing is refused before any load starts.
            ws.continue_cli_chat(id, cx);
            assert_eq!(view.read(cx).load_intent(), Some(LoadIntent::Snapshot));
            assert!(view.read(cx).is_read_only());

            // A snapshot load ends at `Connected`: the pump stops and the
            // session the replay came through is let go.
            let mut snapshot = attach_handle(ws, id, cx);
            assert_eq!(
                ws.fold_agent_chat_event(id, connected(), cx),
                PumpStep::Release
            );
            assert!(view.read(cx).is_read_only());
            assert!(view.read(cx).any_handle().is_none());
            assert_eq!(snapshot.drain(), (0, true));

            exit(ws, 10, "cli-session", cx);
            // A continue whose required load failed stays a snapshot.
            view.update(cx, |v, _| v.set_load_intent(LoadIntent::Continue));
            let failed = daruda_acp::AcpEvent::Error(daruda_acp::AcpFailure::unclassified(
                "session not found",
            ));
            ws.fold_agent_chat_event(id, failed, cx);
            let v = view.read(cx);
            assert!(v.is_read_only());
            assert!(matches!(v.status, AgentSessionStatus::Error { .. }));
            assert_eq!(v.load_intent(), Some(LoadIntent::Snapshot));

            // A continue whose load completed keeps its session and takes input.
            let mut live = attach_handle(ws, id, cx);
            view.update(cx, |v, _| v.set_load_intent(LoadIntent::Continue));
            assert_eq!(
                ws.fold_agent_chat_event(id, connected(), cx),
                PumpStep::Continue
            );
            assert_eq!(live.drain(), (0, false));
            assert_eq!(view.read(cx).access(), AgentChatAccess::Interactive);
            ws.send_agent_prompt_text(id, "carry on".into(), cx);
            assert_eq!(live.drain(), (1, false));
            let v = view.read(cx);
            assert_eq!(v.session_id.as_deref(), Some("cli-session"));
            assert!(ws.task_chat_owner(id, cx).is_none());
            assert_eq!(cx.global::<GlobalTasks>().get(&task).unwrap().state, state);
        });
    })
    .unwrap();
}

/// A continue whose run stopped being safe to take over mid-load settles as
/// a snapshot at `Connected` — its history shown, its session let go, and
/// the closing stream not reported as a failure.
#[gpui::test]
fn a_continue_that_loses_its_gate_mid_load_settles_as_a_snapshot(cx: &mut TestAppContext) {
    let (window, workspace) = build_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let (task, _) = bound_task(ws, 10, cx);
            let id = snapshot_pane(ws, &task, window, cx);
            exit(ws, 10, "cli-session", cx);
            let view = ws.agent_chat_view(id).unwrap().clone();
            view.update(cx, |v, _| v.set_load_intent(LoadIntent::Continue));
            let mut probe = attach_handle(ws, id, cx);
            cx.update_global::<GlobalTasks, _>(|tasks, _| {
                tasks.get_mut(&task).unwrap().execution.as_mut().unwrap().id = "rerun".into();
            });

            assert_eq!(
                ws.fold_agent_chat_event(id, connected(), cx),
                PumpStep::Release
            );
            let v = view.read(cx);
            assert!(v.is_read_only());
            assert!(matches!(v.status, AgentSessionStatus::Connected));
            assert_eq!(probe.drain(), (0, true));
        });
    })
    .unwrap();
}

#[gpui::test]
fn a_snapshot_of_a_deleted_or_rerun_task_stays_read_only(cx: &mut TestAppContext) {
    let (window, workspace) = build_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let (task, _) = bound_task(ws, 10, cx);
            let id = snapshot_pane(ws, &task, window, cx);
            exit(ws, 10, "cli-session", cx);
            let run = ws
                .agent_chat_view(id)
                .unwrap()
                .read(cx)
                .mirrored_run()
                .cloned()
                .unwrap();
            assert!(matches!(
                ws.cli_run_process(&run, cx),
                Some(CliProcessState::ExitConfirmed { .. })
            ));

            cx.update_global::<GlobalTasks, _>(|tasks, _| {
                tasks.get_mut(&task).unwrap().execution.as_mut().unwrap().id = "rerun".into();
            });
            assert_eq!(ws.cli_run_process(&run, cx), None);
            ws.continue_cli_chat(id, cx);
            assert!(ws.agent_chat_view(id).unwrap().read(cx).is_read_only());

            cx.update_global::<GlobalTasks, _>(|tasks, _| tasks.remove(&task));
            assert_eq!(ws.cli_run_process(&run, cx), None);
            let view = ws.agent_chat_view(id).unwrap().clone();
            view.update(cx, |v, _| v.set_load_intent(LoadIntent::Continue));
            ws.finish_cli_chat_load(id, cx);
            assert!(view.read(cx).is_read_only());
        });
    })
    .unwrap();
}

#[gpui::test]
fn open_chat_marks_a_cli_run_read_only_and_restore_keeps_it(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let project = daruda_store::project::Project::from_path(root.path());
    let (window, workspace) = crate::workspace::tests::build_workspace_with(
        cx,
        &daruda_config::Config::default(),
        Some(project),
    );
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            std::sync::Arc::make_mut(&mut ws.agents)[0].launch =
                daruda_config::AgentLaunch::Raw("daruda-missing-test-adapter".into());
            let cwd = ws.active_lane().unwrap().path.clone();
            let task = cli_task(&cwd, cx);
            let pane = terminal_panes(ws)[0];
            ws.bind_task_cli_execution(&task, pane, &cwd, cx);
            cx.update_global::<GlobalTasks, _>(|tasks, _| {
                tasks
                    .get_mut(&task)
                    .unwrap()
                    .execution
                    .as_mut()
                    .unwrap()
                    .agent_id = ws.agents[0].id.clone();
            });
            bind(ws, pane, 10, "cli-session", cx);
            let execution_id = run(&task, cx).id;

            ws.open_task_chat(&task, window, cx);
            let opened = ws.active_runtime().focused_pane_id;
            let expected = AgentChatAccess::CliSnapshot(ExecutionRef {
                task_id: task.clone(),
                execution_id,
            });
            let v = ws.agent_chat_view(opened).unwrap().read(cx);
            assert_eq!(v.access(), expected);
            assert_eq!(v.session_id.as_deref(), Some("cli-session"));

            let (state, projects) = ws.snapshot_for_disk(cx);
            ws.restore_from_disk(&state, &projects, window, cx);
            let chat = ws
                .main_area
                .runtimes
                .values()
                .flat_map(|rt| rt.panes.iter())
                .filter_map(Pane::agent_chat_content)
                .find(|chat| chat.view.read(cx).session_id.as_deref() == Some("cli-session"))
                .unwrap();
            assert_eq!(chat.view.read(cx).access(), expected);
            assert_eq!(chat.agent_id, ws.agents[0].id);
            assert!(chat.task_run.is_none());
        });
    })
    .unwrap();
}
