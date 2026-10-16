//! Flow state owned by [`super::Workspace`]: what the Flows tab browses.
//! Running flows are not here — the process owns them
//! ([`crate::runs::RunsHost`]), so closing this window leaves them going.
//! Where flow locks live is not here either — lane and project removal read
//! it too (`Workspace::lock_root`).

use gpui::{Context, Window};

use super::Workspace;
use super::flow_browser::FlowBrowser;
use super::flow_browser::listing::FlowListing;
use super::flow_cache::LaneCache;
use super::flow_history::FlowHistory;

pub(in crate::workspace) struct FlowContext {
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
            history: LaneCache::default(),
            list: LaneCache::default(),
            browser: FlowBrowser::new(window, cx),
        }
    }
}
