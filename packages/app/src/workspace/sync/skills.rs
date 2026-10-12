//! Compatibility entry points; provider library owns watcher reconciliation.
use crate::workspace::Workspace;
use gpui::Context;
use std::path::PathBuf;
impl Workspace {
    pub(in crate::workspace) fn active_lane_root(&self) -> Option<PathBuf> {
        self.active_lane().map(|lane| lane.path.clone())
    }
    pub fn refresh_skills_watcher(&mut self, cx: &mut Context<Self>) {
        self.refresh_library(cx);
    }
}
