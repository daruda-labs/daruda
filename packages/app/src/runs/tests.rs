use std::io::Write as _;
use std::path::Path;
use std::time::Duration;

use daruda_store::project::ProjectUuid;
use daruda_store::tasks::machine::TaskEdit;
use daruda_store::tasks::{
    CliProcessState, Lifecycle, TaskEvent, TaskSpec, TaskState, TaskStore, TasksState,
};
use gpui::TestAppContext;

use super::seed::{self, SeedEnd, SeedRun};
use super::{GlobalTasks, RunsHost, TaskCommitError, TaskIntent, no_hook};

fn write_legacy_input(data_dir: &Path, rows: &TasksState) -> std::io::Result<()> {
    std::fs::write(
        daruda_store::tasks::tasks_path_in(data_dir),
        serde_json::to_vec_pretty(rows).unwrap(),
    )
}

fn store_at(path: &Path, legacy: Option<&Path>) -> TaskStore {
    TaskStore::in_directory(path, legacy, "test", no_hook()).unwrap()
}

#[gpui::test]
async fn the_tasks_file_is_never_written_after_import(cx: &mut TestAppContext) {
    let log = tempfile::tempdir().unwrap();
    let legacy = tempfile::tempdir().unwrap();
    let task =
        daruda_store::tasks::Task::new(ProjectUuid::new(), "legacy".into(), "prompt".into(), None);
    let id = task.id.clone();
    let rows = TasksState {
        tasks: vec![task],
        ..Default::default()
    };
    let path = daruda_store::tasks::tasks_path_in(legacy.path());
    let original = serde_json::to_vec_pretty(&rows).unwrap();
    std::fs::write(&path, &original).unwrap();
    let store = store_at(log.path(), Some(legacy.path()));
    cx.update(|cx| {
        RunsHost::install(RunsHost::open(Ok(store.clone())), cx);
        RunsHost::install_quit_flush(cx);
    });
    cx.update(|cx| {
        RunsHost::task(
            TaskIntent::Edit {
                id: id.clone(),
                edit: edit("changed in the log"),
            },
            cx,
        )
    })
    .await
    .unwrap();
    cx.run_until_parked();
    assert_eq!(std::fs::read(&path).unwrap(), original);
    cx.quit();
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert_eq!(
        store.current().unwrap().1.get(&id).unwrap().spec.title,
        "changed in the log"
    );
}

fn edit(title: &str) -> TaskEdit {
    TaskEdit {
        title: title.into(),
        prompt: String::new(),
        notes: String::new(),
        auto_execute: true,
        agent_surface: Default::default(),
        base_worktree_path: None,
        branch: None,
        run_in: None,
        project: None,
    }
}

#[gpui::test]
async fn a_reply_observes_its_committed_projection(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = store_at(dir.path(), None);
    let spec = seed::spec(ProjectUuid::new(), "original");
    let id = spec.id.clone();
    cx.update(|cx| RunsHost::install(RunsHost::open(Ok(store)), cx));
    let created = cx.update(|cx| RunsHost::task(TaskIntent::Create { spec }, cx));
    created.await.unwrap();
    cx.update(|cx| {
        assert_eq!(
            cx.global::<GlobalTasks>().get(&id).unwrap().spec.title,
            "original"
        )
    });
    let updated = cx.update(|cx| {
        RunsHost::task(
            TaskIntent::Edit {
                id: id.clone(),
                edit: edit("updated"),
            },
            cx,
        )
    });
    updated.await.unwrap();
    cx.update(|cx| {
        assert_eq!(
            cx.global::<GlobalTasks>().get(&id).unwrap().spec.title,
            "updated"
        )
    });
}

#[gpui::test]
async fn a_hundred_queued_commands_replay_in_order(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = store_at(dir.path(), None);
    let spec = seed::spec(ProjectUuid::new(), "original");
    let id = spec.id.clone();
    cx.update(|cx| RunsHost::install(RunsHost::open(Ok(store.clone())), cx));
    cx.update(|cx| RunsHost::propose(TaskEvent::Created { spec }, cx))
        .await
        .unwrap();
    let replies = cx.update(|cx| {
        (0..100)
            .map(|number| {
                RunsHost::task(
                    TaskIntent::Edit {
                        id: id.clone(),
                        edit: edit(&number.to_string()),
                    },
                    cx,
                )
            })
            .collect::<Vec<_>>()
    });
    for reply in replies.into_iter().rev() {
        reply.await.unwrap();
    }
    let projected = cx.update(|cx| cx.global::<GlobalTasks>().book().clone());
    assert_eq!(projected.get(&id).unwrap().spec.title, "99");
    cx.update(RunsHost::seed_for_test);
    drop(store);
    let restored = store_at(dir.path(), None).current().unwrap().1;
    assert_eq!(restored, projected);
}

#[gpui::test]
async fn a_second_window_preserves_the_process_projection(cx: &mut TestAppContext) {
    let first = crate::test_support::workspace_for_control(cx);
    let project = first
        .workspace
        .read_with(cx, |ws, _| ws.control_task_projects()[0].0);
    let spec = seed::spec(project, "before");
    let id = spec.id.clone();
    cx.update(|cx| seed::commit_now(seed::created(spec), cx));
    let changed = cx.update(|cx| {
        RunsHost::propose(
            TaskEvent::Edited {
                id: id.clone(),
                edit: edit("after"),
                at: chrono::Utc::now(),
            },
            cx,
        )
    });
    let second = crate::test_support::workspace_for_control(cx);
    changed.await.unwrap();
    cx.update(|cx| {
        assert_eq!(
            cx.global::<GlobalTasks>().get(&id).unwrap().spec.title,
            "after"
        )
    });
    drop((first, second));
}

#[gpui::test]
fn quit_flushes_commands_that_have_not_reached_the_projection(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let legacy = tempfile::tempdir().unwrap();
    let store = store_at(dir.path(), None);
    let spec = seed::spec(ProjectUuid::new(), "queued at quit");
    let id = spec.id.clone();
    cx.update(|cx| {
        RunsHost::install(RunsHost::open(Ok(store.clone())), cx);
        RunsHost::install_quit_flush(cx);
        RunsHost::propose(TaskEvent::Created { spec }, cx).detach();
    });
    cx.quit();
    assert_eq!(
        store.current().unwrap().1.get(&id).unwrap().spec.title,
        "queued at quit"
    );
    assert!(!daruda_store::tasks::tasks_path_in(legacy.path()).exists());
}

#[gpui::test]
fn an_unfinished_reconciliation_reopens_read_only_and_preserves_files(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let legacy = tempfile::tempdir().unwrap();
    let store = store_at(dir.path(), None);
    let spec = seed::spec(ProjectUuid::new(), "preserved");
    let id = spec.id.clone();
    smol::block_on(store.propose(TaskEvent::Created { spec })).unwrap();
    let _claim = store.begin_reconcile().unwrap();
    let expected = store.current().unwrap();
    drop(store);
    let log_path = dir.path().join("log.0.jsonl");
    let mut log = std::fs::OpenOptions::new()
        .append(true)
        .open(&log_path)
        .unwrap();
    log.write_all(b"{\"v\":1,\"seq\":").unwrap();
    log.sync_all().unwrap();
    drop(log);
    let path = daruda_store::tasks::tasks_path_in(legacy.path());
    write_legacy_input(legacy.path(), &TasksState::default()).unwrap();
    let original = std::fs::read(&path).unwrap();
    let marker = dir.path().join(".reconciling");
    let original_files = [&log_path, &marker, &path].map(|path| {
        (
            std::fs::read(path).unwrap(),
            std::fs::metadata(path).unwrap().modified().unwrap(),
        )
    });
    let reopened = store_at(dir.path(), Some(legacy.path()));
    assert_eq!(reopened.current().unwrap(), expected);
    cx.update(|cx| {
        RunsHost::install(RunsHost::open(Ok(reopened)), cx);
        RunsHost::install_quit_flush(cx);
        assert!(!RunsHost::is_writable(cx));
        assert!(cx.global::<GlobalTasks>().get(&id).is_some());
    });
    let refused = cx.update(|cx| {
        RunsHost::propose(
            TaskEvent::Deleted {
                id,
                at: chrono::Utc::now(),
            },
            cx,
        )
    });
    cx.run_until_parked();
    assert_eq!(smol::block_on(refused), Err(TaskCommitError::ReadOnly));
    cx.quit();
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert_eq!(
        [&log_path, &marker, &path].map(|path| {
            (
                std::fs::read(path).unwrap(),
                std::fs::metadata(path).unwrap().modified().unwrap(),
            )
        }),
        original_files
    );
}

#[gpui::test]
fn startup_reconciles_only_the_runs_the_process_lost(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = store_at(dir.path(), None);
    let project = ProjectUuid::new();
    let scenarios = [
        (SeedEnd::Preparing, SeedRun::chat("/preparing")),
        (SeedEnd::Dispatching, SeedRun::chat("/dispatching")),
        (SeedEnd::Running, SeedRun::chat("/chat").agent("codex")),
        (
            SeedEnd::Running,
            SeedRun::cli("/cli", CliProcessState::Running { pid: 42 }).session("restart-cli"),
        ),
    ];
    let mut ids = Vec::new();
    for (end, run) in scenarios {
        let mut spec = seed::spec(project, "restart");
        spec.agent_surface = if matches!(
            run.source,
            daruda_store::tasks::TaskExecutionSource::AgentChat
        ) {
            daruda_store::tasks::TaskAgentSurface::Terminal
        } else {
            daruda_store::tasks::TaskAgentSurface::AgentChat
        };
        ids.push(spec.id.clone());
        let mut events = seed::created(spec);
        events.extend(seed::run(ids.last().unwrap(), &run, end));
        for event in events {
            smol::block_on(store.propose(event)).unwrap();
        }
    }
    cx.update(|cx| {
        RunsHost::install(RunsHost::open(Ok(store)), cx);
        let book = cx.global::<GlobalTasks>();
        assert!(matches!(
            book.get(&ids[0]).unwrap().lifecycle,
            Lifecycle::Undelivered { .. }
        ));
        assert!(matches!(
            book.get(&ids[1]).unwrap().lifecycle,
            Lifecycle::Undelivered { .. }
        ));
        assert!(matches!(
            book.get(&ids[2]).unwrap().lifecycle,
            Lifecycle::Interrupted { .. }
        ));
        assert!(matches!(
            book.get(&ids[3]).unwrap().lifecycle,
            Lifecycle::Running { .. }
        ));
    });
}

#[gpui::test]
fn import_is_once_and_never_changes_the_input_file(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let legacy = tempfile::tempdir().unwrap();
    let mut running =
        daruda_store::tasks::Task::new(ProjectUuid::new(), "old run".into(), String::new(), None);
    running.state = TaskState::Running {
        worktree_path: "/old".into(),
    };
    let running_id = running.id.clone();
    let backlog =
        daruda_store::tasks::Task::new(ProjectUuid::new(), "backlog".into(), String::new(), None);
    let mut done =
        daruda_store::tasks::Task::new(ProjectUuid::new(), "done".into(), String::new(), None);
    done.state = TaskState::Done {
        worktree_path: "/done".into(),
        end_reason: daruda_store::tasks::SessionEndReason::Stop,
    };
    let backlog_id = backlog.id.clone();
    let done_id = done.id.clone();
    let tasks = TasksState {
        tasks: vec![backlog, running, done],
        ..Default::default()
    };
    write_legacy_input(legacy.path(), &tasks).unwrap();
    let path = daruda_store::tasks::tasks_path_in(legacy.path());
    let bytes = std::fs::read(&path).unwrap();
    let store = store_at(dir.path(), Some(legacy.path()));
    cx.update(|cx| RunsHost::install(RunsHost::open(Ok(store.clone())), cx));
    let initial = store.current().unwrap().1;
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert!(matches!(
        initial.get(&running_id).unwrap().lifecycle,
        Lifecycle::Interrupted { .. }
    ));
    assert_eq!(
        initial.get(&backlog_id).unwrap().lifecycle,
        Lifecycle::Backlog
    );
    assert!(matches!(
        initial.get(&done_id).unwrap().lifecycle,
        Lifecycle::Done { .. }
    ));
    cx.update(RunsHost::seed_for_test);
    store.flush(Duration::from_secs(2)).unwrap();
    drop(store);
    let restored = store_at(dir.path(), Some(legacy.path()))
        .current()
        .unwrap()
        .1;
    assert_eq!(restored, initial);
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}

#[test]
fn fixture_runs_obey_the_machine_for_new_terminal_states() {
    for end in [SeedEnd::Interrupted, SeedEnd::Undelivered] {
        let spec = TaskSpec::new(
            ProjectUuid::new(),
            "t".into(),
            String::new(),
            None,
            chrono::Utc::now(),
        );
        let id = spec.id.clone();
        let mut events = seed::created(spec);
        events.extend(seed::run(&id, &SeedRun::chat("/wt"), end));
        let book = seed::replay(events);
        assert!(book.get(&id).unwrap().lifecycle.is_terminal());
    }
}
