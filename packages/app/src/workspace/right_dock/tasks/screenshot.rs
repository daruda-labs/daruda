//! Deterministic list fixtures; no task launches or agent sessions.

use crate::runs::seed::{SeedEnd, SeedRun};
use crate::workspace::{Workspace, pages::Page};
use daruda_store::project::ProjectUuid;
use daruda_store::tasks::{TaskEvent, TaskFilter, TaskScope, TaskSpec};
use gpui::{Context, Window};

/// One task per status the list shows, new states included.
fn fixtures(project: ProjectUuid) -> Vec<TaskEvent> {
    let path = std::path::PathBuf::from("/preview/worktree");
    [
        ("Improve project filtering", Some(SeedEnd::Running)),
        ("Add keyboard navigation to task picker", None),
        ("Simplify task creation and editing", Some(SeedEnd::Done)),
        (
            "Investigate CI authentication failure",
            Some(SeedEnd::Errored),
        ),
        ("Explore alternate sidebar layout", Some(SeedEnd::Cancelled)),
        ("Trim the startup splash", Some(SeedEnd::Interrupted)),
        ("Rename the flows page", Some(SeedEnd::Undelivered)),
        ("Check queued task delivery", Some(SeedEnd::Dispatching)),
    ]
    .into_iter()
    .enumerate()
    .flat_map(|(index, (title, end))| {
        let created = chrono::Utc::now() - chrono::Duration::minutes(30 + index as i64);
        let spec = TaskSpec::new(project, title.into(), String::new(), None, created);
        let mut events = crate::runs::seed::created(spec.clone());
        if let Some(end) = end {
            let agent = if index.is_multiple_of(2) {
                "codex"
            } else {
                "claude"
            };
            let run = SeedRun::chat(path.clone()).agent(agent);
            events.extend(crate::runs::seed::run(&spec.id, &run, end));
        }
        events
    })
    .collect()
}

impl Workspace {
    pub(in crate::workspace) fn seed_task_list_for_shot(
        &mut self,
        scope: TaskScope,
        query: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active_project().is_none() {
            self.add_project(self.data_dir.clone(), window, cx);
        }
        let Some(project) = self.active_project().map(|p| p.uuid) else {
            return;
        };
        let active = self.active;
        let root = std::env::temp_dir().join("daruda-task-list-preview");
        if let Err(error) = std::fs::create_dir_all(&root) {
            self.report_error(
                daruda_store::observability::error_report::ErrorReport::new(error.to_string())
                    .at(file!(), line!())
                    .dedup("screenshot.tasks.fixture")
                    .build(),
                cx,
            );
            return;
        }
        self.add_project(root, window, cx);
        let Some(other) = self.active_project().map(|p| p.uuid) else {
            return;
        };
        for p in &mut self.projects {
            if p.uuid == project {
                p.name = "daruda".into();
            }
            if p.uuid == other {
                p.name = "website".into();
            }
        }
        self.activate_lane(active, window, cx);
        let mut events = fixtures(project);
        events.extend(crate::runs::seed::created(TaskSpec::new(
            other,
            "Refresh the documentation site".into(),
            String::new(),
            None,
            chrono::Utc::now(),
        )));
        crate::runs::seed::commit_now(events, cx);
        self.set_task_scope(scope, cx);
        self.set_task_filter(TaskFilter::All, cx);
        self.right_views
            .tasks
            .search
            .clone()
            .update(cx, |input, cx| input.set_value(query, window, cx));
        self.open_page(Page::Tasks, window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use daruda_store::tasks::LifecycleKind;

    #[test]
    fn fixtures_cover_task_statuses_without_live_sessions() {
        let tasks = crate::runs::seed::replay(fixtures(ProjectUuid::new()));
        let kinds: Vec<_> = tasks.records().map(|task| task.lifecycle.kind()).collect();
        for kind in enum_iterator_kinds() {
            assert_eq!(kinds.iter().filter(|k| **k == kind).count(), 1, "{kind:?}");
        }
        assert!(tasks.records().all(|task| {
            task.execution
                .as_ref()
                .is_none_or(|run| run.session_id.is_none())
        }));
    }

    fn enum_iterator_kinds() -> [LifecycleKind; 8] {
        use LifecycleKind as K;
        [
            K::Backlog,
            K::Running,
            K::Done,
            K::Error,
            K::Cancelled,
            K::Interrupted,
            K::Undelivered,
            K::Dispatching,
        ]
    }
}
