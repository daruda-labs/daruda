//! Flow state owned by [`super::Workspace`]: the runs this window started
//! and what the Flows tab browses. Where flow locks live is not here — lane
//! and project removal read it too (`Workspace::lock_root`).

use gpui::{Context, Window};

use super::Workspace;
use super::flow_browser::FlowBrowser;
use super::flow_browser::listing::FlowListing;
use super::flow_cache::LaneCache;
use super::flow_history::FlowHistory;
use super::flow_runs::FlowRuns;

pub(in crate::workspace) struct FlowContext {
    /// The flow runs this app started. See [`FlowRuns`] for why the rules
    /// about them live in a type rather than here.
    pub(in crate::workspace) runs: FlowRuns,
    /// One worktree's past runs, read from disk when the Flows tab needs
    /// them. See [`LaneCache`] for the rule both caches share.
    pub(in crate::workspace) history: LaneCache<FlowHistory>,
    /// The browsed worktree's flow files, listed from disk. Cached because the
    /// snapshot that needs it is rebuilt every frame, and a directory listing
    /// per frame is not what a panel costs.
    pub(in crate::workspace) list: LaneCache<FlowListing>,
    pub(in crate::workspace) browser: FlowBrowser,
}

impl FlowContext {
    pub(in crate::workspace) fn new(window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        Self {
            runs: FlowRuns::default(),
            history: LaneCache::default(),
            list: LaneCache::default(),
            browser: FlowBrowser::new(window, cx),
        }
    }
}
