//! A backlink from a continued chat to its recorded flow attempt.

use daruda_flow::NodeId;
use daruda_store::project::LaneRef;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::workspace) struct RunOrigin {
    pub lane: LaneRef,
    pub dir: PathBuf,
    pub node: NodeId,
    pub evidence: u32,
}

#[cfg(test)]
mod tests {}
