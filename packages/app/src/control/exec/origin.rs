//! Control-entry provenance converted at each domain boundary.

/// Where a control command entered the process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommandOrigin {
    Phone,
    Orchestrator,
}

impl CommandOrigin {
    pub(crate) fn is_solicited(self, cx: &gpui::App) -> bool {
        match self {
            Self::Phone => true,
            Self::Orchestrator => crate::window_registry::WindowRegistry::orchestrator(cx)
                .and_then(|(_, workspace)| workspace.upgrade())
                .is_some_and(|workspace| {
                    let workspace = workspace.read(cx);
                    workspace
                        .orchestrator_chat_pane()
                        .is_some_and(|pane| workspace.phone_turn_open(pane.pane, cx))
                }),
        }
    }

    pub(crate) fn task_started_by(self) -> daruda_store::tasks::StartedBy {
        match self {
            Self::Phone => daruda_store::tasks::StartedBy::Phone,
            Self::Orchestrator => daruda_store::tasks::StartedBy::Orchestrator,
        }
    }

    pub(crate) fn flow_started_by(self) -> daruda_flow::request::StartedBy {
        match self {
            Self::Phone => daruda_flow::request::StartedBy::Phone,
            Self::Orchestrator => daruda_flow::request::StartedBy::Orchestrator,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CommandOrigin;

    /// Two unrelated domain enums with the same variants: a swapped arm
    /// would compile, so the mapping is pinned here.
    #[test]
    fn every_origin_maps_to_the_same_starter_at_both_boundaries() {
        use daruda_flow::request::StartedBy as Flow;
        use daruda_store::tasks::StartedBy as Task;
        assert_eq!(CommandOrigin::Phone.task_started_by(), Task::Phone);
        assert_eq!(
            CommandOrigin::Orchestrator.task_started_by(),
            Task::Orchestrator
        );
        assert_eq!(CommandOrigin::Phone.flow_started_by(), Flow::Phone);
        assert_eq!(
            CommandOrigin::Orchestrator.flow_started_by(),
            Flow::Orchestrator
        );
    }
}
