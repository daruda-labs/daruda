//! Control-entry provenance converted at each domain boundary.

/// Where a control command entered the process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommandOrigin {
    Phone,
    Orchestrator,
}

impl CommandOrigin {
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
mod tests {}
