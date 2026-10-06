//! Search and status projections; live evidence wins over the disk snapshot.

use std::path::Path;

use crate::workspace::{
    flow_browser::RunFilter,
    flow_history::{FlowHistory, FlowRunEntry},
    flow_paths::{FlowOrigin, FoundFlow},
    flow_request::FlowSource,
    flow_rows::FlowRunRow,
};

pub(super) const ORIGINS: [Option<FlowOrigin>; 4] = [
    None,
    Some(FlowOrigin::Repo),
    Some(FlowOrigin::Project),
    Some(FlowOrigin::Global),
];

pub(super) struct DefinitionList<'a> {
    pub visible: Vec<&'a FoundFlow>,
    pub counts: [usize; 4],
    pub total: usize,
}

impl<'a> DefinitionList<'a> {
    pub fn project(files: &'a [FoundFlow], origin: Option<FlowOrigin>, query: &str) -> Self {
        let query = query.trim().to_lowercase();
        let mut counts = [0; 4];
        let visible = files
            .iter()
            .filter(|file| {
                file.name.to_lowercase().contains(&query)
                    || file.path.to_string_lossy().to_lowercase().contains(&query)
            })
            .filter(|file| {
                for (index, wanted) in ORIGINS.into_iter().enumerate() {
                    if wanted.is_none_or(|wanted| wanted == file.origin) {
                        counts[index] += 1;
                    }
                }
                origin.is_none_or(|wanted| wanted == file.origin)
            })
            .collect();
        Self {
            visible,
            counts,
            total: files.len(),
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum RunRow<'a> {
    Live(&'a FlowRunRow),
    Past(&'a FlowRunEntry),
}

impl RunRow<'_> {
    pub fn dir(&self) -> &Path {
        match self {
            Self::Live(run) => &run.run_dir,
            Self::Past(run) => &run.dir,
        }
    }

    pub fn filter(&self) -> RunFilter {
        match self {
            Self::Live(run) if run.asking.is_some() => RunFilter::Asking,
            Self::Live(_) => RunFilter::Running,
            Self::Past(run) => RunFilter::for_status(run.status),
        }
    }

    pub fn source(&self) -> Option<&Path> {
        match self {
            Self::Live(FlowRunRow {
                source: FlowSource::File(path),
                ..
            }) => Some(path),
            _ => None,
        }
    }

    fn matches(&self, query: &str) -> bool {
        query.is_empty()
            || self
                .dir()
                .file_name()
                .is_some_and(|name| name.to_string_lossy().to_lowercase().contains(query))
            || self
                .source()
                .is_some_and(|path| path.to_string_lossy().to_lowercase().contains(query))
            || matches!(self, Self::Live(run) if run.doing.to_lowercase().contains(query))
    }
}

pub(super) struct RunList<'a> {
    pub visible: Vec<RunRow<'a>>,
    pub counts: [usize; 8],
    pub total: usize,
    pub waiting: usize,
}

impl<'a> RunList<'a> {
    pub fn project(
        live: &'a [FlowRunRow],
        history: Option<&'a FlowHistory>,
        filter: RunFilter,
        query: &str,
    ) -> Self {
        let mut rows: Vec<_> = live.iter().map(RunRow::Live).collect();
        if let Some(history) = history {
            rows.extend(
                history
                    .runs()
                    .iter()
                    .filter(|past| !live.iter().any(|run| run.run_dir == past.dir))
                    .map(RunRow::Past),
            );
        }
        rows.sort_by(|a, b| {
            let rank = |row: &RunRow<'_>| match row.filter() {
                RunFilter::Asking => 0,
                RunFilter::Running => 1,
                _ => 2,
            };
            rank(a).cmp(&rank(b)).then_with(|| b.dir().cmp(a.dir()))
        });
        let total = rows.len();
        let waiting = live
            .iter()
            .filter(|run| run.asking.is_some())
            .map(|run| 1 + run.also_waiting)
            .sum();
        let query = query.trim().to_lowercase();
        let mut counts = [0; 8];
        let visible = rows
            .into_iter()
            .filter(|row| row.matches(&query))
            .filter(|row| {
                counts[0] += 1;
                if let Some(index) = RunFilter::ALL
                    .iter()
                    .position(|filter| *filter == row.filter())
                {
                    counts[index] += 1;
                }
                filter == RunFilter::All || filter == row.filter()
            })
            .collect();
        Self {
            visible,
            counts,
            total,
            waiting,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_and_recorded_evidence_form_one_row_with_disjoint_status_counts() {
        use daruda_flow::marker::RunStatus;
        let history = FlowHistory::seeded(vec![
            FlowRunEntry {
                dir: "run-03".into(),
                started: "now".into(),
                report: None,
                status: RunStatus::Running,
            },
            FlowRunEntry {
                dir: "run-02".into(),
                started: "then".into(),
                report: None,
                status: RunStatus::Crashed,
            },
            FlowRunEntry {
                dir: "run-01".into(),
                started: "before".into(),
                report: None,
                status: RunStatus::Failed,
            },
        ]);
        let live = vec![FlowRunRow {
            lane: Default::default(),
            run_dir: "run-03".into(),
            source: FlowSource::File("ship.yaml".into()),
            lane_label: "main".into(),
            doing: "build".into(),
            asking: None,
            also_waiting: 0,
        }];
        let list = RunList::project(&live, Some(&history), RunFilter::All, "");
        assert_eq!(list.total, 3);
        assert_eq!(list.counts, [3, 1, 0, 0, 1, 1, 0, 0]);
        assert!(matches!(list.visible[0], RunRow::Live(_)));
        let filtered = RunList::project(&live, Some(&history), RunFilter::Resumable, "");
        assert_eq!(filtered.visible.len(), 1);
        assert_eq!(filtered.visible[0].dir(), Path::new("run-02"));
    }

    #[test]
    fn source_counts_follow_search_without_following_the_selected_source() {
        let files = vec![
            FoundFlow {
                path: "repo/Ship.yaml".into(),
                name: "Release review".into(),
                origin: FlowOrigin::Repo,
            },
            FoundFlow {
                path: "own/ship.yaml".into(),
                name: "Personal review".into(),
                origin: FlowOrigin::Global,
            },
            FoundFlow {
                path: "repo/test.yml".into(),
                name: "Checks".into(),
                origin: FlowOrigin::Repo,
            },
        ];
        let list = DefinitionList::project(&files, Some(FlowOrigin::Global), "SHIP");
        assert_eq!(list.total, 3);
        assert_eq!(list.visible.len(), 1);
        assert_eq!(list.counts, [2, 1, 0, 1]);
        let named = DefinitionList::project(&files, None, "release review");
        assert_eq!(named.visible.len(), 1);
        assert_eq!(
            named.visible[0].path,
            std::path::PathBuf::from("repo/Ship.yaml")
        );
    }

    #[test]
    fn live_run_search_uses_source_and_stage_without_losing_waiting_count() {
        let runs = vec![FlowRunRow {
            lane: Default::default(),
            run_dir: "run-01".into(),
            source: FlowSource::File("ship.yaml".into()),
            lane_label: "project / main".into(),
            doing: "build".into(),
            asking: Some(crate::workspace::flow_rows::AskRowData {
                ask_id: 1,
                tool: "tool".into(),
                detail: None,
                options: Vec::new(),
            }),
            also_waiting: 2,
        }];
        let filtered = RunList::project(&runs, None, RunFilter::Failed, "nothing");
        assert_eq!(filtered.waiting, 3);
        assert_eq!(filtered.total, 1);
        assert_eq!(filtered.counts, [0; 8]);
        for query in ["SHIP", "build", "run-01"] {
            let list = RunList::project(&runs, None, RunFilter::Asking, query);
            assert_eq!(list.visible.len(), 1);
            assert_eq!(list.counts, [1, 0, 1, 0, 0, 0, 0, 0]);
        }
    }
}
