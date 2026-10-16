//! Transient Git presentation fixture; no git commands or commits are run.

use crate::lane::git::{GitFileEntry, GitTracking, GitWorktreeStatus};
use crate::workspace::Workspace;
use daruda_store::project::{LaneKind, LeftDockView};
use gpui::{Context, Window};

impl Workspace {
    pub(in crate::workspace) fn seed_left_dock_git_for_shot(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.seed_left_dock_for_shot(false, true, false, window, cx);
        let active = self.active;
        if let Some(lane) = self.lane_for_mut(active) {
            lane.kind = LaneKind::Git {
                branch: Some("sidebar-sizing".into()),
                repo_root: lane.path.clone(),
                worktree_root: lane.path.clone(),
            };
        }
        let git = &mut self.lane_scoped_mut(active).git;
        git.tracking = Some(GitTracking {
            branch: Some("sidebar-sizing".into()),
            upstream: Some("origin/main".into()),
            ahead: 2,
            behind: 1,
        });
        git.worktree = Some(GitWorktreeStatus {
            staged: vec![GitFileEntry {
                path: "README.md".into(),
                x: 'M',
                y: ' ',
                original_path: None,
            }],
            unstaged: ["src/sidebar.rs", "src/settings.rs"]
                .into_iter()
                .map(|path| GitFileEntry {
                    path: path.into(),
                    x: ' ',
                    y: 'M',
                    original_path: None,
                })
                .collect(),
            ..Default::default()
        });
        self.docks.left_view = LeftDockView::GitChanges;
        self.git.commit_input.update(cx, |panel, cx| {
            panel.set_text("Adjust sidebar presentation", window, cx)
        });
        self.sync_commit_buttons(cx);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn scenario_name_is_reachable() {
        assert!(
            crate::workspace::screenshot_scenario::ScreenshotScenario::from_cli_name(
                "left-dock-git"
            )
            .is_some()
        );
    }
}
