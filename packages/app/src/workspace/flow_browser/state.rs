//! Transient browsing choices, independent of the active worktree and engine.

use daruda_flow::marker::RunStatus;
use daruda_store::project::LaneRef;

use crate::workspace::flow_paths::FlowOrigin;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::workspace) enum FlowScope {
    #[default]
    Current,
    Worktree(LaneRef),
}

impl FlowScope {
    pub fn resolve(self, active: LaneRef) -> LaneRef {
        match self {
            Self::Current => active,
            Self::Worktree(lane) => lane,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::workspace) enum FlowTab {
    #[default]
    Definitions,
    Runs,
}

impl FlowTab {
    pub fn index(self) -> usize {
        match self {
            Self::Definitions => 0,
            Self::Runs => 1,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::workspace) enum RunFilter {
    #[default]
    All,
    Running,
    Asking,
    Done,
    Failed,
    Resumable,
    Canceled,
    Unknown,
}

impl RunFilter {
    pub const ALL: [Self; 8] = [
        Self::All,
        Self::Running,
        Self::Asking,
        Self::Done,
        Self::Failed,
        Self::Resumable,
        Self::Canceled,
        Self::Unknown,
    ];

    pub fn for_status(status: RunStatus) -> Self {
        match status {
            RunStatus::Running => Self::Running,
            RunStatus::Done => Self::Done,
            RunStatus::Failed => Self::Failed,
            RunStatus::Crashed | RunStatus::Stalled => Self::Resumable,
            RunStatus::Canceled => Self::Canceled,
            RunStatus::Unknown => Self::Unknown,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::workspace) enum FlowGrouping {
    #[default]
    None,
    Origin,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::workspace) struct FlowBrowserState {
    pub scope: FlowScope,
    pub tab: FlowTab,
    pub origin: Option<FlowOrigin>,
    pub run_filter: RunFilter,
    pub grouping: FlowGrouping,
    collapsed: Vec<FlowOrigin>,
}

impl FlowBrowserState {
    pub fn is_open(&self, origin: FlowOrigin) -> bool {
        !self.collapsed.contains(&origin)
    }

    pub fn toggle_group(&mut self, origin: FlowOrigin) {
        if self.is_open(origin) {
            self.collapsed.push(origin);
        } else {
            self.collapsed.retain(|held| *held != origin);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chosen_scope_does_not_follow_the_active_worktree() {
        let a = LaneRef {
            project: 1,
            lane: 0,
        };
        let b = LaneRef {
            project: 2,
            lane: 0,
        };
        assert_eq!(FlowScope::Current.resolve(b), b);
        assert_eq!(FlowScope::Worktree(a).resolve(b), a);
    }

    #[test]
    fn resumable_and_unknown_are_not_failures() {
        for status in [RunStatus::Crashed, RunStatus::Stalled] {
            assert!(daruda_flow::resume::is_resumable(status));
            assert_eq!(RunFilter::for_status(status), RunFilter::Resumable);
        }
        assert_eq!(
            RunFilter::for_status(RunStatus::Unknown),
            RunFilter::Unknown
        );
        assert_eq!(RunFilter::for_status(RunStatus::Failed), RunFilter::Failed);
    }

    #[test]
    fn collapsing_sources_keeps_filters_and_other_groups() {
        let mut state = FlowBrowserState::default();
        state.toggle_group(FlowOrigin::Repo);
        assert!(!state.is_open(FlowOrigin::Repo));
        assert!(state.is_open(FlowOrigin::Project));
        state.toggle_group(FlowOrigin::Repo);
        assert!(state.is_open(FlowOrigin::Repo));
        assert_eq!(state.origin, None);
    }
}
